//! The home owner (ADR 0011, "One home, one host"): the host, one machine and
//! one PID namespace, that a climon2 home belongs to, as `<home>/owner`
//! records it. This is the record's format and the message of a refusal; the
//! file itself belongs to the platform.
use crate::{ContractError, metadata::SessionOrigin};
use std::fmt;

/// The owner record's name in the climon2 home.
pub const OWNER_FILE: &str = "owner";

/// The most bytes of an owner record that are read.
pub const OWNER_RECORD_LIMIT: u64 = 4096;

/// The owner a record names: a JSON object with the members of a session
/// origin, at least one of them present. Unknown members are ignored, so later
/// releases can add to it. A record naming neither a machine nor a PID
/// namespace would be the same place as every macOS and Windows host, so it is
/// refused.
pub fn from_json(bytes: &[u8]) -> Result<SessionOrigin, ContractError> {
    let invalid = || ContractError("invalid home owner record".into());
    let owner: SessionOrigin = crate::json::from_object(bytes).map_err(|_| invalid())?;
    if owner.machine_id.is_none() && owner.pid_namespace.is_none() {
        return Err(invalid());
    }
    Ok(owner)
}

pub fn to_json(owner: &SessionOrigin) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(owner)
}

/// Why a command refuses to use a climon2 home. Every refusal exits 125.
#[derive(Debug)]
pub enum Refusal {
    /// Another machine or container owns the home.
    Foreign(SessionOrigin),
    /// The home has an owner, and this host cannot read its own origin.
    Unchecked {
        owner: SessionOrigin,
        error: std::io::Error,
    },
    /// The owner record at `path` cannot be read, or is not a valid record.
    Unreadable {
        path: std::path::PathBuf,
        error: std::io::Error,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Foreign(owner) => write!(
                out,
                "this climon2 home is in use by another machine or container ({owner}). \
                 climon2 does not work across machines yet (`c2 relay` is planned). To move \
                 this home here, run `c2 home release`."
            ),
            Self::Unchecked { owner, error } => write!(
                out,
                "this climon2 home belongs to {owner}, and this host's machine and PID \
                 namespace could not be checked: {error}"
            ),
            Self::Unreadable { path, error } => write!(
                out,
                "this climon2 home's owner record {} cannot be read: {error}. To let the next \
                 command claim this home afresh, run `c2 home release` or delete {}.",
                path.display(),
                path.display()
            ),
        }
    }
}

impl std::error::Error for Refusal {}
