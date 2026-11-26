//! The machine, from systemd's `/etc/machine-id`, which survives reboots unlike
//! the boot ID, and the PID namespace, as the inode of `/proc/self/ns/pid`. A
//! PID namespace that cannot be read is an error.
//!
//! Only a stable machine ID is recorded (ADR 0011). A machine without one (as
//! in many container images), or whose ID does not survive a reboot (the
//! transient one systemd mounts from `/run/machine-id` over
//! `/etc/machine-id`, or one on the tmpfs or overlay root of a live system),
//! is told apart by its PID namespace alone.
use climon2_core::metadata::{MachineId, SessionOrigin};
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

/// Where Linux reads its origin from: the system's files, or a test's.
#[derive(Clone, Debug)]
pub struct Sources {
    /// systemd's machine ID, `/etc/machine-id`.
    pub machine_id: PathBuf,
    /// This process's mounts, `/proc/self/mountinfo`.
    pub mountinfo: PathBuf,
    /// This process's PID namespace, `/proc/self/ns/pid`.
    pub pid_namespace: PathBuf,
}

pub fn here() -> io::Result<SessionOrigin> {
    current_at(&Sources {
        machine_id: "/etc/machine-id".into(),
        mountinfo: "/proc/self/mountinfo".into(),
        pid_namespace: "/proc/self/ns/pid".into(),
    })
}

/// The origin `sources` describe, read now.
pub fn current_at(sources: &Sources) -> io::Result<SessionOrigin> {
    let pid_namespace = fs::metadata(&sources.pid_namespace)?.ino();
    Ok(SessionOrigin {
        machine_id: machine_id(&sources.machine_id).filter(|_| !transient(&sources.mountinfo)),
        pid_namespace: Some(pid_namespace),
    })
}

fn machine_id(path: &Path) -> Option<MachineId> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// The mount points that can hold `/etc/machine-id`: the file itself, the
/// folder it is in, and the root.
const HOLDERS: [&str; 3] = ["/etc/machine-id", "/etc", "/"];
/// File systems whose contents do not survive a reboot as they are: systemd's
/// transient ID on the tmpfs at `/run`, and the tmpfs or overlay root of a
/// live system, which writes a fresh ID there at every boot.
const VOLATILE: [&str; 2] = ["tmpfs", "overlay"];

/// Whether `/etc/machine-id` is not stable: the last mount that holds it is a
/// tmpfs or an overlay. A container's bind mount of its host's real file is a
/// mount of the host's file system, and unreadable mounts are taken as none.
fn transient(mountinfo: &Path) -> bool {
    let mounts = fs::read_to_string(mountinfo).unwrap_or_default();
    mounts
        .lines()
        .filter_map(mount)
        .rfind(|(point, _)| HOLDERS.contains(point))
        .is_some_and(|(_, kind)| VOLATILE.contains(&kind))
}

/// One `mountinfo` line's mount point and file system type.
fn mount(line: &str) -> Option<(&str, &str)> {
    let (fields, tail) = line.split_once(" - ")?;
    let point = fields.split(' ').nth(4)?;
    Some((point, tail.split(' ').next()?))
}
