use std::ffi::{CStr, c_char, c_int};

unsafe extern "C" {
    fn renamex_np(from: *const c_char, to: *const c_char, flags: u32) -> c_int;
}

pub(super) fn exchange(source: &CStr, destination: &CStr) -> c_int {
    unsafe { renamex_np(source.as_ptr(), destination.as_ptr(), 0x00000002) }
}
