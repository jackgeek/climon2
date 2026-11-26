//! Where this process runs, which a session daemon records in its metadata as
//! `daemon_origin` (#133, ADR 0005 "Optional session origin"): its machine and,
//! on Linux, its PID namespace. Only a reader in the same place can prove the
//! daemon gone. Part of the process-identity seam (ADR 0003, seam 10), kept
//! apart from `process` so each body lives in its own native file.
use climon2_core::metadata::SessionOrigin;
use std::{io, path::Path, sync::OnceLock};

#[cfg(target_os = "linux")]
pub use native::{Sources, current_at};

/// This process's origin, for the climon2 home at `home`. A process never
/// changes machine or PID namespace, so what the system says is read once
/// and kept for the life of the process: discovery asks once per refused
/// probe, and `gethostuuid` can be slow. A failed read is not kept, so a
/// later call reads again. Only Linux uses `home`: there climon2 keeps its own
/// machine ID in the home when systemd's is transient.
pub fn current(home: &Path) -> io::Result<SessionOrigin> {
    static HERE: OnceLock<native::Here> = OnceLock::new();
    let here = match HERE.get() {
        Some(here) => here,
        None => {
            let read = native::here()?;
            HERE.get_or_init(|| read)
        }
    };
    native::origin(here, home)
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
