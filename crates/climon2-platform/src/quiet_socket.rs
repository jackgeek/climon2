//! Sending on a socket without `SIGPIPE`: a closed peer fails the send with
//! `EPIPE` instead of raising a signal in a process whose signal handling
//! climon2 does not own, such as a client library's embedder.
#[cfg(target_os = "linux")]
#[path = "quiet_socket/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "quiet_socket/macos.rs"]
mod platform;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use platform::{prepare, send};
