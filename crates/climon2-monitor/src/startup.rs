//! Everything that can refuse the monitor, checked in order before the host
//! terminal changes: terminals, the climon2 home and executable home, then the
//! one configuration snapshot, whose limits must hold one session's terminal
//! model. The key introduction's preference is read last; it never refuses
//! the monitor.
use crate::{Error, preference::Preference};
use climon2_core::{
    VersionName,
    config::{ConfigError, ConfigSource, MandatoryAllocation, MonitorConfig},
    exit::CLIMON2_ERROR,
    layout::{ExecutableHome, ExecutableLayout, HomeLayout, VersionBinary},
    protocol::TerminalSize,
};
use climon2_platform::{home, logging::LogFile, terminal};
use climon2_vt::{Geometry, Mandatory, Model, Parser};
use std::{io, path::Path};

/// A validated start: nothing has touched the host terminal yet.
pub(crate) struct Startup {
    pub(crate) log: LogFile,
    pub(crate) version: VersionName,
    /// The canonical climon2 home, whose sessions the monitor discovers.
    pub(crate) home: HomeLayout,
    /// Where each session's exact-release client library is loaded from.
    pub(crate) executable: ExecutableHome,
    pub(crate) config: MonitorConfig,
    pub(crate) preference: Preference,
}

impl Startup {
    pub(crate) fn read() -> Result<Self, Error> {
        terminal::require_ttys()
            .map_err(|_| Error::new(CLIMON2_ERROR, "c2 monitor needs a terminal"))?;
        let home = home::canonical()?;
        let log = LogFile::monitor(&home)?;
        Self::configured(&home, log)
    }

    fn configured(home: &Path, log: LogFile) -> Result<Self, Error> {
        let logged = |error: io::Error| Error::from(error).logged(log.path());
        let installed = installed().map_err(logged)?;
        let home = HomeLayout::at(home)
            .map_err(io::Error::other)
            .map_err(logged)?;
        let config = configuration(&home).map_err(logged)?;
        Ok(Self {
            log,
            version: installed.version().clone(),
            preference: Preference::read(&home),
            home,
            executable: installed.home(),
            config,
        })
    }
}

/// The version folder this monitor runs from, found exactly as the CLI finds
/// its own executable home.
fn installed() -> io::Result<ExecutableLayout> {
    home::executable_layout(VersionBinary::Monitor)?.map_err(|error| {
        io::Error::other(format!("cannot find the climon2 executable home: {error}"))
    })
}

/// The one configuration snapshot: the client bindings, whose escape prefix
/// the monitor needs to reach its actions, and the `monitor` section, whose
/// limits must hold one session's terminal model. Nothing reads the file
/// again.
fn configuration(home: &HomeLayout) -> io::Result<MonitorConfig> {
    ConfigSource::read(home)
        .and_then(|source| source.monitor())
        .and_then(admissible)
        .map_err(io::Error::other)
}

/// The snapshot, if its limits hold the mandatory allocation of one session
/// at the largest geometry a session can have.
fn admissible(config: MonitorConfig) -> Result<MonitorConfig, ConfigError> {
    let limits = config.settings().limits;
    limits
        .hold(largest_mandatory_allocation(limits.history_rows()))
        .map(|()| config)
}

/// The mandatory allocation `climon2-vt` charges a model keeping
/// `history_rows` rows at the largest session geometry: the current model,
/// and the first history chunk, which holds the recent page and is admitted
/// at the first scroll; beside them the monitor's one shared parser.
fn largest_mandatory_allocation(history_rows: u32) -> MandatoryAllocation {
    let largest = Geometry::LARGEST;
    let Mandatory {
        current_model,
        recent_page,
    } = Model::mandatory(largest, history_rows as usize);
    MandatoryAllocation {
        size: TerminalSize {
            cols: largest.cols(),
            rows: largest.rows(),
        },
        parser: Parser::BYTES as u64,
        current_model: current_model as u64,
        recent_page: recent_page as u64,
    }
}
