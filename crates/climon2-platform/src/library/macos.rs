//! macOS's library checks (library embedding): whether a file the dynamic
//! loader refused has a slice dyld would load into this process, and which
//! file dyld actually loaded. dyld looks for a library's file name in every
//! `DYLD_LIBRARY_PATH` directory before it opens an absolute path, so a
//! library found there is refused rather than used for the one requested.
use std::{
    ffi::{CStr, c_int, c_void},
    fs::File,
    mem::MaybeUninit,
    os::fd::AsRawFd,
    ptr,
};

/// What a loadable client library is here.
pub(super) const ARCHITECTURE: &str = "a Mach-O library with a slice for this process";

unsafe extern "C" {
    /// `<mach-o/utils.h>` (macOS 13): zero when the open file has a slice
    /// that dyld would load into this process. A null block only asks.
    fn macho_best_slice_in_fd(fd: c_int, best_slice: *const c_void) -> c_int;
}

/// Whether `file` has a slice for this process: a library without one, or a
/// file that is no Mach-O file, does not.
pub(super) fn runs_here(file: &File) -> bool {
    // SAFETY: an open descriptor and the documented null block.
    unsafe { macho_best_slice_in_fd(file.as_raw_fd(), ptr::null()) == 0 }
}

/// The file dyld loaded for `symbol`'s image when it is not `requested`; an
/// image `dladdr` cannot name is not the requested one either.
pub(super) fn substitute(symbol: *const c_void, requested: &CStr) -> Option<String> {
    let mut info = MaybeUninit::<libc::Dl_info>::zeroed();
    // SAFETY: dladdr writes at most the supplied Dl_info; it stays zeroed when
    // dladdr fails.
    let name = unsafe {
        libc::dladdr(symbol, info.as_mut_ptr());
        info.assume_init().dli_fname
    };
    // SAFETY: a nonnull image name is a NUL-terminated path owned by dyld.
    let loaded = (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) });
    (loaded != Some(requested)).then(|| {
        loaded.map_or_else(
            || "an image dladdr cannot name".into(),
            |name| name.to_string_lossy().into_owned(),
        )
    })
}
