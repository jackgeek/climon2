//! Where a climon2 executable finds its two homes: the climon2 home its
//! environment names, and the install layout around its own path. Every
//! executable finds them here, so each rule has one implementation. The
//! environment variables each platform reads are its native names.
use climon2_core::{
    ContractError,
    layout::{ExecutableLayout, HomeLayout, VersionBinary},
};
use std::{
    io,
    path::{Path, PathBuf},
};

#[cfg(unix)]
mod native {
    /// The variable naming the user's home, which holds `.climon2`.
    pub(super) const USER_HOME: &str = "HOME";
    /// The variable naming the directory that holds the default executable home.
    pub(super) const EXECUTABLE_HOME_BASE: &str = "HOME";
    /// The default executable home within that directory.
    pub(super) const EXECUTABLE_HOME: &str = ".local/share/climon2";
}

#[cfg(windows)]
#[path = "home/windows.rs"]
mod native;

/// The climon2 home the environment names: `CLIMON2_HOME`, else `.climon2`
/// in the user's home (`HOME`; `USERPROFILE` on Windows). It must be
/// absolute; nothing on disk is touched.
pub fn named() -> io::Result<HomeLayout> {
    let user_home = std::env::var_os(native::USER_HOME).unwrap_or_default();
    let override_value = std::env::var_os("CLIMON2_HOME");
    HomeLayout::resolve(override_value.as_deref(), Path::new(&user_home)).map_err(io::Error::other)
}

/// The executable home an installer uses without `--dir`:
/// `.local/share/climon2` in `HOME`, or `climon2` in `LOCALAPPDATA` on
/// Windows. The variable must name an absolute directory; nothing on disk is
/// touched.
pub fn default_executable_home() -> io::Result<PathBuf> {
    let base = PathBuf::from(std::env::var_os(native::EXECUTABLE_HOME_BASE).unwrap_or_default());
    if !base.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} must name an absolute directory",
                native::EXECUTABLE_HOME_BASE
            ),
        ));
    }
    Ok(base.join(native::EXECUTABLE_HOME))
}

/// The named climon2 home, created when missing and canonicalized: the
/// invocation's one `runtime::canonical_home` call.
#[cfg(unix)]
pub fn canonical() -> io::Result<PathBuf> {
    crate::runtime::canonical_home(named()?.root())
}

/// The install layout around this running executable, which must be `binary`
/// in a version folder. The outer error is the system failing to name the
/// executable; the inner one, a real path outside any version folder.
pub fn executable_layout(
    binary: VersionBinary,
) -> io::Result<Result<ExecutableLayout, ContractError>> {
    let executable = std::env::current_exe()?.canonicalize()?;
    Ok(ExecutableLayout::from_binary_path(&executable, binary))
}
