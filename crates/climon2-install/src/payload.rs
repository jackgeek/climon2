use climon2_core::{VersionName, layout::client_library_name};
use std::io;
use std::path::{Path, PathBuf};

/// The executables a release places in its version folder.
const COMPANIONS: [&str; 4] = [
    "climon2-cli",
    "climon2-daemon",
    "climon2-client",
    "climon2-monitor",
];

/// Every file a release places in its version folder, in installation order:
/// the executables, then the client library.
pub fn version_files() -> Vec<String> {
    COMPANIONS
        .into_iter()
        .map(str::to_owned)
        .chain([client_library_name()])
        .collect()
}

/// Where a Cargo build leaves a version-folder file: Cargo uplifts binaries
/// into its output directory, but leaves a dependency's cdylib in `deps/`.
pub fn cargo_output(directory: &Path, name: &str) -> PathBuf {
    if name == client_library_name() {
        directory.join("deps").join(name)
    } else {
        directory.join(name)
    }
}

/// Sources are read in place, preserving instrumented binaries without Cargo builds.
#[derive(Debug)]
pub struct Payload {
    version: VersionName,
    pub(crate) launcher: PathBuf,
    /// Each version-folder file's name and source, in installation order.
    pub(crate) companions: Vec<(String, PathBuf)>,
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

    /// Read the release archive's `climon2` and `<version>/` files.
    pub fn from_directory(directory: &Path, version: &str) -> io::Result<Self> {
        Self::from_paths(version, directory.join("climon2"), |name| {
            directory.join(version).join(name)
        })
    }

    /// Build a test/coverage layout from Cargo output siblings, without rebuilding.
    pub fn from_siblings(directory: &Path, version: &str) -> io::Result<Self> {
        Self::from_paths(version, directory.join("climon2"), |name| {
            cargo_output(directory, name)
        })
    }

    fn from_paths(
        version: &str,
        launcher: PathBuf,
        source: impl Fn(&str) -> PathBuf,
    ) -> io::Result<Self> {
        let version = version
            .parse()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        let companions = version_files()
            .into_iter()
            .map(|name| {
                let path = source(&name);
                (name, path)
            })
            .collect();
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
        for (_, path) in &self.companions {
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
