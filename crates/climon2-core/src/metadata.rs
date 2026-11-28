//! ADR 0005 frozen, write-once record. File permissions, atomic publication,
//! liveness and deletion belong to platform/callers, not to this pure parser.
pub use crate::identity::{MachineId, MetadataPath};
use crate::{ContractError, SessionId, Timestamp, VersionName, identity::validate_guid};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{OsStr, OsString},
    num::NonZeroU32,
    path::Path,
};

/// One-way display projection; runtime boundaries retain their original OsStrings.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisplayCommand {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: MetadataPath,
}

impl DisplayCommand {
    pub fn from_runtime(
        command: &OsStr,
        args: &[OsString],
        cwd: &Path,
    ) -> Result<Self, ContractError> {
        if !cwd.is_absolute() {
            return Err(ContractError("runtime cwd must be absolute".into()));
        }
        Ok(Self {
            command: command.to_string_lossy().into_owned(),
            args: args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
            cwd: cwd.to_string_lossy().parse()?,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LinuxProcessIdentity {
    pub boot_id: String,
    pub start_ticks: u64,
}

impl LinuxProcessIdentity {
    fn validate(&self) -> Result<(), ContractError> {
        validate_guid(&self.boot_id).map_err(|_| ContractError("invalid Linux boot ID".into()))
    }
}

/// Where a process runs (#133): its machine and, on Linux, its PID namespace.
/// A session daemon records its own as `daemon_origin` for diagnostics (ADR
/// 0005), and a climon2 home records its owner's (ADR 0011).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionOrigin {
    /// Absent where the machine has no stable identity to read, as in many
    /// container images without `/etc/machine-id`, or where Linux's is
    /// transient.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine_id: Option<MachineId>,
    /// The inode of the process's `/proc/self/ns/pid`; Linux only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid_namespace: Option<u64>,
}

impl SessionOrigin {
    /// Whether two origins are the same host (ADR 0011): when both have a
    /// stable machine ID, it alone decides, since WSL2, LXC/LXD,
    /// systemd-nspawn and Flatpak get a new PID namespace at every start.
    /// Otherwise the PID namespace decides.
    pub fn is_same_place(&self, other: &Self) -> bool {
        match (&self.machine_id, &other.machine_id) {
            (Some(machine), Some(other)) => machine == other,
            _ => self.pid_namespace == other.pid_namespace,
        }
    }
}

/// For diagnostics: `machine <ID>, PID namespace <inode>`, naming only what is
/// recorded.
impl std::fmt::Display for SessionOrigin {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.machine_id, self.pid_namespace) {
            (Some(machine), Some(namespace)) => {
                write!(out, "machine {machine}, PID namespace {namespace}")
            }
            (Some(machine), None) => write!(out, "machine {machine}"),
            (None, Some(namespace)) => write!(out, "PID namespace {namespace}"),
            (None, None) => out.write_str("an unknown machine"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub format_version: NonZeroU32,
    pub session_id: SessionId,
    pub socket_path: MetadataPath,
    pub daemon_pid: u32,
    /// Microseconds since the Unix epoch, from the process-start-time seam.
    pub daemon_start_time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_linux_identity: Option<LinuxProcessIdentity>,
    /// Absent in records from older writers, and when the writer could not
    /// read where it runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_origin: Option<SessionOrigin>,
    /// Display only; never use these strings to reconstruct a command.
    pub command: String,
    pub args: Vec<String>,
    pub cwd: MetadataPath,
    pub depth: NonZeroU32,
    pub created_at: Timestamp,
    pub climon2_version: VersionName,
}

impl SessionMetadata {
    pub fn from_json(filename: &Path, bytes: &[u8]) -> Result<Self, ContractError> {
        let id = metadata_session_id(filename)?;
        let metadata: Self = crate::json::from_object(bytes)
            .map_err(|_| ContractError("invalid session metadata JSON or field".into()))?;
        metadata.validate_linux_identity()?;
        if metadata.session_id != id {
            return Err(ContractError(
                "metadata session ID does not match filename".into(),
            ));
        }
        Ok(metadata)
    }

    pub fn to_json(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    fn validate_linux_identity(&self) -> Result<(), ContractError> {
        self.daemon_linux_identity
            .as_ref()
            .map(LinuxProcessIdentity::validate)
            .transpose()
            .map(|_| ())
    }
}

/// Only canonical GUID stems with the exact `.pty` extension are candidates.
pub fn metadata_session_id(path: &Path) -> Result<SessionId, ContractError> {
    if path.extension().and_then(|part| part.to_str()) != Some("pty") {
        return Err(ContractError("metadata filename must end in .pty".into()));
    }
    path.file_stem()
        .and_then(|part| part.to_str())
        .ok_or_else(|| ContractError("metadata filename is not UTF-8".into()))?
        .parse()
}
