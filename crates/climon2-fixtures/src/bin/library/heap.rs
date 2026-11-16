//! The host's own count of the heap the client library holds (#180), on
//! x86-64 GNU/Linux only: an oracle independent of the library's accounting.
//!
//! The host executable defines glibc's `malloc`, `calloc`, `realloc`, `free`
//! and `posix_memalign`, exported dynamically (`build.rs`), so the dynamic
//! loader binds the loaded library, and glibc itself, to them; each forwards
//! to glibc's own allocator. Each allocating entry is a naked function that
//! passes on its return address, so the count knows which code asked. While
//! measuring, a block the library image's own code allocated is recorded
//! with the bytes requested until it is freed, by anyone. glibc's and the
//! loader's internal blocks, such as thread-local destructor lists, are not
//! the library's and are never recorded. A resized block stays recorded, or
//! unrecorded, as it was.

use serde_json::{Value, json};
use std::{
    ffi::{c_int, c_void},
    ops::Range,
    sync::{
        Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

unsafe extern "C" {
    fn __libc_malloc(size: usize) -> *mut c_void;
    fn __libc_calloc(count: usize, size: usize) -> *mut c_void;
    fn __libc_realloc(block: *mut c_void, size: usize) -> *mut c_void;
    fn __libc_free(block: *mut c_void);
    fn __libc_memalign(alignment: usize, size: usize) -> *mut c_void;
}

/// More live blocks than one instance holds.
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
        if self.count == CAPACITY {
            // Never panic here: a panic allocates.
            std::process::abort();
        }
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
/// The library image's mapped addresses.
static IMAGE_START: AtomicUsize = AtomicUsize::new(0);
static IMAGE_END: AtomicUsize = AtomicUsize::new(0);
static BLOCKS: Mutex<Blocks> = Mutex::new(Blocks {
    addresses: [0; CAPACITY],
    sizes: [0; CAPACITY],
    count: 0,
    live: 0,
    peak: 0,
});

fn blocks() -> MutexGuard<'static, Blocks> {
    BLOCKS.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What [`image`] looks for and finds.
struct Search {
    address: usize,
    found: Range<usize>,
}

/// One loaded image: its loadable segments' span, kept when it contains the
/// searched address.
unsafe extern "C" fn visit(
    info: *mut libc::dl_phdr_info,
    _: libc::size_t,
    data: *mut c_void,
) -> c_int {
    // SAFETY: the loader passes a valid entry; `data` is the search.
    let (info, search) = unsafe { (&*info, &mut *data.cast::<Search>()) };
    // SAFETY: the entry's headers cover `dlpi_phnum` entries.
    let headers = unsafe { std::slice::from_raw_parts(info.dlpi_phdr, info.dlpi_phnum.into()) };
    let base = info.dlpi_addr as usize;
    let segments = headers
        .iter()
        .filter(|header| header.p_type == libc::PT_LOAD)
        .map(|header| {
            let start = base + header.p_vaddr as usize;
            start..start + header.p_memsz as usize
        });
    let start = segments.clone().map(|segment| segment.start).min();
    let end = segments.map(|segment| segment.end).max();
    match (start, end) {
        (Some(start), Some(end)) if (start..end).contains(&search.address) => {
            search.found = start..end;
            1
        }
        _ => 0,
    }
}

/// The addresses of the loaded image that contains `address`.
fn image(address: usize) -> Range<usize> {
    let mut search = Search {
        address,
        found: 0..0,
    };
    // SAFETY: the callback only reads the loader's entries and writes the
    // search, which outlives the call.
    unsafe { libc::dl_iterate_phdr(Some(visit), (&raw mut search).cast()) };
    assert!(!search.found.is_empty(), "the library image is loaded");
    search.found
}

/// Starts measuring, from nothing, the image that contains `library`:
/// blocks allocated earlier are never counted.
pub fn measure(library: *const c_void) {
    let image = image(library as usize);
    IMAGE_START.store(image.start, Ordering::SeqCst);
    IMAGE_END.store(image.end, Ordering::SeqCst);
    let mut blocks = blocks();
    blocks.count = 0;
    blocks.live = 0;
    blocks.peak = 0;
    MEASURING.store(true, Ordering::SeqCst);
}

/// The bytes the library holds now, and the most it held at once.
pub fn observe() -> Value {
    let blocks = blocks();
    json!({ "live": blocks.live, "peak": blocks.peak, "blocks": blocks.count })
}

/// Whether code at `caller` is the library's, while measuring.
fn library(caller: usize) -> bool {
    MEASURING.load(Ordering::SeqCst)
        && (IMAGE_START.load(Ordering::SeqCst)..IMAGE_END.load(Ordering::SeqCst)).contains(&caller)
}

/// Records a new block when the library's code allocated it.
fn allocated(block: *mut c_void, size: usize, caller: usize) {
    if !block.is_null() && library(caller) {
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
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn malloc(size: usize) -> *mut c_void {
    // The return address, at the stack top on entry, becomes the next
    // argument; the jump keeps the caller's frame, so the callee returns to it.
    std::arch::naked_asm!("mov rsi, [rsp]", "jmp {}", sym malloc_from)
}

extern "C" fn malloc_from(size: usize, caller: usize) -> *mut c_void {
    // SAFETY: forwarded unchanged.
    let block = unsafe { __libc_malloc(size) };
    allocated(block, size, caller);
    block
}

/// # Safety
///
/// glibc's `calloc` contract.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn calloc(count: usize, size: usize) -> *mut c_void {
    std::arch::naked_asm!("mov rdx, [rsp]", "jmp {}", sym calloc_from)
}

extern "C" fn calloc_from(count: usize, size: usize, caller: usize) -> *mut c_void {
    // SAFETY: forwarded unchanged; a product that overflows fails there.
    let block = unsafe { __libc_calloc(count, size) };
    allocated(block, count.wrapping_mul(size), caller);
    block
}

/// # Safety
///
/// glibc's `realloc` contract.
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn realloc(block: *mut c_void, size: usize) -> *mut c_void {
    std::arch::naked_asm!("mov rdx, [rsp]", "jmp {}", sym realloc_from)
}

extern "C" fn realloc_from(block: *mut c_void, size: usize, caller: usize) -> *mut c_void {
    if block.is_null() {
        return malloc_from(size, caller);
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
#[unsafe(naked)]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn posix_memalign(
    out: *mut *mut c_void,
    alignment: usize,
    size: usize,
) -> c_int {
    std::arch::naked_asm!("mov rcx, [rsp]", "jmp {}", sym posix_memalign_from)
}

extern "C" fn posix_memalign_from(
    out: *mut *mut c_void,
    alignment: usize,
    size: usize,
    caller: usize,
) -> c_int {
    if !alignment.is_power_of_two() || !alignment.is_multiple_of(size_of::<*mut c_void>()) {
        return libc::EINVAL;
    }
    // SAFETY: a valid alignment, forwarded.
    let block = unsafe { __libc_memalign(alignment, size) };
    if block.is_null() {
        return libc::ENOMEM;
    }
    allocated(block, size, caller);
    // SAFETY: the caller supplies writable storage for one pointer.
    unsafe { out.write(block) };
    0
}
