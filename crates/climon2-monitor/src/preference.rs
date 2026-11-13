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
use climon2_platform::process;
use std::{
    ffi::{OsStr, OsString},
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
/// renames it over `path`, so the file is never seen half written. What
/// interrupted saves left there is removed first, and a failed save removes
/// what it wrote.
fn save(path: &Path, preferences: MonitorPreferences) -> io::Result<()> {
    remove_leftovers(path);
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

/// Removes the temporary files that saves interrupted before their rename
/// left beside `path`: those of processes no longer running, and this
/// process's own, which only an earlier process with its pid can have left.
/// Those of running processes may still be saving, so they stay.
fn remove_leftovers(path: &Path) {
    let Some(entries) = path.parent().and_then(|home| fs::read_dir(home).ok()) else {
        return;
    };
    for entry in entries.flatten() {
        let leftover = temporary_pid(&entry.file_name())
            .is_some_and(|pid| pid == std::process::id() || ended(pid));
        if leftover {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The pid in `name`, if it names a temporary file of the preference file.
fn temporary_pid(name: &OsStr) -> Option<u32> {
    name.to_str()?
        .strip_prefix(FILE_NAME)?
        .strip_prefix('.')?
        .strip_suffix(".tmp")?
        .parse()
        .ok()
}

/// Whether no process runs as `pid` any more. A check that fails, as one of
/// another user's process may, is no evidence that it ended.
fn ended(pid: u32) -> bool {
    matches!(process::start_time(pid), Ok(None))
}

/// A new file, readable and writable by its owner alone whatever the umask;
/// an existing file or link at `path` is refused.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}
