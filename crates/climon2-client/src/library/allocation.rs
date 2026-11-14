//! Instance allocation, measured at the allocator (#180): every block the
//! library allocates while working for an instance is charged to that
//! instance's ledger until it is freed, on whatever thread, so `snapshot`
//! counts everything the instance holds, its diagnostics dispatcher and any
//! dependency's internal allocations included.
//!
//! This image's global allocator prefixes every block with the ledger it is
//! charged to, or with none. A block is charged with everything requested
//! from the system allocator for it, prefix included. A block resized while
//! working for its own instance stays charged to it; resized under another
//! instance's charge, or none, it moves off its ledger to the shared-release
//! allocation, which is not tracked yet. Process- and thread-wide state a
//! dependency creates is created uncharged first (`warm`).

use super::instance::Gauge;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ptr,
    sync::{
        Arc, OnceLock,
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
#[derive(Default)]
struct Ledger {
    current: AtomicU64,
    peak: AtomicU64,
}

impl Ledger {
    const fn holding(bytes: u64) -> Self {
        Self {
            current: AtomicU64::new(bytes),
            peak: AtomicU64::new(bytes),
        }
    }

    /// A charged size changed from `old` to `new` bytes, in one step, so the
    /// peak never counts a resized block twice. Wrapping arithmetic adds the
    /// difference either way.
    fn change(&self, old: usize, new: usize) {
        let change = (new as u64).wrapping_sub(old as u64);
        let current = self
            .current
            .fetch_add(change, Ordering::Relaxed)
            .wrapping_add(change);
        self.peak.fetch_max(current, Ordering::Relaxed);
    }

    fn charge(&self, bytes: usize) {
        self.change(0, bytes);
    }

    fn discharge(&self, bytes: usize) {
        self.change(bytes, 0);
    }
}

/// The prefix of a block with `layout`.
fn prefix(layout: Layout) -> usize {
    layout.align().max(PREFIX)
}

/// The bytes one ledger's own block takes, which it holds itself: measured
/// once, outside any instance, by charging one ledger's allocation to a
/// probe ledger.
fn ledger_bytes() -> u64 {
    static BYTES: OnceLock<u64> = OnceLock::new();
    *BYTES.get_or_init(|| {
        let probe = Account(Arc::default());
        let measured = probe.charged(Arc::<Ledger>::default);
        let bytes = probe.0.current.load(Ordering::Relaxed);
        drop(measured);
        bytes
    })
}

/// Measures what this allocator needs to know before any instance exists.
pub(super) fn prepare() {
    ledger_bytes();
}

/// One instance's share of its ledger.
#[derive(Clone)]
pub(super) struct Account(Arc<Ledger>);

impl Account {
    /// A new ledger, already charged its own block.
    pub(super) fn open() -> Self {
        Self(Arc::new(Ledger::holding(ledger_bytes())))
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
    pub(super) fn measure(&self) -> Gauge<u64> {
        Gauge {
            current: self.0.current.load(Ordering::Relaxed),
            peak: self.0.peak.load(Ordering::Relaxed),
        }
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
    unsafe fn allocate(layout: Layout, system: unsafe fn(&System, Layout) -> *mut u8) -> *mut u8 {
        let prefix = prefix(layout);
        let Some(outer) = layout
            .size()
            .checked_add(prefix)
            .and_then(|size| Layout::from_size_align(size, prefix).ok())
        else {
            return ptr::null_mut();
        };
        // SAFETY: `outer` includes the prefix, so its size is nonzero, as
        // the system allocator requires; it is a valid layout.
        let base = unsafe { system(&System, outer) };
        if base.is_null() {
            return base;
        }
        let ledger = CHARGED.get();
        // SAFETY: a ledger set as charged is live while it is set.
        if let Some(charged) = unsafe { ledger.as_ref() } {
            // SAFETY: as above; the block now shares the ledger.
            unsafe { Arc::increment_strong_count(ledger) };
            charged.charge(outer.size());
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
    /// `block` was allocated by this allocator with `layout`.
    unsafe fn prefixed(block: *mut u8, layout: Layout) -> (*mut u8, Layout, *const Ledger) {
        let prefix = prefix(layout);
        // SAFETY: this layout was valid when the block was allocated.
        let outer = unsafe { Layout::from_size_align_unchecked(layout.size() + prefix, prefix) };
        // SAFETY: the prefix precedes the block.
        let base = unsafe { block.sub(prefix) };
        // SAFETY: the prefix holds the block's ledger.
        let ledger = unsafe { base.cast::<*const Ledger>().read() };
        (base, outer, ledger)
    }

    /// A charged block was resized from `old` to `new` bytes: it stays
    /// charged when resized while working for its own instance, and moves off
    /// its ledger otherwise.
    ///
    /// # Safety
    ///
    /// `base` is the resized block, whose prefix holds `ledger`, live or null.
    unsafe fn resized(base: *mut u8, ledger: *const Ledger, old: usize, new: usize) {
        if ledger == CHARGED.get() {
            // SAFETY: the block's share keeps its ledger live.
            if let Some(charged) = unsafe { ledger.as_ref() } {
                charged.change(old, new);
            }
        } else if !ledger.is_null() {
            // Resized for someone else: shared-release allocation now.
            // SAFETY: the prefix moved with the block.
            unsafe { base.cast::<*const Ledger>().write(ptr::null()) };
            // SAFETY: the block's ledger and its charge before the resize.
            unsafe { Self::release(ledger, old) };
        }
    }

    /// A block leaves `ledger`, whose share it held.
    ///
    /// # Safety
    ///
    /// `ledger` is the block's live ledger, and `bytes` its charge.
    unsafe fn release(ledger: *const Ledger, bytes: usize) {
        // SAFETY: the block's share keeps its ledger live.
        unsafe { &*ledger }.discharge(bytes);
        // SAFETY: the block's share ends here.
        unsafe { Arc::decrement_strong_count(ledger) };
    }
}

// SAFETY: every block is the system allocator's, offset by a prefix that keeps
// the caller's alignment; the caller's layout always yields the same prefix.
unsafe impl GlobalAlloc for Accounting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract.
        unsafe { Self::allocate(layout, System::alloc) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller's contract.
        unsafe { Self::allocate(layout, System::alloc_zeroed) }
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        // SAFETY: the caller's contract.
        let (base, outer, ledger) = unsafe { Self::prefixed(block, layout) };
        // SAFETY: as above.
        unsafe { System.dealloc(base, outer) };
        if !ledger.is_null() {
            // SAFETY: the freed block's ledger and charge.
            unsafe { Self::release(ledger, outer.size()) };
        }
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's contract.
        let (base, outer, ledger) = unsafe { Self::prefixed(block, layout) };
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
        // SAFETY: the resized block, its ledger and both charges.
        unsafe { Self::resized(resized, ledger, outer.size(), size.size()) };
        // SAFETY: within the resized block.
        unsafe { resized.add(prefix) }
    }
}

#[cfg(test)]
mod tests {
    use super::{Account, PREFIX};
    use std::alloc::{Layout, alloc, dealloc, realloc};

    const BLOCK: usize = 64;
    const GROWN: usize = 96;

    /// An open ledger already holds exactly its own block.
    #[test]
    fn a_new_ledger_holds_its_own_block() {
        let account = Account::open();
        let gauge = account.measure();
        assert!(gauge.current > PREFIX as u64);
        assert_eq!(gauge.peak, gauge.current);
    }

    /// A block is charged with its prefix; resized for its own instance it
    /// stays charged in one step; resized for another, or for none, it moves
    /// off its ledger, and its later release changes no ledger.
    #[test]
    fn a_block_stays_charged_only_while_resized_for_its_own_instance() {
        let layout = Layout::from_size_align(BLOCK, 8).unwrap();
        for other in [Some(Account::open()), None] {
            let (owner, opened) = (Account::open(), Account::open().measure());
            // SAFETY (each call): a nonzero layout, and every block is
            // resized and freed with the layout it has then.
            let block = owner.charged(|| unsafe { alloc(layout) });
            let charged = owner.measure();
            assert_eq!(charged.current, opened.current + (BLOCK + PREFIX) as u64);
            let block = owner.charged(|| unsafe { realloc(block, layout, GROWN) });
            let grown = owner.measure();
            assert_eq!(grown.current, charged.current + (GROWN - BLOCK) as u64);
            assert_eq!(grown.peak, grown.current, "one step");
            let layout = Layout::from_size_align(GROWN, 8).unwrap();
            let resize = || unsafe { realloc(block, layout, BLOCK) };
            let block = match &other {
                Some(other) => other.charged(resize),
                None => resize(),
            };
            assert_eq!(owner.measure().current, opened.current, "moved off");
            let unchanged = other.as_ref().map(Account::measure);
            let layout = Layout::from_size_align(BLOCK, 8).unwrap();
            owner.charged(|| unsafe { dealloc(block, layout) });
            assert_eq!(owner.measure().current, opened.current);
            assert_eq!(owner.measure().peak, grown.peak);
            if let (Some(other), Some(before)) = (&other, unchanged) {
                assert_eq!(other.measure().current, before.current);
                assert_eq!(other.measure().current, opened.current);
            }
        }
    }
}
