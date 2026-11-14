//! The host's own count of the heap the client library's calls hold (#180),
//! Linux only: an oracle independent of the library's accounting.
//!
//! The host executable defines glibc's `malloc`, `calloc`, `realloc`, `free`
//! and `posix_memalign`, exported dynamically (`build.rs`), so the dynamic
//! loader binds the loaded library, and glibc itself, to them; each forwards
//! to glibc's own allocator. While measuring, a block allocated by a thread
//! inside a library call is recorded with the bytes requested until it is
//! freed, by any thread. A resized block stays recorded, or unrecorded, as it
//! was: it stays charged to whoever first allocated it.

use serde_json::{Value, json};
use std::{
    cell::Cell,
    ffi::{c_int, c_void},
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

unsafe extern "C" {
    fn __libc_malloc(size: usize) -> *mut c_void;
    fn __libc_calloc(count: usize, size: usize) -> *mut c_void;
    fn __libc_realloc(block: *mut c_void, size: usize) -> *mut c_void;
    fn __libc_free(block: *mut c_void);
    fn __libc_memalign(alignment: usize, size: usize) -> *mut c_void;
}

/// More live blocks than one instance's calls hold.
const CAPACITY: usize = 16_384;

/// The recorded live blocks: their addresses and requested sizes.
struct Blocks {
    addresses: [usize; CAPACITY],
    sizes: [usize; CAPACITY],
    count: usize,
    live: u64,
    peak: u64,
}

impl Blocks {
    fn insert(&mut self, address: usize, size: usize) {
        assert!(self.count < CAPACITY, "the host's block table is full");
        self.addresses[self.count] = address;
        self.sizes[self.count] = size;
        self.count += 1;
        self.live += size as u64;
        self.peak = self.peak.max(self.live);
    }

    /// Forgets `address`, returning its size when it was recorded.
    fn remove(&mut self, address: usize) -> Option<usize> {
        let index = self.addresses[..self.count]
            .iter()
            .position(|recorded| *recorded == address)?;
        let size = self.sizes[index];
        self.count -= 1;
        self.addresses[index] = self.addresses[self.count];
        self.sizes[index] = self.sizes[self.count];
        self.live -= size as u64;
        Some(size)
    }
}

static MEASURING: AtomicBool = AtomicBool::new(false);
static BLOCKS: Mutex<Blocks> = Mutex::new(Blocks {
    addresses: [0; CAPACITY],
    sizes: [0; CAPACITY],
    count: 0,
    live: 0,
    peak: 0,
});

thread_local! {
    /// Whether this thread is inside a library call.
    static CALLING: Cell<bool> = const { Cell::new(false) };
}

fn blocks() -> MutexGuard<'static, Blocks> {
    BLOCKS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Makes one library call: what it allocates is the library's.
pub fn calling<T>(call: impl FnOnce() -> T) -> T {
    let outer = CALLING.replace(true);
    let result = call();
    CALLING.set(outer);
    result
}

/// Starts measuring from nothing: blocks allocated earlier are never counted.
pub fn measure() {
    let mut blocks = blocks();
    blocks.count = 0;
    blocks.live = 0;
    blocks.peak = 0;
    MEASURING.store(true, Ordering::SeqCst);
}

/// The bytes library calls hold now, and the most they held at once.
pub fn observe() -> Value {
    let blocks = blocks();
    json!({ "live": blocks.live, "peak": blocks.peak, "blocks": blocks.count })
}

/// Records a new block when a measured library call allocated it.
fn allocated(block: *mut c_void, size: usize) {
    if MEASURING.load(Ordering::SeqCst) && !block.is_null() && CALLING.get() {
        blocks().insert(block as usize, size);
    }
}

fn forget(block: *mut c_void) -> Option<usize> {
    if MEASURING.load(Ordering::SeqCst) && !block.is_null() {
        blocks().remove(block as usize)
    } else {
        None
    }
}

/// # Safety
///
/// glibc's `malloc` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    // SAFETY: forwarded unchanged.
    let block = unsafe { __libc_malloc(size) };
    allocated(block, size);
    block
}

/// # Safety
///
/// glibc's `calloc` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    // SAFETY: forwarded unchanged; a product that overflows fails there.
    let block = unsafe { __libc_calloc(count, size) };
    allocated(block, count.wrapping_mul(size));
    block
}

/// # Safety
///
/// glibc's `realloc` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(block: *mut c_void, size: usize) -> *mut c_void {
    if block.is_null() {
        // SAFETY: realloc of null is malloc.
        return unsafe { malloc(size) };
    }
    // Forgotten first, so no other thread can record the address freed here.
    let recorded = forget(block);
    // SAFETY: forwarded unchanged.
    let resized = unsafe { __libc_realloc(block, size) };
    let kept = if resized.is_null() && size != 0 {
        // Failed: the old block is unchanged.
        recorded.map(|old| (block, old))
    } else {
        recorded.map(|_| (resized, size))
    };
    if let Some((address, size)) = kept.filter(|(address, _)| !address.is_null()) {
        blocks().insert(address as usize, size);
    }
    resized
}

/// # Safety
///
/// glibc's `free` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn free(block: *mut c_void) {
    forget(block);
    // SAFETY: forwarded unchanged, after the block is forgotten.
    unsafe { __libc_free(block) }
}

/// # Safety
///
/// glibc's `posix_memalign` contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_memalign(
    out: *mut *mut c_void,
    alignment: usize,
    size: usize,
) -> c_int {
    if !alignment.is_power_of_two() || !alignment.is_multiple_of(size_of::<*mut c_void>()) {
        return libc::EINVAL;
    }
    // SAFETY: a valid alignment, forwarded.
    let block = unsafe { __libc_memalign(alignment, size) };
    if block.is_null() {
        return libc::ENOMEM;
    }
    allocated(block, size);
    // SAFETY: the caller supplies writable storage for one pointer.
    unsafe { out.write(block) };
    0
}
