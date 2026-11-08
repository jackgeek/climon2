//! The interactive monitor (spec #73): one terminal that tracks every live
//! session. This walking skeleton validates its start before touching the
//! host terminal; nothing is discovered yet.

mod startup;

use std::{
    fmt,
    path::{Path, PathBuf},
};

/// A one-line diagnostic that the thin main prints, after the host is
/// restored, with the climon2 error code.
#[derive(Debug)]
pub struct Error {
    message: String,
    logs: Option<PathBuf>,
}

impl Error {
    fn new(message: impl fmt::Display) -> Self {
        Self {
            message: message.to_string(),
            logs: None,
        }
    }

    fn logged(mut self, path: &Path) -> Self {
        self.logs = Some(path.to_owned());
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&one_line(&self.message))?;
        if let Some(logs) = &self.logs {
            write!(out, " (logs: {})", one_line(&logs.to_string_lossy()))?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

fn one_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

/// The CLI execs the monitor with no arguments. Returns the exit status of an
/// orderly exit.
pub fn run() -> Result<u32, Error> {
    let startup = startup::Startup::read()?;
    let log = startup.log.path().to_owned();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = startup
        .log
        .dispatch(filter.as_deref())
        .map_err(|error| Error::new(error).logged(&log))?;
    tracing::dispatcher::with_default(&dispatch, || {
        tracing::info!(version = %startup.version, prefix = %startup.prefix, "monitor started");
    });
    Ok(0)
}
