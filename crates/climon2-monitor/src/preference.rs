//! The key introduction's **Don't show again** choice, kept per climon2 home
//! in the monitor preference file (spec #73, Introduction preference). It is
//! read once, before the host terminal changes, and written only when the user
//! changes the checkbox: atomically, as a private temporary file renamed over
//! it. An unreadable or malformed file means not suppressed, with a
//! diagnostic, and is never rewritten implicitly.
use climon2_core::{
    layout::HomeLayout,
    preferences::{FILE_NAME, MonitorPreferences},
};
use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

/// The choice as this monitor knows it: what the file saves, or not
/// suppressed with the reason the file cannot be used.
pub(crate) struct Preference {
    path: PathBuf,
    suppressed: bool,
    problem: Option<String>,
}

impl Preference {
    pub(crate) fn read(home: &HomeLayout) -> Self {
        let path = home.monitor_preferences();
        let (suppressed, problem) = match saved(&path) {
            Ok(suppressed) => (suppressed, None),
            Err(problem) => (false, Some(problem)),
        };
        Self {
            path,
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

    /// Saves the reversed choice. Only a completed save changes it; a failed
    /// one becomes the problem.
    pub(crate) fn reverse(&mut self) {
        let suppressed = !self.suppressed;
        match save(&self.path, MonitorPreferences::new(suppressed)) {
            Ok(()) => {
                tracing::info!(suppressed, "monitor preference saved");
                self.suppressed = suppressed;
                self.problem = None;
            }
            Err(error) => {
                let problem = format!("cannot save {FILE_NAME}: {error}");
                tracing::warn!(%problem, "monitor preference not saved");
                self.problem = Some(problem);
            }
        }
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

/// Writes `preferences` to a private temporary file beside `path`, then
/// renames it over `path`, so the file is never seen half written. A failed
/// save removes the temporary file it may have left.
fn save(path: &Path, preferences: MonitorPreferences) -> io::Result<()> {
    let temporary = temporary(path);
    let saved = write_private(&temporary, &preferences.to_json())
        .and_then(|()| fs::rename(&temporary, path));
    if saved.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    saved
}

/// This process's temporary file for `path`, so two monitors saving in one
/// home never write the same one.
fn temporary(path: &Path) -> PathBuf {
    let mut name = OsString::from(path);
    name.push(format!(".{}.tmp", std::process::id()));
    PathBuf::from(name)
}

/// Created for its owner alone, whatever the umask; a file an interrupted
/// save left at `path` is reused.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
