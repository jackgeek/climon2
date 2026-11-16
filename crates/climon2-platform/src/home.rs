//! Where a climon2 executable finds its two homes: the climon2 home its
//! environment names, and the install layout around its own path. Every
//! executable finds them here, so each rule has one implementation.
use crate::runtime;
use climon2_core::{
    ContractError,
    layout::{ExecutableLayout, HomeLayout, VersionBinary},
};
use std::{
    io,
    path::{Path, PathBuf},
};

/// The climon2 home the environment names: `CLIMON2_HOME`, else `.climon2`
/// in `HOME`. It must be absolute; nothing on disk is touched.
pub fn named() -> io::Result<HomeLayout> {
    let user_home = std::env::var_os("HOME").unwrap_or_default();
    let override_value = std::env::var_os("CLIMON2_HOME");
    HomeLayout::resolve(override_value.as_deref(), Path::new(&user_home)).map_err(io::Error::other)
}

/// The named climon2 home, created when missing and canonicalized: the
/// invocation's one `runtime::canonical_home` call.
pub fn canonical() -> io::Result<PathBuf> {
    runtime::canonical_home(named()?.root())
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
