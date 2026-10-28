use crate::{Payload, install};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Run the installer command with arguments excluding argv[0].
pub fn run(args: impl IntoIterator<Item = OsString>) -> io::Result<()> {
    let destination = destination(args)?;
    let payload = Payload::beside_installer()?;
    let report = install(&payload, &destination)?;
    if !on_path(&report.executable_home) {
        std::io::stdout().write_all(&path_hint(&report.executable_home))?;
    }
    Ok(())
}

fn destination(args: impl IntoIterator<Item = OsString>) -> io::Result<PathBuf> {
    let mut args = args.into_iter();
    let Some(option) = args.next() else {
        return crate::platform::default_directory();
    };
    if option != "--dir" {
        return Err(usage());
    }
    let path = args
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }
    Ok(PathBuf::from(path))
}

fn usage() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "usage: install [--dir <directory>]",
    )
}

fn on_path(home: &Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|entry| same_directory(&entry, home))
}

fn same_directory(entry: &Path, home: &Path) -> bool {
    let entry = if entry.as_os_str().is_empty() {
        Path::new(".")
    } else {
        entry
    };
    entry.canonicalize().is_ok_and(|path| path == home)
}

fn path_hint(home: &Path) -> Vec<u8> {
    let mut hint = b"export PATH='".to_vec();
    for byte in home.as_os_str().as_encoded_bytes() {
        if *byte == b'\'' {
            hint.extend_from_slice(b"'\\''");
        } else {
            hint.push(*byte);
        }
    }
    hint.extend_from_slice(b"':\"$PATH\"\n");
    hint
}
