//! The Library layer's harness: the installed client library, loaded by the
//! `climon2-library-host` fixture through the shared validated loader and the
//! frozen C ABI only.

use super::{Package, TIMEOUT};
use climon2_core::layout::ExecutableHome;
use climon2_test_term::{bin_path, output_within, process_command};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// This build's version, whose library the package installs.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The package's executable home, canonical as the monitor resolves it.
pub fn home(package: &Package) -> ExecutableHome {
    ExecutableHome::new(&package.executable_home().canonicalize().unwrap()).unwrap()
}

/// `version`'s library in `home`, by absolute path.
pub fn library(home: &ExecutableHome, version: &str) -> PathBuf {
    home.client_library(&version.parse().unwrap())
}

/// The host loading `version`'s library from `home`, with a cleared
/// environment; instrumented builds keep their profile.
pub fn host(home: &ExecutableHome, version: &str, directory: &Path) -> Command {
    host_at(&bin_path("climon2-library-host"), home, version, directory)
}

/// `host`, run from a copy of the host executable at `executable`.
pub fn host_at(
    executable: &Path,
    home: &ExecutableHome,
    version: &str,
    directory: &Path,
) -> Command {
    let mut command = process_command(executable);
    command
        .arg(home.path())
        .arg(version)
        .current_dir(directory)
        .env_clear();
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

/// The host's one JSON report. It must exit successfully and nothing, neither
/// the library nor the loader, may write to its standard error.
pub fn observe(mut host: Command) -> Value {
    let output = output_within(&mut host, TIMEOUT);
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "library or loader wrote: {output:?}"
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// Whether the library under test is an exact release archive's, which has
/// no test hooks.
pub fn archive() -> bool {
    std::env::var_os("CLIMON2_E2E_ARCHIVE").is_some()
}

/// One host scenario over the installed library, with `fault` selecting the
/// hooked library's `CLIMON2_LIBRARY_FAULT`.
pub fn scenario(package: &Package, name: &str, input: &Value, fault: Option<&str>) -> Value {
    let mut command = host(&home(package), VERSION, package.isolation.home());
    command.arg(name).arg(input.to_string());
    if let Some(fault) = fault {
        command.env("CLIMON2_LIBRARY_FAULT", fault);
    }
    observe(command)
}
