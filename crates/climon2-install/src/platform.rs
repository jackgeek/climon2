#[cfg(not(target_os = "macos"))]
compile_error!("climon2 installer currently supports macOS only");

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

unsafe extern "C" {
    fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    fn renamex_np(
        from: *const std::ffi::c_char,
        to: *const std::ffi::c_char,
        flags: u32,
    ) -> std::ffi::c_int;
}

pub(crate) fn default_directory() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "HOME is required without --dir",
        )
    })?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "HOME must be absolute",
        ));
    }
    Ok(home.join(".local/share/climon2"))
}

pub(crate) fn swap(source: &Path, destination: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())?;
    let destination = CString::new(destination.as_os_str().as_bytes())?;
    // Darwin RENAME_SWAP exchanges whole directories without a missing-version window.
    if unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 0x00000002) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn lock(home: &Path) -> io::Result<File> {
    let file = File::open(home)?;
    // LOCK_EX | LOCK_NB: one writer owns both publication and crash recovery.
    if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}
