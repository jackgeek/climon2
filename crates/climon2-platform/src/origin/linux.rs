//! The machine, from systemd's `/etc/machine-id`, which survives reboots unlike
//! the boot ID, and the PID namespace, as the inode of `/proc/self/ns/pid`. A
//! PID namespace that cannot be read is an error; a machine without an ID (as
//! in many container images) is told apart by its namespace alone.
//!
//! On a live or read-only-root system systemd generates a transient machine ID
//! at each boot and mounts it from `/run/machine-id` over `/etc/machine-id`.
//! There climon2 generates its own machine ID once and keeps it in the climon2
//! home, keyed by host name, at `machines/<hostname>.id` (ADR 0005).
use crate::private;
use climon2_core::metadata::{MachineId, SessionOrigin};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

/// Where Linux reads its origin from: the system's files, or a test's.
#[derive(Clone, Debug)]
pub struct Sources {
    /// systemd's machine ID, `/etc/machine-id`.
    pub machine_id: PathBuf,
    /// This process's mounts, `/proc/self/mountinfo`.
    pub mountinfo: PathBuf,
    /// The host name, `/proc/sys/kernel/hostname`.
    pub hostname: PathBuf,
    /// This process's PID namespace, `/proc/self/ns/pid`.
    pub pid_namespace: PathBuf,
}

impl Sources {
    fn system() -> Self {
        Self {
            machine_id: "/etc/machine-id".into(),
            mountinfo: "/proc/self/mountinfo".into(),
            hostname: "/proc/sys/kernel/hostname".into(),
            pid_namespace: "/proc/self/ns/pid".into(),
        }
    }
}

/// What the system says about where this process runs.
pub struct Here {
    machine: Machine,
    pid_namespace: u64,
    hostname: PathBuf,
}

/// What `/etc/machine-id` says about this machine.
enum Machine {
    /// Missing or unreadable: not transient, so the PID namespace alone tells
    /// this machine apart (ADR 0005).
    Missing,
    Stable(MachineId),
    /// Mounted from systemd's `/run/machine-id`, so it changes at each boot.
    Transient,
}

pub fn here() -> io::Result<Here> {
    read(&Sources::system())
}

pub fn origin(here: &Here, home: &Path) -> SessionOrigin {
    let machine_id = match &here.machine {
        Machine::Missing => None,
        Machine::Stable(machine) => Some(machine.clone()),
        Machine::Transient => own_machine_id(&here.hostname, home),
    };
    SessionOrigin {
        machine_id,
        pid_namespace: Some(here.pid_namespace),
    }
}

/// The origin `sources` describe, for the climon2 home at `home`, read now.
pub fn current_at(sources: &Sources, home: &Path) -> io::Result<SessionOrigin> {
    Ok(origin(&read(sources)?, home))
}

fn read(sources: &Sources) -> io::Result<Here> {
    let pid_namespace = fs::metadata(&sources.pid_namespace)?.ino();
    let machine = match machine_id(&sources.machine_id) {
        None => Machine::Missing,
        Some(_) if transient(&sources.mountinfo) => Machine::Transient,
        Some(machine) => Machine::Stable(machine),
    };
    Ok(Here {
        machine,
        pid_namespace,
        hostname: sources.hostname.clone(),
    })
}

fn machine_id(path: &Path) -> Option<MachineId> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

/// Whether the last mount at `/etc/machine-id` is systemd's transient one: the
/// file `/machine-id` of the tmpfs at `/run`. A container's bind mount of its
/// host's real file has another root or file system, and unreadable mounts
/// are taken as none.
fn transient(mountinfo: &Path) -> bool {
    let mounts = fs::read_to_string(mountinfo).unwrap_or_default();
    mounts
        .lines()
        .filter_map(mount)
        .rfind(|(_, point, _)| *point == "/etc/machine-id")
        .is_some_and(|(root, _, kind)| root == "/machine-id" && kind == "tmpfs")
}

/// One `mountinfo` line's root, mount point and file system type.
fn mount(line: &str) -> Option<(&str, &str, &str)> {
    let (fields, tail) = line.split_once(" - ")?;
    let mut fields = fields.split(' ').skip(3);
    let root = fields.next()?;
    let point = fields.next()?;
    Some((root, point, tail.split(' ').next()?))
}

/// climon2's own machine ID for this host name, generated once. Without it
/// the record carries no machine ID, as on a machine without one, and the
/// reason is logged.
fn own_machine_id(hostname: &Path, home: &Path) -> Option<MachineId> {
    persisted(hostname, home)
        .inspect_err(|error| {
            tracing::warn!(%error, errno = error.raw_os_error(), "own machine ID unavailable")
        })
        .ok()
}

fn persisted(hostname: &Path, home: &Path) -> io::Result<MachineId> {
    let name = host_name(&fs::read_to_string(hostname)?)?;
    let machines = home.join("machines");
    let path = machines.join(format!("{name}.id"));
    match load(&path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            private::private_directory(&machines)?;
            create(&machines, &name, &path)
        }
        result => result,
    }
}

/// A host name that is one safe file name: ASCII letters, digits, hyphens,
/// dots and underscores, not starting with a dot.
fn host_name(text: &str) -> io::Result<String> {
    let name = text.trim();
    let allowed = |character: char| character.is_ascii_alphanumeric() || "-._".contains(character);
    if name.is_empty() || name.starts_with('.') || !name.chars().all(allowed) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "host name is not a file name",
        ));
    }
    Ok(name.to_owned())
}

/// Reads a kept ID without following a symlink. Anything but 32 lowercase hex
/// digits and a newline is refused, never replaced.
fn load(path: &Path) -> io::Result<MachineId> {
    let mut text = String::new();
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?
        .read_to_string(&mut text)?;
    let id = text.strip_suffix('\n').unwrap_or_default();
    let hex = |character: char| matches!(character, '0'..='9' | 'a'..='f');
    if id.len() != 32 || !id.chars().all(hex) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid machine ID in {}", path.display()),
        ));
    }
    id.parse().map_err(io::Error::other)
}

/// Writes a new random ID to an owner-only file of its own, then links it into
/// place, so the ID appears whole and only once: when another process got
/// there first, its ID is the one used.
fn create(machines: &Path, name: &str, path: &Path) -> io::Result<MachineId> {
    let id = random_id()?;
    let temporary = machines.join(format!(".{name}.{id}.tmp"));
    let mut file = private::create_private_file(&temporary)?;
    let linked = file
        .write_all(format!("{id}\n").as_bytes())
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::hard_link(&temporary, path));
    fs::remove_file(&temporary)?;
    // Whichever ID is in place is the one; without one, the failure that kept
    // ours out says why.
    load(path).map_err(|loaded| linked.err().unwrap_or(loaded))
}

/// 128 random bits as 32 lowercase hex digits, as systemd writes machine IDs.
fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
