//! Interactive startup orchestration; binaries communicate only through contracts.

mod exec;
pub mod list;

use climon2_core::{cli::CliArgs, layout::HomeLayout};
use climon2_platform::{
    logging::{self, LogFile},
    runtime,
};
use std::{
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Debug)]
pub struct Error {
    pub code: u32,
    pub message: String,
    pub logs: Option<PathBuf>,
}

impl Error {
    fn new(code: u32, message: impl fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
            logs: None,
        }
    }

    fn logged(mut self, path: &Path) -> Self {
        if self.logs.is_none() {
            self.logs = Some(path.to_owned());
        }
        self
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::new(125, error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(out, "{}", one_line(&self.message))?;
        if let Some(logs) = &self.logs {
            write!(out, " (logs: {})", one_line(&logs.to_string_lossy()))?;
        }
        Ok(())
    }
}

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

impl std::error::Error for Error {}

fn home() -> io::Result<PathBuf> {
    let user_home = std::env::var_os("HOME").unwrap_or_default();
    let override_value = std::env::var_os("CLIMON2_HOME");
    let layout = HomeLayout::resolve(override_value.as_deref(), Path::new(&user_home))
        .map_err(io::Error::other)?;
    runtime::canonical_home(layout.root())
}

/// Arguments exclude argv[0]. Success is a final status, except successful exec,
/// which replaces this process with the metadata version's client.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        CliArgs::parse(arguments).map_err(|error| Error::new(error.exit_code(), error))?;
    if presentation(&arguments) {
        return Ok(0);
    }
    logged_run(arguments)
}

fn presentation(arguments: &CliArgs) -> bool {
    match arguments {
        CliArgs::Help => {
            println!("usage: climon2 exec <command> [args...] | list [--json]");
            true
        }
        CliArgs::Version => {
            println!("climon2 {}", env!("CARGO_PKG_VERSION"));
            true
        }
        _ => false,
    }
}

fn logged_run(arguments: CliArgs) -> Result<u32, Error> {
    let home = home()?;
    let general = LogFile::general(&home)?;
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = general
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(general.path()))?;
    tracing::dispatcher::with_default(&dispatch, || {
        logging::prune(&home, SystemTime::now());
        command(arguments, &home, &general, filter.as_deref())
            .map_err(|error| error.logged(general.path()))
    })
}

fn command(
    arguments: CliArgs,
    home: &Path,
    general: &LogFile,
    filter: Option<&str>,
) -> Result<u32, Error> {
    match arguments {
        CliArgs::Exec(command) => exec::run(home, command, general, filter),
        CliArgs::List { json } => list::run(home, json).map(|()| 0).map_err(Error::from),
        _ => unreachable!("help/version returned before home resolution"),
    }
}
