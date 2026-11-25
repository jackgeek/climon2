use climon2_core::metadata::{MachineId, SessionOrigin};
use std::{fs, io, os::unix::fs::MetadataExt};

/// The machine, from systemd's `/etc/machine-id`, which survives reboots unlike
/// the boot ID, and the PID namespace, as the inode of `/proc/self/ns/pid`. A
/// PID namespace that cannot be read is an error; a machine without an ID (as
/// in many container images) is told apart by its namespace alone.
pub fn current() -> io::Result<SessionOrigin> {
    Ok(SessionOrigin {
        machine_id: machine_id(),
        pid_namespace: Some(fs::metadata("/proc/self/ns/pid")?.ino()),
    })
}

fn machine_id() -> Option<MachineId> {
    fs::read_to_string("/etc/machine-id")
        .ok()?
        .trim()
        .parse()
        .ok()
}
