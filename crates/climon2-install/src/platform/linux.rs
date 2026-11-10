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

pub(super) fn exchange(source: &CStr, destination: &CStr) -> c_int {
    unsafe { renameat2(-100, source.as_ptr(), -100, destination.as_ptr(), 2) }
}
