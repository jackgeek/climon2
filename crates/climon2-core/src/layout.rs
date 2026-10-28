//! Pure layout rules. Paths documented as canonical must be canonicalized by
//! the caller; this module never touches the filesystem or repairs permissions.
use crate::{ContractError, SessionId, VersionName};
use sha2::{Digest, Sha256};
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Invocation {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HomeLayout(PathBuf);

impl HomeLayout {
    pub fn resolve(
        override_value: Option<&OsStr>,
        user_home: &Path,
    ) -> Result<Self, ContractError> {
        let path = match override_value {
            Some(value) => PathBuf::from(value),
            None => user_home.join(".climon2"),
        };
        absolute(&path)?;
        Ok(Self(path))
    }

    pub fn root(&self) -> &Path {
        &self.0
    }

    pub fn sessions(&self) -> PathBuf {
        self.0.join("sessions")
    }

    pub fn metadata(&self, id: &SessionId) -> PathBuf {
        self.sessions().join(format!("{id}.pty"))
    }

    pub fn session_logs(&self, id: &SessionId) -> PathBuf {
        self.0.join("logs").join(id.as_str())
    }

    pub fn general_logs(&self) -> PathBuf {
        self.0.join("logs/general")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VersionBinary {
    Cli,
    Daemon,
    Client,
}

impl VersionBinary {
    fn name(self) -> &'static str {
        match self {
            Self::Cli => "climon2-cli",
            Self::Daemon => "climon2-daemon",
            Self::Client => "climon2-client",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutableLayout {
    root: PathBuf,
    version: VersionName,
    suffix: &'static str,
}

impl ExecutableLayout {
    pub fn from_cli_path(canonical_cli: &Path) -> Result<Self, ContractError> {
        canonical_absolute(canonical_cli)?;
        let suffix = Self::cli_suffix(canonical_cli)?;
        let (root, folder) = Self::home_and_version_folder(canonical_cli)
            .ok_or_else(|| ContractError("invalid version folder".into()))?;
        Ok(Self {
            root: root.to_owned(),
            version: folder.parse()?,
            suffix,
        })
    }

    /// The CLI lives at `<home>/<version>/climon2-cli`.
    fn home_and_version_folder(cli: &Path) -> Option<(&Path, &str)> {
        let folder = cli.parent()?;
        let name = folder.file_name()?.to_str()?;
        Some((folder.parent()?, name))
    }

    fn cli_suffix(path: &Path) -> Result<&'static str, ContractError> {
        match path.file_name().and_then(OsStr::to_str) {
            Some("climon2-cli") => Ok(""),
            Some("climon2-cli.exe") => Ok(".exe"),
            _ => Err(ContractError("expected canonical climon2-cli path".into())),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn version(&self) -> &VersionName {
        &self.version
    }

    pub fn binary(&self, version: &VersionName, binary: VersionBinary) -> PathBuf {
        self.root
            .join(version.as_str())
            .join(format!("{}{}", binary.name(), self.suffix))
    }

    /// ADR 0001/#28: no additional flags; configuration travels via environment.
    pub fn client_invocation(
        &self,
        version: &VersionName,
        metadata: &Path,
    ) -> Result<Invocation, ContractError> {
        absolute(metadata)?;
        Ok(Invocation {
            program: self.binary(version, VersionBinary::Client),
            args: vec![
                "attach".into(),
                "--metadata".into(),
                metadata.as_os_str().to_owned(),
            ],
        })
    }
}

pub fn parse_current(contents: &str) -> Result<VersionName, ContractError> {
    contents
        .strip_suffix("\r\n")
        .or_else(|| contents.strip_suffix('\n'))
        .unwrap_or(contents)
        .parse()
}

/// SHA-256 input is the canonical home's exact native encoded bytes, never lossy
/// display text. A runtime name is local to the same machine and OS.
pub fn home_hash8(canonical_home_bytes: &[u8]) -> String {
    let hash = Sha256::digest(canonical_home_bytes);
    hash[..4].iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeLayout(PathBuf);

impl RuntimeLayout {
    /// Platform selects the runtime root; core supplies only naming and bounds.
    pub fn new(
        runtime_root: &Path,
        uid: u32,
        canonical_home: &Path,
    ) -> Result<Self, ContractError> {
        absolute(runtime_root)?;
        canonical_absolute(canonical_home)?;
        let hash = home_hash8(canonical_home.as_os_str().as_encoded_bytes());
        Ok(Self(runtime_root.join(format!("climon2-{uid}-{hash}"))))
    }

    pub fn directory(&self) -> &Path {
        &self.0
    }

    pub fn socket_path(&self, id: &SessionId, max_bytes: usize) -> Result<PathBuf, ContractError> {
        let path = self.0.join(format!("{id}.sock"));
        if path.as_os_str().as_encoded_bytes().len() > max_bytes {
            return Err(ContractError("socket path exceeds platform limit".into()));
        }
        Ok(path)
    }
}

fn absolute(path: &Path) -> Result<(), ContractError> {
    if !path.is_absolute() {
        return Err(ContractError("path must be nonempty and absolute".into()));
    }
    Ok(())
}

fn canonical_absolute(path: &Path) -> Result<(), ContractError> {
    absolute(path)?;
    if path
        .components()
        .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(ContractError("path must be canonical".into()));
    }
    Ok(())
}
