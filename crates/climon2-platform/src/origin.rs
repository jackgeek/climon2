//! Where this process runs, which a session daemon records in its metadata as
//! `daemon_origin` (#133, ADR 0005 "Optional session origin"): its machine and,
//! on Linux, its PID namespace. Only a reader with the same origin can prove the
//! daemon gone. Part of the process-identity seam (ADR 0003, seam 10), kept
//! apart from `process` so each body lives in its own native file.
use climon2_core::metadata::SessionOrigin;
use std::{io, sync::OnceLock};

/// This process's origin. A process never changes machine or PID namespace,
/// so the first origin read is kept for the life of the process: discovery
/// asks once per refused probe, and `gethostuuid` can be slow. A failed read
/// is not kept, so a later call reads again.
pub fn current() -> io::Result<SessionOrigin> {
    static HERE: OnceLock<SessionOrigin> = OnceLock::new();
    match HERE.get() {
        Some(here) => Ok(here.clone()),
        None => native::current().map(|here| HERE.get_or_init(|| here).clone()),
    }
}

#[cfg(target_os = "linux")]
#[path = "origin/linux.rs"]
mod native;

#[cfg(target_os = "macos")]
#[path = "origin/macos.rs"]
mod native;

#[cfg(windows)]
#[path = "origin/windows.rs"]
mod native;
