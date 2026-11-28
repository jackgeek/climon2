//! Where this process runs (#133): its machine and, on Linux, its PID
//! namespace. A climon2 home's owner record holds its owner's (ADR 0011), and
//! a session daemon records its own as `daemon_origin` for diagnostics (ADR
//! 0005). Part of the process-identity seam (ADR 0003, seam 10), kept apart
//! from `process` so each body lives in its own native file.
use climon2_core::metadata::SessionOrigin;
use std::{io, sync::OnceLock};

#[cfg(target_os = "linux")]
pub use native::{Sources, current_at};

/// This process's origin. A process never changes machine or PID namespace,
/// so the first origin read is kept for the life of the process:
/// `gethostuuid` can be slow. A failed read is not kept, so a later call reads
/// again.
pub fn current() -> io::Result<SessionOrigin> {
    static HERE: OnceLock<SessionOrigin> = OnceLock::new();
    match HERE.get() {
        Some(here) => Ok(here.clone()),
        None => native::here().map(|here| HERE.get_or_init(|| here).clone()),
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
