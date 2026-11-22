//! The client library's notifier: a pollable descriptor it raises and
//! clears, for its notification descriptor and each instance's waker.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("notifier seam requires macOS or Linux");

#[cfg(target_os = "linux")]
#[path = "notifier/linux.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "notifier/macos.rs"]
mod native;

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use native::Notifier;
