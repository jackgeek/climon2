//! Where tests find the [second-release fixture](../../../GLOSSARY.md#testing)
//! (#97): the test-only `climon2-second-release` package's client library and
//! session daemon, which Cargo builds into the target directory without
//! nested builds. Neither is ever shipped.
use std::{
    env::consts::{DLL_PREFIX, DLL_SUFFIX},
    path::{Path, PathBuf},
};

/// The second-release fixture's version: the `climon2-second-release` package's
/// version, which both artifacts carry as their release identity.
pub const VERSION: &str = "0.0.97";

/// The second-release fixture's session daemon executable, beside the other
/// workspace binaries.
pub const DAEMON: &str = "climon2-second-release-daemon";

/// The second-release fixture's client library in `binaries`, a Cargo output
/// directory such as `target/debug`: a dependency's cdylib stays in `deps/`.
pub fn library(binaries: &Path) -> PathBuf {
    binaries
        .join("deps")
        .join(format!("{DLL_PREFIX}climon2_second_release{DLL_SUFFIX}"))
}
