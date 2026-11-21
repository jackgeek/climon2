//! The system's limits on paths.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    /// `PATH_MAX`: a native path, with its terminating NUL, is shorter.
    pub const PATH_MAX: usize = libc::PATH_MAX as usize;
}
