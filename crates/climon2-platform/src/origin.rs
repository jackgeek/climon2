//! Where this process runs, which a session daemon records in its metadata as
//! `daemon_origin` (#133, ADR 0005 "Optional session origin"): its machine and,
//! on Linux, its PID namespace. Only a reader with the same origin can prove the
//! daemon gone. Part of the process-identity seam (ADR 0003, seam 10), kept
//! apart from `process` so each body lives in its own native file.
pub use native::current;

#[cfg(target_os = "linux")]
#[path = "origin/linux.rs"]
mod native;

#[cfg(target_os = "macos")]
#[path = "origin/macos.rs"]
mod native;

#[cfg(windows)]
#[path = "origin/windows.rs"]
mod native;
