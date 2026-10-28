#[cfg(not(unix))]
compile_error!("climon2 launcher currently supports Unix only");

use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

pub fn run() -> std::process::ExitCode {
    let error = handoff();
    eprintln!("climon2: error: {error}");
    std::process::ExitCode::from(125)
}

fn handoff() -> io::Error {
    match companion() {
        Ok(path) => exec(path),
        Err(error) => error,
    }
}

fn companion() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?.canonicalize()?;
    let home = executable
        .parent()
        .ok_or_else(|| invalid("launcher path"))?;
    let current = home.join("current");
    let contents = std::fs::read_to_string(&current)
        .map_err(|error| io::Error::new(error.kind(), format!("cannot read current: {error}")))?;
    let version = parse_current(&contents).ok_or_else(|| invalid("invalid current version"))?;
    Ok(home.join(version).join("climon2-cli"))
}

fn exec(path: PathBuf) -> io::Error {
    let mut args = std::env::args_os();
    let mut command = Command::new(&path);
    if let Some(arg0) = args.next() {
        command.arg0(arg0);
    }
    let error = command.args(args).exec();
    io::Error::new(
        error.kind(),
        format!("cannot execute climon2-cli at {path:?}: {error}"),
    )
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn parse_current(contents: &str) -> Option<&str> {
    let version = contents.strip_suffix('\n').unwrap_or(contents);
    let version = if contents.ends_with('\n') {
        version.strip_suffix('\r').unwrap_or(version)
    } else {
        version
    };
    valid_version(version).then_some(version)
}

fn valid_version(version: &str) -> bool {
    let mut build = version.split('+');
    let release = build.next().unwrap_or_default();
    if !valid_optional_identifiers(build, false) {
        return false;
    }
    let (core, prerelease) = release.split_once('-').unwrap_or((release, ""));
    if release.contains('-') && !valid_identifiers(prerelease, true) {
        return false;
    }
    valid_core(core)
}

fn valid_optional_identifiers(mut parts: std::str::Split<'_, char>, prerelease: bool) -> bool {
    match parts.next() {
        None => true,
        Some(value) => valid_identifiers(value, prerelease) && parts.next().is_none(),
    }
}

fn valid_core(core: &str) -> bool {
    let numbers: Vec<_> = core.split('.').collect();
    numbers.len() == 3 && numbers.into_iter().all(valid_number)
}

fn valid_number(number: &str) -> bool {
    if number.len() > 1 && number.starts_with('0') {
        return false;
    }
    number.parse::<u64>().is_ok() && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_identifiers(value: &str, prerelease: bool) -> bool {
    value
        .split('.')
        .all(|part| valid_identifier(part, prerelease))
}

fn valid_identifier(part: &str, prerelease: bool) -> bool {
    if part.is_empty() || !part.bytes().all(identifier_byte) {
        return false;
    }
    let numeric = part.bytes().all(|byte| byte.is_ascii_digit());
    !(prerelease && numeric && part.len() > 1 && part.starts_with('0'))
}

fn identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-'
}
