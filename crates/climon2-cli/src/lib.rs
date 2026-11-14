//! Interactive startup orchestration; binaries communicate only through contracts.

mod attach;
mod exec;
mod kill;
pub mod list;
mod monitor;

use climon2_core::{
    cli::{CliArgs, CommandArgs},
    layout::HomeLayout,
    lifetime::Lifetime,
    selector::Selector,
};
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

/// Arguments exclude argv[0]. Success is a final status, except successful
/// ordinary exec or attach, which replace this process with the metadata
/// version's client, and monitor, which replaces it with this version's monitor.
pub fn run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error> {
    let arguments =
        CliArgs::parse(arguments).map_err(|error| Error::new(error.exit_code(), error))?;
    match work(arguments) {
        Ok(work) => logged_run(work),
        Err(text) => {
            println!("{text}");
            Ok(0)
        }
    }
}

/// The subcommands that need a home; help and version are answered here.
enum Work {
    Exec(Lifetime, CommandArgs),
    List { json: bool },
    Handoff(Handoff),
}

/// Subcommands that another executable of a version folder carries out: the
/// client attaches or kills, and the monitor monitors.
enum Handoff {
    Attach(Selector),
    Kill(Selector),
    Monitor,
}

fn work(arguments: CliArgs) -> Result<Work, String> {
    match arguments {
        CliArgs::Exec { lifetime, command } => Ok(Work::Exec(lifetime, command)),
        CliArgs::List { json } => Ok(Work::List { json }),
        other => handoff(other).map(Work::Handoff),
    }
}

fn handoff(arguments: CliArgs) -> Result<Handoff, String> {
    match arguments {
        CliArgs::Attach(selector) => Ok(Handoff::Attach(selector)),
        CliArgs::Kill(selector) => Ok(Handoff::Kill(selector)),
        CliArgs::Monitor => Ok(Handoff::Monitor),
        other => Err(presentation(&other)),
    }
}

fn presentation(arguments: &CliArgs) -> String {
    match arguments {
        CliArgs::Version => format!("climon2 {}", env!("CARGO_PKG_VERSION")),
        _ => climon2_core::cli::USAGE.to_owned(),
    }
}

fn logged_run(work: Work) -> Result<u32, Error> {
    let home = home()?;
    let general = LogFile::general(&home)?;
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = general
        .dispatch(filter.as_deref())
        .map_err(|error| Error::from(error).logged(general.path()))?;
    tracing::dispatcher::with_default(&dispatch, || {
        logging::prune(&home, SystemTime::now());
        command(work, &home, &general, filter.as_deref())
            .map_err(|error| error.logged(general.path()))
    })
}

fn command(work: Work, home: &Path, general: &LogFile, filter: Option<&str>) -> Result<u32, Error> {
    match work {
        Work::Exec(lifetime, command) => exec::run(home, lifetime, command, general, filter),
        Work::List { json } => list::run(home, json).map(|()| 0).map_err(Error::from),
        Work::Handoff(handoff) => handoff.run(home),
    }
}

impl Handoff {
    fn run(self, home: &Path) -> Result<u32, Error> {
        match self {
            Self::Attach(selector) => attach::run(home, selector),
            Self::Kill(selector) => kill::run(home, &selector),
            Self::Monitor => monitor::run(),
        }
    }
}
