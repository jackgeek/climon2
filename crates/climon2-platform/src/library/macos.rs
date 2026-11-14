//! macOS's library checks (library embedding): whether a file the dynamic
//! loader refused has a slice dyld would load into this process, and which
//! file dyld actually loaded. dyld looks for a library's file name in every
//! `DYLD_LIBRARY_PATH` directory before it opens an absolute path, so a
//! library found there is refused rather than used for the one requested.
use std::{
    ffi::{CStr, CString, OsStr, c_char, c_int, c_void},
    fs::File,
    mem::MaybeUninit,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::Path,
    ptr,
};

/// What a loadable client library is here.
pub(super) const ARCHITECTURE: &str = "a Mach-O library with a slice for this process";

unsafe extern "C" {
    /// `<mach-o/utils.h>` (macOS 13): zero when the open file has a slice
    /// that dyld would load into this process. A null block only asks.
    fn macho_best_slice_in_fd(fd: c_int, best_slice: *const c_void) -> c_int;
    /// `<mach-o/dyld.h>`: how many images dyld has loaded.
    fn _dyld_image_count() -> u32;
    /// `<mach-o/dyld.h>`: the path loaded image `index` was loaded from, or
    /// null once it is gone.
    fn _dyld_get_image_name(index: u32) -> *const c_char;
}

/// Whether `file` has a slice for this process: a library without one, or a
/// file that is no Mach-O file, does not.
pub(super) fn runs_here(file: &File) -> bool {
    // SAFETY: an open descriptor and the documented null block.
    unsafe { macho_best_slice_in_fd(file.as_raw_fd(), ptr::null()) == 0 }
}

/// The file dyld loaded for `requested` when it is another one. `dladdr`
/// names the image of the bootstrap `symbol`; without that symbol, the
/// requested file is loaded only if dyld lists it among its images.
pub(super) fn loaded_instead(symbol: *const c_void, requested: &CStr) -> Option<String> {
    let loaded = if symbol.is_null() {
        images()
            .into_iter()
            .find(|image| image.as_c_str() == requested)
    } else {
        image_of(symbol)
    };
    (loaded.as_deref() != Some(requested)).then(|| match loaded {
        Some(image) => image.to_string_lossy().into_owned(),
        None => named_like(requested),
    })
}

/// The image `dladdr` names for `symbol`, if it can.
fn image_of(symbol: *const c_void) -> Option<CString> {
    let mut info = MaybeUninit::<libc::Dl_info>::zeroed();
    // SAFETY: dladdr writes at most the supplied Dl_info; it stays zeroed when
    // dladdr fails.
    let name = unsafe {
        libc::dladdr(symbol, info.as_mut_ptr());
        info.assume_init().dli_fname
    };
    // SAFETY: a nonnull image name is a NUL-terminated path owned by dyld.
    (!name.is_null()).then(|| unsafe { CStr::from_ptr(name) }.to_owned())
}

/// Every image dyld has loaded, by the path it loaded it from.
fn images() -> Vec<CString> {
    // SAFETY: dyld's image list, read by index.
    (0..unsafe { _dyld_image_count() })
        .map(|index| unsafe { _dyld_get_image_name(index) })
        .filter(|name| !name.is_null())
        // SAFETY: a nonnull image name is NUL-terminated.
        .map(|name| unsafe { CStr::from_ptr(name) }.to_owned())
        .collect()
}

/// The loaded images with `requested`'s file name: one of them is what dyld
/// loaded for it.
fn named_like(requested: &CStr) -> String {
    let file = |path: &CStr| {
        Path::new(OsStr::from_bytes(path.to_bytes()))
            .file_name()
            .map(OsStr::to_owned)
    };
    let name = file(requested);
    let images: Vec<_> = images()
        .into_iter()
        .filter(|image| file(image) == name)
        .map(|image| image.to_string_lossy().into_owned())
        .collect();
    format!("a file of the same name ({})", images.join(", "))
}
