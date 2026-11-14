//! The interactive monitor (spec #73): one terminal that tracks every live
//! session. This walking skeleton validates its start before touching the
//! host terminal, then owns the host with one permanent top row above an
//! explicit empty state, under the key introduction when it is due, until
//! prefix+x or a catchable termination signal. Nothing is discovered yet.

#[cfg(feature = "test-hooks")]
mod hooks;
mod host;
mod introduction;
mod keys;
mod preference;
mod screen;
mod startup;

use climon2_core::{config::MonitorConfig, protocol::TerminalSize};
use host::{Ending, Event, Host};
use introduction::KeyIntroduction;
use keys::{Action, Key, Keys};
use preference::Preference;
use screen::Screen;
use startup::Startup;
use std::{
    fmt, io,
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
/// orderly exit: 0 for prefix+x, 128 + signal for a termination signal.
pub fn run() -> Result<u32, Error> {
    let startup = Startup::read()?;
    let log = startup.log.path().to_owned();
    let filter = std::env::var("CLIMON2_LOG").ok();
    let dispatch = startup
        .log
        .dispatch(filter.as_deref())
        .map_err(|error| Error::new(error).logged(&log))?;
    tracing::dispatcher::with_default(&dispatch, || monitor(startup))
        .map_err(|error| Error::new(error).logged(&log))
}

/// Records how the monitor ended: the host terminal may be gone, so a failure
/// is logged as well as returned for main to print.
fn monitor(startup: Startup) -> io::Result<u32> {
    tracing::info!(version = %startup.version, "monitor started");
    tracing::info!(effective = %startup.config, "monitor configuration");
    if let Some(problem) = startup.preference.problem() {
        tracing::warn!(%problem, "monitor preference unusable");
    }
    let status = owned(&startup.config, startup.preference)
        .inspect_err(|error| tracing::error!(%error, "monitor failed"))?;
    tracing::info!(status, "monitor stopped");
    Ok(status)
}

/// Owns the host until an ending, and releases it before any diagnostic.
fn owned(config: &MonitorConfig, preference: Preference) -> io::Result<u32> {
    let (mut host, size) = Host::enter()?;
    let served =
        Monitor::start(size, config, preference).and_then(|monitor| monitor.serve(&mut host));
    let released = host.release();
    served.and_then(|ending| released.map(|()| ending.status()))
}

/// The empty monitor: what it draws and the keys that act on it.
struct Monitor {
    screen: Screen,
    keys: Keys,
    introduction: KeyIntroduction,
}

impl Monitor {
    fn start(
        size: TerminalSize,
        config: &MonitorConfig,
        preference: Preference,
    ) -> io::Result<Self> {
        let prefix = config.escape_prefix();
        let hints = config.settings().show_key_hints;
        let introduction = KeyIntroduction::new(prefix, preference, hints);
        Ok(Self {
            screen: Screen::new(size, prefix, hints, &introduction)?,
            keys: Keys::new(prefix),
            introduction,
        })
    }

    /// Acts on host events until an ending. Consuming the monitor drops its
    /// screen before the host is released.
    fn serve(mut self, host: &mut Host) -> io::Result<Ending> {
        loop {
            let event = host.next()?;
            if let Some(ending) = self.handle(event)? {
                return Ok(ending);
            }
        }
    }

    fn handle(&mut self, event: Event<'_>) -> io::Result<Option<Ending>> {
        match event {
            Event::End(ending) => Ok(Some(ending)),
            Event::Resize(size) => self.screen.resize(size, &self.introduction).map(|()| None),
            Event::Input(bytes) => self.input(bytes),
            Event::Idle => Ok(None),
        }
    }

    /// Acts on each byte in turn until one ends the monitor, then shows the
    /// key introduction as they leave it.
    fn input(&mut self, bytes: &[u8]) -> io::Result<Option<Ending>> {
        match bytes.iter().find_map(|&byte| self.act(byte)) {
            Some(ending) => Ok(Some(ending)),
            None => self.screen.show(&self.introduction).map(|()| None),
        }
    }

    /// Acts on the key `byte` completes: prefix+x ends the monitor, prefix+?
    /// opens the key introduction, and a typed key goes to it.
    fn act(&mut self, byte: u8) -> Option<Ending> {
        match self.keys.key(byte) {
            Some(Key::Action(Action::Exit)) => return Some(Ending::Exit),
            Some(Key::Action(Action::KeyIntroduction)) => self.introduction.open(),
            Some(Key::Typed(byte)) => self.introduction.key(byte),
            Some(Key::Action(Action::SessionList)) | None => {}
        }
        None
    }
}
