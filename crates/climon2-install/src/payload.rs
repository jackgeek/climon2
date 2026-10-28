use climon2_core::VersionName;
use std::io;
use std::path::{Path, PathBuf};

pub(crate) const COMPANIONS: [&str; 3] = ["climon2-cli", "climon2-daemon", "climon2-client"];

/// Sources are read in place, preserving instrumented binaries without Cargo builds.
#[derive(Debug)]
pub struct Payload {
    version: VersionName,
    pub(crate) launcher: PathBuf,
    pub(crate) companions: [PathBuf; 3],
}

impl Payload {
    /// Locate the packaged release beside this executable, resolving symlinks.
    pub fn beside_installer() -> io::Result<Self> {
        let executable = std::env::current_exe()?.canonicalize()?;
        let directory = executable.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "installer has no parent directory",
            )
        })?;
        Self::from_directory(directory, env!("CARGO_PKG_VERSION"))
    }

    /// Read the release archive's `climon2` and `<version>/climon2-*` siblings.
    pub fn from_directory(directory: &Path, version: &str) -> io::Result<Self> {
        Self::from_paths(
            version,
            directory.join("climon2"),
            COMPANIONS.map(|name| directory.join(version).join(name)),
        )
    }

    /// Build a test/coverage layout from Cargo output siblings, without rebuilding.
    pub fn from_siblings(directory: &Path, version: &str) -> io::Result<Self> {
        Self::from_paths(
            version,
            directory.join("climon2"),
            COMPANIONS.map(|name| directory.join(name)),
        )
    }

    fn from_paths(version: &str, launcher: PathBuf, companions: [PathBuf; 3]) -> io::Result<Self> {
        let version = version
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let payload = Self {
            version,
            launcher,
            companions,
        };
        payload.validate()?;
        Ok(payload)
    }

    pub fn version(&self) -> &VersionName {
        &self.version
    }

    pub(crate) fn validate(&self) -> io::Result<()> {
        validate_file(&self.launcher)?;
        for path in &self.companions {
            validate_file(path)?;
        }
        Ok(())
    }
}

fn validate_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| io::Error::new(error.kind(), format!("payload {path:?}: {error}")))?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o111 == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("payload {path:?} is not a regular executable"),
        ));
    }
    Ok(())
}
