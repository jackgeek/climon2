//! Everything that can refuse the monitor, checked in order before the host
//! terminal changes: terminals, the climon2 home and executable home, then the
//! one configuration snapshot, whose limits must hold one session's terminal
//! model.
use crate::Error;
use climon2_core::{
    VersionName,
    config::{ConfigError, ConfigSource, MandatoryAllocation, MonitorConfig},
    layout::{ExecutableLayout, HomeLayout, VersionBinary},
    protocol::TerminalSize,
};
use climon2_platform::{logging::LogFile, runtime, terminal};
use climon2_vt::{Geometry, Model};
use std::{
    io,
    path::{Path, PathBuf},
};

/// A validated start: nothing has touched the host terminal yet.
pub(crate) struct Startup {
    pub(crate) log: LogFile,
    pub(crate) version: VersionName,
    pub(crate) config: MonitorConfig,
}

impl Startup {
    pub(crate) fn read() -> Result<Self, Error> {
        terminal::require_ttys().map_err(|_| Error::new("climon2 monitor needs a terminal"))?;
        let home = home().map_err(Error::new)?;
        let log = LogFile::monitor(&home).map_err(Error::new)?;
        Self::configured(&home, log)
    }

    fn configured(home: &Path, log: LogFile) -> Result<Self, Error> {
        let logged = |error: io::Error| Error::new(error).logged(log.path());
        let version = installed_version().map_err(logged)?;
        let config = configuration(home).map_err(logged)?;
        Ok(Self {
            log,
            version,
            config,
        })
    }
}

/// The climon2 home, resolved, created and canonicalized exactly as the CLI does.
fn home() -> io::Result<PathBuf> {
    let user_home = std::env::var_os("HOME").unwrap_or_default();
    let override_value = std::env::var_os("CLIMON2_HOME");
    let layout = HomeLayout::resolve(override_value.as_deref(), Path::new(&user_home))
        .map_err(io::Error::other)?;
    runtime::canonical_home(layout.root())
}

/// The version folder this monitor runs from, found exactly as the CLI finds
/// its own executable home.
fn installed_version() -> io::Result<VersionName> {
    let executable = std::env::current_exe()?.canonicalize()?;
    ExecutableLayout::from_binary_path(&executable, VersionBinary::Monitor)
        .map(|layout| layout.version().clone())
        .map_err(|error| {
            io::Error::other(format!("cannot find the climon2 executable home: {error}"))
        })
}

/// The one configuration snapshot: the client bindings, whose escape prefix
/// the monitor needs to reach its actions, and the `monitor` section, whose
/// limits must hold one session's terminal model. Nothing reads the file
/// again.
fn configuration(home: &Path) -> io::Result<MonitorConfig> {
    let layout =
        HomeLayout::resolve(Some(home.as_os_str()), Path::new("/")).map_err(io::Error::other)?;
    ConfigSource::read(&layout)
        .and_then(|source| source.monitor())
        .and_then(admissible)
        .map_err(io::Error::other)
}

/// The snapshot, if its limits hold the mandatory allocation of one session
/// at the largest geometry a session can have.
fn admissible(config: MonitorConfig) -> Result<MonitorConfig, ConfigError> {
    let limits = config.settings().limits;
    limits
        .hold(largest_session(limits.history_rows()))
        .map(|()| config)
}

/// What `climon2-vt` charges a blank model keeping `history_rows` rows at the
/// largest geometry, the one the largest host clamps to: the model alone,
/// and the first history chunk, which holds the recent page.
fn largest_session(history_rows: u32) -> MandatoryAllocation {
    let at = Geometry::for_host(u16::MAX, u16::MAX).expect("a nonzero size");
    let current_model = Model::mandatory(at, 0);
    let with_page = Model::mandatory(at, history_rows as usize);
    MandatoryAllocation {
        size: TerminalSize {
            cols: at.cols(),
            rows: at.rows(),
        },
        current_model: current_model as u64,
        recent_page: (with_page - current_model) as u64,
    }
}
