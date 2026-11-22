//! Terminals on Windows, so far only what `exec --headless` and its detached
//! daemon need (#317). The console rules for `exec` and `attach`
//! (docs/windows-lifecycle.md, "Client console") arrive with #321.
use climon2_core::protocol::TerminalSize;
use std::{fs::File, io};

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "interactive terminals are not supported on Windows yet",
    )
}

/// Only headless `exec` runs on Windows so far, and it needs no terminal.
pub fn require_ttys() -> io::Result<()> {
    Err(unsupported())
}

/// No console is read yet, so a session starts at the fallback size.
pub fn input() -> io::Result<File> {
    Err(unsupported())
}

pub fn size(_terminal: &File) -> io::Result<TerminalSize> {
    Err(unsupported())
}

/// The daemon is detached: it has no console, so no console control event
/// reaches it and there is nothing to register.
pub struct Signals;

impl Signals {
    pub fn daemon() -> io::Result<Self> {
        Ok(Self)
    }

    /// The client's kill mode: it handles no console control event yet
    /// (#321), so none ends it early.
    pub fn client() -> io::Result<Self> {
        Ok(Self)
    }

    /// Never set on Windows, where no signal ends a daemon.
    pub fn termination_signal(&self) -> Option<u32> {
        None
    }
}
