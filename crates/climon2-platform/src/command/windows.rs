//! How a session's command is found and started on Windows (#322;
//! docs/for-humans/windows/lifecycle.md, "Command resolution and arguments").
//!
//! climon2 resolves the command itself: each `PATH` folder in order, and in
//! each the `PATHEXT` extensions in order. The current directory is never
//! searched, nor is a relative `PATH` entry. A command naming a folder is
//! found from the working directory instead. `CreateProcessW` then gets the
//! resolved path as its application and the command as typed as the first
//! token of the command line, quoted by the C runtime's rules, so the program
//! sees its first argument as typed.
//!
//! A `.bat` or `.cmd` file runs through `cmd.exe /e:ON /v:OFF /d /c` with the
//! batch escaping of Rust std 1.96.0's `make_bat_command_line`. An argument
//! that escaping refuses, one holding CR, LF or NUL, is refused here too,
//! before anything starts.
use crate::pty::{command_line, wide};
use std::{
    ffi::{OsStr, OsString},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
};
use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;

/// What `CreateProcessW` is given for a command: the program it runs and the
/// command line it passes, both NUL-terminated.
pub struct Launch {
    application: Vec<u16>,
    line: Vec<u16>,
}

impl Launch {
    pub(crate) fn application(&self) -> &[u16] {
        &self.application
    }

    /// A copy of the command line, which `CreateProcessW` may change.
    pub(crate) fn line(&self) -> Vec<u16> {
        self.line.clone()
    }
}

/// Why a command cannot be launched.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// No `PATH` folder, or the working directory for a command naming a
    /// folder, holds it.
    NotFound,
    /// It is a batch file, and `cmd.exe` cannot pass one of its arguments
    /// unchanged.
    Unrepresentable,
}

/// How to start `command` with `arguments`, resolved with this process's
/// `PATH` and `PATHEXT` and, for a command naming a folder, from `cwd`.
pub fn launch(command: &OsStr, arguments: &[OsString], cwd: &Path) -> Result<Launch, Refusal> {
    let (path, pathext) = (std::env::var_os("PATH"), std::env::var_os("PATHEXT"));
    let program =
        search(command, cwd, path.as_deref(), pathext.as_deref()).ok_or(Refusal::NotFound)?;
    if is_batch(&program) {
        return batch(&program, arguments);
    }
    Ok(Launch {
        application: wide(program.as_os_str()),
        line: command_line(
            std::iter::once(command).chain(arguments.iter().map(OsString::as_os_str)),
        ),
    })
}

/// The file `command` names: the first candidate that is a file, made
/// absolute as Windows opens it (trailing dots and spaces dropped).
pub fn search(
    command: &OsStr,
    cwd: &Path,
    path: Option<&OsStr>,
    pathext: Option<&OsStr>,
) -> Option<PathBuf> {
    let extensions = extensions(pathext);
    folders(command, cwd, path)
        .iter()
        .flat_map(|folder| candidates(&folder.join(command), &extensions))
        .find(|candidate| candidate.is_file())
        .and_then(|found| std::path::absolute(found).ok())
}

/// `PATHEXT`'s extensions, or `cmd.exe`'s own when it is unset or empty.
fn extensions(pathext: Option<&OsStr>) -> Vec<OsString> {
    let listed: Vec<OsString> = pathext
        .map(|pathext| split(pathext, u16::from(b';')))
        .unwrap_or_default();
    if listed.is_empty() {
        return [".COM", ".EXE", ".BAT", ".CMD"]
            .map(OsString::from)
            .to_vec();
    }
    listed
}

/// The non-empty parts of `text` between `separator`s.
fn split(text: &OsStr, separator: u16) -> Vec<OsString> {
    let units: Vec<u16> = text.encode_wide().collect();
    units
        .split(|&unit| unit == separator)
        .filter(|part| !part.is_empty())
        .map(OsString::from_wide)
        .collect()
}

/// The working directory for a command naming a folder or drive; otherwise
/// each absolute `PATH` folder, unquoted.
fn folders(command: &OsStr, cwd: &Path, path: Option<&OsStr>) -> Vec<PathBuf> {
    let qualified = command
        .encode_wide()
        .any(|unit| matches!(unit, 0x5C | 0x2F | 0x3A));
    if qualified {
        return vec![cwd.to_owned()];
    }
    std::env::split_paths(path.unwrap_or_default())
        .filter(|folder| folder.is_absolute())
        .collect()
}

/// `base` as typed when its name has an extension, then `base` with each
/// extension added.
fn candidates(base: &Path, extensions: &[OsString]) -> Vec<PathBuf> {
    let typed = base.extension().map(|_| base.to_owned());
    typed
        .into_iter()
        .chain(extensions.iter().map(|extension| {
            let mut candidate = base.as_os_str().to_owned();
            candidate.push(extension);
            PathBuf::from(candidate)
        }))
        .collect()
}

fn is_batch(program: &Path) -> bool {
    program.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
    })
}

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;
const SPACE: u16 = b' ' as u16;
const PERCENT: u16 = b'%' as u16;

/// `cmd.exe` from the system folder, given `""<script>" <arguments>"`.
fn batch(script: &Path, arguments: &[OsString]) -> Result<Launch, Refusal> {
    let mut line: Vec<u16> = "cmd.exe /e:ON /v:OFF /d /c \"\"".encode_utf16().collect();
    line.extend(script.as_os_str().encode_wide());
    line.push(QUOTE);
    for argument in arguments {
        line.push(SPACE);
        batch_argument(argument, &mut line)?;
    }
    line.extend([QUOTE, 0u16]);
    let mut application = system_folder();
    application.push("cmd.exe");
    Ok(Launch {
        application: wide(application.as_os_str()),
        line,
    })
}

/// std's `append_bat_arg`: quoted when empty, ending in a backslash, or
/// holding any ASCII character but a letter, digit or one of
/// ``#$*+-./:?@\_``, or a control character. Inside, a quote doubles, the
/// backslashes before it double, and `%` becomes `%%cd:~,%`, which `cmd.exe`
/// expands to `%`, so no variable is expanded.
fn batch_argument(argument: &OsStr, line: &mut Vec<u16>) -> Result<(), Refusal> {
    let units: Vec<u16> = argument.encode_wide().collect();
    if units.iter().any(|&unit| matches!(unit, 0x00 | 0x0A | 0x0D)) {
        return Err(Refusal::Unrepresentable);
    }
    let quoted = needs_quotes(&units);
    if quoted {
        line.push(QUOTE);
    }
    let mut backslashes = 0;
    for unit in units {
        backslashes = batch_unit(unit, backslashes, line);
    }
    if quoted {
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        line.push(QUOTE);
    }
    Ok(())
}

fn needs_quotes(units: &[u16]) -> bool {
    units.last().is_none_or(|&unit| unit == BACKSLASH)
        || char::decode_utf16(units.iter().copied())
            .flatten()
            .any(|character| {
                let plain =
                    character.is_ascii_alphanumeric() || r"#$*+-./:?@\_".contains(character);
                (character.is_ascii() && !plain) || character.is_control()
            })
}

/// Adds `unit` after `backslashes` backslashes; the backslashes now
/// directly before the next unit.
fn batch_unit(unit: u16, backslashes: usize, line: &mut Vec<u16>) -> usize {
    if unit == BACKSLASH {
        line.push(unit);
        return backslashes + 1;
    }
    if unit == QUOTE {
        line.extend(std::iter::repeat_n(BACKSLASH, backslashes));
        line.push(QUOTE);
    } else if unit == PERCENT {
        line.extend("%%cd:~,".encode_utf16());
    }
    line.push(unit);
    0
}

/// The system folder, as std finds `cmd.exe`.
fn system_folder() -> PathBuf {
    let mut buffer = vec![0u16; 260];
    // The buffer holds its length in UTF-16 units; the call writes at most
    // that many, NUL included, and returns how many it wrote without the NUL.
    let length = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
    buffer.truncate(length as usize);
    PathBuf::from(OsString::from_wide(&buffer))
}
