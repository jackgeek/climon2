//! How a climon2 executable fails: the exit status its thin main returns and
//! the one-line diagnostic it prints, naming the logs that recorded the failure.
use crate::exit::CLIMON2_ERROR;
use std::{
    fmt, io,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct Failure {
    pub code: u32,
    pub message: String,
    pub logs: Option<PathBuf>,
}

impl Failure {
    pub fn new(code: u32, message: impl fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
            logs: None,
        }
    }

    /// Names the logs that recorded the failure; the first logs named keep it.
    pub fn logged(mut self, path: &Path) -> Self {
        if self.logs.is_none() {
            self.logs = Some(path.to_owned());
        }
        self
    }
}

/// An I/O failure ends with the climon2 error code.
impl From<io::Error> for Failure {
    fn from(error: io::Error) -> Self {
        Self::new(CLIMON2_ERROR, error)
    }
}

impl fmt::Display for Failure {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}", one_line(&self.message))?;
        if let Some(logs) = &self.logs {
            write!(out, " (logs: {})", one_line(&logs.to_string_lossy()))?;
        }
        Ok(())
    }
}

impl std::error::Error for Failure {}

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
