use std::ffi::{CStr, c_char, c_int};

unsafe extern "C" {
    fn renameat2(
        olddirfd: c_int,
        oldpath: *const c_char,
        newdirfd: c_int,
        newpath: *const c_char,
        flags: u32,
    ) -> c_int;
}

/// Paths resolve from the working directory; the installer's are absolute, so it is unused.
const AT_FDCWD: c_int = -100;
const RENAME_EXCHANGE: u32 = 2;

pub(super) fn exchange(source: &CStr, destination: &CStr) -> c_int {
    unsafe {
        renameat2(
            AT_FDCWD,
            source.as_ptr(),
            AT_FDCWD,
            destination.as_ptr(),
            RENAME_EXCHANGE,
        )
    }
}
