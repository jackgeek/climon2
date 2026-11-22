//! Windows's client-library loading (#318; library embedding, "Windows").
//! The library opens by exact path with `LoadLibraryExW` and the search flags
//! the daemon uses for `conpty.dll`, so its own imports resolve only beside
//! it and in System32. It is never freed. DLL redirection and `.local` files
//! can still load another file for that path, so the loaded module's path
//! must be the one requested. Failures are classified from the loader's own
//! results, never from message text.
use super::{Failure, Refusal};
use climon2_core::native::{self, Bootstrap};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::Path,
};
use windows_sys::Win32::{
    Foundation::{ERROR_BAD_EXE_FORMAT, HMODULE},
    System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryExW},
};

/// Beside the library, then System32: never the working directory or `PATH`.
/// `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR (0x100) | LOAD_LIBRARY_SEARCH_SYSTEM32 (0x800)`,
/// spelled as its value: the bits are disjoint, so `^` for `|` would be an
/// equivalent mutant.
const SEARCH: u32 = 0x0900;

/// The PE machine of every climon2 Windows artifact.
const X86_64: u16 = 0x8664;

/// The most code units a module path can hold.
const LONGEST: usize = 32_768;

/// The bootstrap of the library at `path`, loaded by exact path and never
/// freed.
pub(super) fn bootstrap(path: &Path) -> Result<Bootstrap, Refusal> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated path and no file handle; the module is
    // deliberately never freed.
    let module = unsafe { LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), SEARCH) };
    if module.is_null() {
        return Err(unloadable(path, &io::Error::last_os_error()));
    }
    if let Some(loaded) = loaded_instead(module, path) {
        return Err((
            Failure::SubstitutedImage,
            format!("Windows loaded {loaded} instead, through DLL redirection"),
        ));
    }
    // SAFETY: a loaded module and a NUL-terminated symbol name.
    let Some(symbol) = (unsafe { GetProcAddress(module, native::BOOTSTRAP.as_ptr().cast()) })
    else {
        return Err((
            Failure::MissingSymbol,
            format!("it does not export {}", native::BOOTSTRAP.to_string_lossy()),
        ));
    };
    // SAFETY: the frozen bootstrap symbol has the `Bootstrap` signature.
    Ok(unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, Bootstrap>(symbol) })
}

/// Why Windows could not load `path`, decided from its own result and the
/// file it was given; the system message is only quoted. A file that cannot
/// be opened afterwards is a missing library; a file Windows calls no image,
/// or whose PE machine is not x86-64, is another architecture; anything else
/// is a library this process could load but for something it needs.
fn unloadable(path: &Path, error: &io::Error) -> Refusal {
    let bad_format = error.raw_os_error() == Some(ERROR_BAD_EXE_FORMAT as i32);
    match File::open(path) {
        Err(open) => (
            Failure::MissingLibrary,
            format!("it cannot be opened: {open}"),
        ),
        Ok(file) if !bad_format && machine(&file) == Some(X86_64) => (
            Failure::MissingDependency,
            format!("Windows cannot load it or a library it needs: {error}"),
        ),
        Ok(_) => (
            Failure::IncompatibleArchitecture,
            format!("it is not an x86-64 DLL; Windows said: {error}"),
        ),
    }
}

/// The machine a PE file's header names: the DOS header's `e_lfanew` at
/// 0x3C locates the `PE\0\0` signature, which the machine follows. A header
/// that cannot be read names none.
fn machine(file: &File) -> Option<u16> {
    let mut header = Vec::new();
    file.take(4096).read_to_end(&mut header).ok()?;
    let offset = u32::from_le_bytes(header.get(0x3C..0x40)?.try_into().ok()?) as usize;
    let signature = header.get(offset..offset.checked_add(6)?)?;
    let dos = header.starts_with(b"MZ") && signature.starts_with(b"PE\0\0");
    dos.then(|| u16::from_le_bytes([signature[4], signature[5]]))
}

/// The file Windows loaded as `module` when it is not `requested`, or `None`
/// when they are the same path. A module whose path cannot be read is not
/// the one requested.
fn loaded_instead(module: HMODULE, requested: &Path) -> Option<String> {
    let mut buffer = vec![0u16; LONGEST];
    // SAFETY: a loaded module and a buffer of `LONGEST` code units.
    let length = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), LONGEST as u32) };
    buffer.truncate(length as usize);
    let loaded = OsString::from_wide(&buffer);
    (loaded != requested.as_os_str()).then(|| format!("{loaded:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    /// kernel32.dll, which every process has loaded.
    fn kernel32() -> HMODULE {
        let name: Vec<u16> = "kernel32.dll".encode_utf16().chain([0]).collect();
        // SAFETY: a NUL-terminated module name.
        let module = unsafe { GetModuleHandleW(name.as_ptr()) };
        assert!(!module.is_null());
        module
    }

    /// A module loaded from another file than the one requested is refused
    /// by name. A test cannot make DLL redirection substitute a file, so it
    /// asks about a module it knows is another file.
    #[test]
    fn a_module_from_another_file_than_the_one_requested_is_named() {
        let requested = Path::new(r"C:\climon2\0.1.0\climon2_client.dll");
        let loaded = loaded_instead(kernel32(), requested).unwrap();
        assert!(
            loaded.to_ascii_lowercase().ends_with(r#"\\kernel32.dll""#),
            "{loaded}"
        );
    }

    /// The module's own path is the one requested.
    #[test]
    fn a_module_from_the_requested_file_is_accepted() {
        let mut buffer = vec![0u16; LONGEST];
        // SAFETY: a loaded module and a buffer of `LONGEST` code units.
        let length = unsafe { GetModuleFileNameW(kernel32(), buffer.as_mut_ptr(), LONGEST as u32) };
        let own = OsString::from_wide(&buffer[..length as usize]);
        assert_eq!(loaded_instead(kernel32(), Path::new(&own)), None);
    }
}
