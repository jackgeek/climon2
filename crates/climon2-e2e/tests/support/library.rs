//! The Library layer's harness: the installed client library, loaded by the
//! `climon2-library-host` fixture through the frozen C ABI only.

use super::{LIBRARY, Package, TIMEOUT};
use climon2_test_term::{bin_path, output_within, process_command};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

/// The installed library of this package's version, by absolute path.
pub fn installed_library(package: &Package) -> PathBuf {
    package
        .executable_home()
        .join(env!("CARGO_PKG_VERSION"))
        .join(LIBRARY)
        .canonicalize()
        .unwrap()
}

/// The host with a cleared environment; instrumented builds keep their profile.
pub fn host(library: &Path, directory: &Path) -> Command {
    let mut command = process_command(bin_path("climon2-library-host"));
    command.arg(library).current_dir(directory).env_clear();
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
    let library = installed_library(package);
    let mut command = host(&library, package.isolation.home());
    command.arg(name).arg(input.to_string());
    if let Some(fault) = fault {
        command.env("CLIMON2_LIBRARY_FAULT", fault);
    }
    observe(command)
}
