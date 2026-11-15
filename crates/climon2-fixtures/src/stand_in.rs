//! Builds the client library stand-in, `stand-in/client-library.c`: a clearly
//! labelled, test-only client library that is never shipped. Tests compile it
//! exact or with one defect; with a defect it is the faulty library.
use crate::run;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

/// The library the dependent stand-in needs, which nothing installs.
pub const DEPENDENCY: &str = "climon2_stand_in_dependency";

fn stand_in() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("stand-in")
}

/// Compiles the stand-in describing `release` to `output`, with `options`:
/// its defect macros, architectures or link options. Like rustc's cdylib, the
/// library exports only `climon2_client_*` symbols; on macOS it is ad-hoc
/// signed. Compiler diagnostics go to `<output>.log`.
pub fn compile(output: &Path, release: &str, options: &[&str]) -> io::Result<()> {
    let mut command = Command::new("cc");
    #[cfg(target_os = "linux")]
    command.args(["-shared", "-fPIC"]).arg(format!(
        "-Wl,--version-script={}",
        stand_in().join("client-library.map").display()
    ));
    #[cfg(target_os = "macos")]
    command.arg("-dynamiclib");
    command
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
        .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/protocol/monitor-client"))
        .arg(format!("-DRELEASE=\"{release}\""))
        .args(options)
        .arg("-o")
        .arg(output)
        .arg(stand_in().join("client-library.c"));
    run(&mut command, &output.with_extension("log"))?;
    #[cfg(target_os = "macos")]
    run(
        Command::new("codesign")
            .args(["--force", "-s", "-"])
            .arg(output),
        &output.with_extension("log"),
    )?;
    Ok(())
}

/// Compiles the exact stand-in describing `release` to `output`, needing a
/// library that is found nowhere: it links one built in `scratch`, which it
/// then deletes. On macOS that library would sit beside the stand-in.
pub fn compile_needing_a_missing_library(
    output: &Path,
    release: &str,
    scratch: &Path,
) -> io::Result<()> {
    let name = format!(
        "{}{DEPENDENCY}{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    let dependency = scratch.join(&name);
    let mut command = Command::new("cc");
    #[cfg(target_os = "linux")]
    command.args(["-shared", "-fPIC"]);
    #[cfg(target_os = "macos")]
    command
        .arg("-dynamiclib")
        .arg(format!("-Wl,-install_name,@loader_path/{name}"));
    run(
        command
            .args(["-x", "c", "/dev/null", "-o"])
            .arg(&dependency),
        &dependency.with_extension("log"),
    )?;
    let search = format!("-L{}", scratch.display());
    let link = format!("-l{DEPENDENCY}");
    // Ubuntu's toolchain drops an unused needed library unless told not to.
    let keep: &[&str] = if cfg!(target_os = "linux") {
        &["-Wl,--no-as-needed"]
    } else {
        &[]
    };
    compile(output, release, &[keep, &[search.as_str(), &link]].concat())?;
    fs::remove_file(dependency)
}

/// Rewrites the ELF header of the library at `path` to name AArch64, so that
/// it is not a library for an x86-64 process.
pub fn name_aarch64(path: &Path) -> io::Result<()> {
    const AARCH64: u16 = 183;
    let mut bytes = fs::read(path)?;
    bytes[18..20].copy_from_slice(&AARCH64.to_le_bytes());
    fs::write(path, bytes)
}
