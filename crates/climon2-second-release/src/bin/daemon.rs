//! Test-only fixture, never shipped (#97): the session daemon of the
//! [second-release fixture](../../../../GLOSSARY.md#testing). It runs the
//! `climon2-daemon` library unchanged behind this thin main, which differs
//! from the shipped one in two deliberate ways:
//!
//! - its release identity is compiled in: it serves only this package's
//!   version, and refuses an invocation naming any other, so no other
//!   release's CLI can start it (a shipped daemon serves whichever release
//!   its CLI names);
//! - its private behaviour differs: it gives its session's command
//!   `CLIMON2_SECOND_RELEASE_DAEMON` set to its release, which a test's
//!   command can turn into its title, so a client library relays, over
//!   revision 1's `title-v1`, which release's daemon it is connected to.
//!
//! Its hello reply and session metadata carry the release it serves, as
//! every daemon's do; neither the frozen hello nor revision 1 changes.

use std::{ffi::OsString, process::ExitCode};

/// This daemon's release identity, compiled in.
const RELEASE: &str = env!("CARGO_PKG_VERSION");
/// What its session's command finds in its environment.
const RELEASE_VARIABLE: &str = "CLIMON2_SECOND_RELEASE_DAEMON";

fn main() -> ExitCode {
    let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
    if arguments
        .first()
        .is_some_and(|first| first == "--exec-command")
    {
        return ExitCode::from(climon2_daemon::run(arguments));
    }
    if !names_its_release(&arguments) {
        eprintln!("climon2: error: this daemon serves only release {RELEASE}");
        return ExitCode::from(125);
    }
    // SAFETY: the process is still single-threaded.
    unsafe { std::env::set_var(RELEASE_VARIABLE, RELEASE) };
    ExitCode::from(climon2_daemon::run(arguments))
}

/// Whether the invocation's options, the `KEY VALUE` pairs before `--`,
/// name this release as `--version`.
fn names_its_release(arguments: &[OsString]) -> bool {
    let options: Vec<&OsString> = arguments.iter().take_while(|word| *word != "--").collect();
    options
        .chunks_exact(2)
        .any(|pair| pair[0] == "--version" && pair[1] == RELEASE)
}
