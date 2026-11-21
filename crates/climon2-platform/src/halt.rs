//! Ending the process at once.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    /// Ends the whole process with `code` now, without running exit handlers
    /// or library destructors, so no code still running on another thread is
    /// torn down under it. Buffered output not yet flushed is lost.
    pub fn halt(code: i32) -> ! {
        // SAFETY: `_exit` only ends the process.
        unsafe { libc::_exit(code) }
    }
}
