#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("climon2 installer supports macOS and GNU/Linux");

use std::ffi::CString;
use std::fs::File;
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::path::PathBuf;

unsafe extern "C" {
    fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    #[cfg(target_os = "macos")]
    fn renamex_np(
        from: *const std::ffi::c_char,
        to: *const std::ffi::c_char,
        flags: u32,
    ) -> std::ffi::c_int;
    #[cfg(target_os = "linux")]
    fn renameat2(
        olddirfd: std::ffi::c_int,
        oldpath: *const std::ffi::c_char,
        newdirfd: std::ffi::c_int,
        newpath: *const std::ffi::c_char,
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
    // Both kernels exchange directories without a missing-version window.
    #[cfg(target_os = "macos")]
    let result = unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 0x00000002) };
    #[cfg(target_os = "linux")]
    let result = unsafe { renameat2(-100, source.as_ptr(), -100, destination.as_ptr(), 2) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Exclusive installer ownership of one executable home.
pub(crate) struct Lock(File);

impl Drop for Lock {
    fn drop(&mut self) {
        // flock belongs to the open file description, which a concurrently forked
        // child may still share before exec; closing alone would leave it held.
        unsafe { flock(self.0.as_raw_fd(), 8) }; // LOCK_UN
    }
}

pub(crate) fn lock(home: &Path) -> io::Result<Lock> {
    let file = File::open(home)?;
    // LOCK_EX | LOCK_NB: one writer owns both publication and crash recovery.
    if unsafe { flock(file.as_raw_fd(), 2 | 4) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Lock(file))
}
