//! Everything that can refuse the monitor, checked in order before the host
//! terminal changes: terminals, the climon2 home and executable home, then the
//! one configuration snapshot. The key introduction's preference is read last;
//! it never refuses the monitor.
use crate::{Error, preference::Preference};
use climon2_core::{
    VersionName,
    config::{ConfigSource, MonitorConfig},
    layout::{ExecutableHome, ExecutableLayout, HomeLayout, VersionBinary},
};
use climon2_platform::{logging::LogFile, runtime, terminal};
use std::{
    io,
    path::{Path, PathBuf},
};

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
        terminal::require_ttys().map_err(|_| Error::new("climon2 monitor needs a terminal"))?;
        let home = home().map_err(Error::new)?;
        let log = LogFile::monitor(&home).map_err(Error::new)?;
        Self::configured(&home, log)
    }

    fn configured(home: &Path, log: LogFile) -> Result<Self, Error> {
        let logged = |error: io::Error| Error::new(error).logged(log.path());
        let installed = installed().map_err(logged)?;
        let home = HomeLayout::resolve(Some(home.as_os_str()), Path::new("/"))
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
fn installed() -> io::Result<ExecutableLayout> {
    let executable = std::env::current_exe()?.canonicalize()?;
    ExecutableLayout::from_binary_path(&executable, VersionBinary::Monitor).map_err(|error| {
        io::Error::other(format!("cannot find the climon2 executable home: {error}"))
    })
}

/// The one configuration snapshot: the client bindings, whose escape prefix
/// the monitor needs to reach its actions, and the `monitor` section. Nothing
/// reads the file again.
fn configuration(home: &HomeLayout) -> io::Result<MonitorConfig> {
    ConfigSource::read(home)
        .and_then(|source| source.monitor())
        .map_err(io::Error::other)
}
