//! Instance allocation, measured at the allocator (#180): every block the
//! library allocates while working for an instance is charged to that
//! instance's ledger until it is freed, on whatever thread, so `snapshot`
//! counts everything the instance holds, its diagnostics dispatcher and any
//! dependency's internal allocations included.
//!
//! This image's global allocator prefixes every block with the ledger it is
//! charged to, or with none. A block is charged with everything requested
//! from the system allocator for it, prefix included. A resized block stays
//! charged to its ledger. What a dependency keeps for the whole process or
//! for one thread, created lazily by an instance's call, is charged to that
//! instance.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ptr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

/// Each block's prefix holds its ledger; at least this long, and as long as
/// the block's alignment, so the block keeps its alignment.
const PREFIX: usize = 16;

#[global_allocator]
static ALLOCATOR: Accounting = Accounting;

thread_local! {
    /// The ledger this thread's allocations are charged to now, if any.
    static CHARGED: Cell<*const Ledger> = const { Cell::new(ptr::null()) };
}

/// What one instance holds, and the most it has held. Its instance and every
/// block charged to it share it, so it lives while any of them does.
struct Ledger {
    current: AtomicU64,
    peak: AtomicU64,
}

impl Ledger {
    /// A charged size changed from `old` to `new` bytes, in one step, so the
    /// peak never counts a resized block twice. Wrapping arithmetic adds the
    /// difference either way.
    fn resize(&self, old: usize, new: usize) {
        let change = (new as u64).wrapping_sub(old as u64);
        let current = self
            .current
            .fetch_add(change, Ordering::Relaxed)
            .wrapping_add(change);
        self.peak.fetch_max(current, Ordering::Relaxed);
    }
}

/// The prefix of a block with `layout`.
fn prefix(layout: Layout) -> usize {
    layout.align().max(PREFIX)
}

/// The bytes a ledger's own block is charged, which it holds itself: the
/// `Arc`'s two counts, then the ledger, behind its prefix.
const LEDGER_BYTES: usize = 2 * size_of::<usize>() + size_of::<Ledger>() + PREFIX;

/// One instance's share of its ledger.
#[derive(Clone)]
pub(super) struct Account(Arc<Ledger>);

impl Account {
    /// A new ledger, already charged its own block.
    pub(super) fn open() -> Self {
        Self(Arc::new(Ledger {
            current: AtomicU64::new(LEDGER_BYTES as u64),
            peak: AtomicU64::new(LEDGER_BYTES as u64),
        }))
    }

    /// Runs `work` with every allocation it makes on this thread charged
    /// here, even if it unwinds.
    pub(super) fn charged<T>(&self, work: impl FnOnce() -> T) -> T {
        struct Restore(*const Ledger);

        impl Drop for Restore {
            fn drop(&mut self) {
                CHARGED.set(self.0);
            }
        }

        let _restore = Restore(CHARGED.replace(Arc::as_ptr(&self.0)));
        work()
    }

    /// The bytes held now, and the most held at once.
    pub(super) fn measure(&self) -> (u64, u64) {
        (
            self.0.current.load(Ordering::Relaxed),
            self.0.peak.load(Ordering::Relaxed),
        )
    }
}

/// This image's global allocator: the system allocator, with every block
/// prefixed by the ledger it is charged to.
struct Accounting;

impl Accounting {
    /// Allocates through `system` and charges the current ledger.
    ///
    /// # Safety
    ///
    /// `GlobalAlloc::alloc`'s contract for `layout`.
    unsafe fn place(layout: Layout, system: unsafe fn(&System, Layout) -> *mut u8) -> *mut u8 {
        let prefix = prefix(layout);
        let Some(outer) = layout
            .size()
            .checked_add(prefix)
            .and_then(|size| Layout::from_size_align(size, prefix).ok())
        else {
            return ptr::null_mut();
        };
        // SAFETY: `outer` is nonzero in size.
        let base = unsafe { system(&System, outer) };
        if base.is_null() {
            return base;
        }
        let ledger = CHARGED.get();
        // SAFETY: a ledger set as charged is live while it is set.
        if let Some(charged) = unsafe { ledger.as_ref() } {
            // SAFETY: as above; the block now shares the ledger.
            unsafe { Arc::increment_strong_count(ledger) };
            charged.resize(0, outer.size());
        }
        // SAFETY: the prefix is at least a pointer long and aligned for one.
        unsafe { base.cast::<*const Ledger>().write(ledger) };
        // SAFETY: within the block.
        unsafe { base.add(prefix) }
    }

    /// The system block behind `block`, its layout and its ledger.
    ///
    /// # Safety
    ///
    /// `block` was placed by this allocator with `layout`.
    unsafe fn base(block: *mut u8, layout: Layout) -> (*mut u8, Layout, *const Ledger) {
        let prefix = prefix(layout);
        // SAFETY: this layout was valid when the block was placed.
        let outer = unsafe { Layout::from_size_align_unchecked(layout.size() + prefix, prefix) };
        // SAFETY: the prefix precedes the block.
        let base = unsafe { block.sub(prefix) };
        // SAFETY: placed holds the block's ledger.
        let ledger = unsafe { base.cast::<*const Ledger>().read() };
        (base, outer, ledger)
    }
}

// SAFETY: every block is the system allocator's, offset by a prefix that keeps
// the caller's alignment; the caller's layout always yields the same prefix.
unsafe impl GlobalAlloc for Accounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract.
        unsafe { Self::place(layout, System::alloc) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract.
        unsafe { Self::place(layout, System::alloc_zeroed) }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract.
        let (base, outer, ledger) = unsafe { Self::base(block, layout) };
        // SAFETY: as above.
        unsafe { System.dealloc(base, outer) };
        // SAFETY: the block's share keeps its ledger live.
        if let Some(charged) = unsafe { ledger.as_ref() } {
            charged.resize(outer.size(), 0);
            // SAFETY: the block's share ends here.
            unsafe { Arc::decrement_strong_count(ledger) };
        }
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract.
        let (base, outer, ledger) = unsafe { Self::base(block, layout) };
        let prefix = prefix(layout);
        // The caller's `new_size` is at most `isize::MAX`, so this cannot wrap.
        let Ok(size) = Layout::from_size_align(new_size + prefix, prefix) else {
            return ptr::null_mut();
        };
        // SAFETY: the caller's contract; the prefix moves with the block.
        let resized = unsafe { System.realloc(base, outer, size.size()) };
        if resized.is_null() {
            return resized;
        }
        // SAFETY: the block's share keeps its ledger live.
        if let Some(ledger) = unsafe { ledger.as_ref() } {
            ledger.resize(outer.size(), size.size());
        }
        // SAFETY: within the resized block.
        unsafe { resized.add(prefix) }
    }
}
