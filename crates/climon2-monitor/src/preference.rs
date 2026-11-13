//! The key introduction's **Don't show again** choice, kept per climon2 home
//! in the monitor preference file (spec #73, Introduction preference). It is
//! read once, before the host terminal changes. An unreadable or malformed
//! file means not suppressed, with a diagnostic.
use climon2_core::{
    layout::HomeLayout,
    preferences::{FILE_NAME, MonitorPreferences},
};
use std::{fs, io, path::Path};

/// The choice as this monitor knows it: what the file saves, or not
/// suppressed with the reason the file cannot be used.
pub(crate) struct Preference {
    suppressed: bool,
    problem: Option<String>,
}

impl Preference {
    pub(crate) fn read(home: &HomeLayout) -> Self {
        let (suppressed, problem) = match saved(&home.monitor_preferences()) {
            Ok(suppressed) => (suppressed, None),
            Err(problem) => (false, Some(problem)),
        };
        Self {
            suppressed,
            problem,
        }
    }

    /// Whether **Don't show again** is saved.
    pub(crate) fn suppressed(&self) -> bool {
        self.suppressed
    }

    /// Why the saved choice is unknown, if it is.
    pub(crate) fn problem(&self) -> Option<&str> {
        self.problem.as_deref()
    }
}

/// Whether the file saves **Don't show again**; a missing file saves nothing.
fn saved(path: &Path) -> Result<bool, String> {
    match fs::read(path) {
        Ok(bytes) => MonitorPreferences::parse(&bytes)
            .map(MonitorPreferences::suppresses_key_introduction)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("cannot read {FILE_NAME}: {error}")),
    }
}
