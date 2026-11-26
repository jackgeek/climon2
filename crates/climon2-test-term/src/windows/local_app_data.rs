//! A test's own `LOCALAPPDATA`, short enough for its sessions' socket paths
//! (#422). A socket is `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock`, 54 bytes more
//! than `LOCALAPPDATA`, and its path has 107 bytes, so `LOCALAPPDATA` may
//! have 53. A folder in `TEMP` does not leave that room: the mutation gate's
//! `TEMP` is 38 bytes, and a `tempfile` folder in it with `\local` is 55.
//!
//! The folders go in `%LOCALAPPDATA%\c2t`, the real one, which only its user
//! can use and whose length does not depend on `TEMP`. A run that is killed
//! leaves its folder behind, so each new folder first sweeps those older than
//! any test runs.

use std::{
    fs,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use tempfile::TempDir;
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    },
};

/// The bytes `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock` adds to `LOCALAPPDATA`.
const SOCKET_SUFFIX: usize = 54;
/// `sun_path` holds 108 bytes of UTF-8 with its NUL.
const SOCKET_PATH_MAX: usize = 107;
/// No test runs this long, so a folder this old belongs to a run that ended
/// without removing it.
const STALE: Duration = Duration::from_secs(60 * 60);
/// How long [`end_process`] waits for a process to exit.
const EXIT_WAIT: Duration = Duration::from_secs(10);

/// `%LOCALAPPDATA%\c2t`, where every test's folder goes.
fn parent() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|base| base.is_absolute())
        .unwrap_or_else(|| panic!("LOCALAPPDATA must name an absolute directory for the tests"));
    base.join("c2t")
}

/// A new, empty folder in `%LOCALAPPDATA%\c2t`, removed when dropped, to set
/// as a test's `LOCALAPPDATA` whatever `TEMP` is.
pub fn local_app_data() -> TempDir {
    let parent = parent();
    fs::create_dir_all(&parent)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    sweep(&parent, SystemTime::now());
    let folder = tempfile::Builder::new()
        .prefix("")
        .tempdir_in(&parent)
        .unwrap_or_else(|error| panic!("cannot create a folder in {}: {error}", parent.display()));
    let length = folder.path().as_os_str().len();
    assert!(
        length + SOCKET_SUFFIX <= SOCKET_PATH_MAX,
        "{} is {length} bytes, too long for a {SOCKET_PATH_MAX}-byte socket path",
        folder.path().display()
    );
    folder
}

/// Removes what `parent` holds that was made [`STALE`] or longer before
/// `now`. Anything still in use cannot be removed and stays.
fn sweep(parent: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(made) = metadata.created().or_else(|_| metadata.modified()) else {
            continue;
        };
        if now.duration_since(made).is_ok_and(|age| age >= STALE) {
            let path = entry.path();
            let _ = if metadata.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
        }
    }
}

/// Ends process `pid` and waits up to ten seconds for it to exit, so that
/// the folders it used can be removed. A process already gone is no error.
pub fn end_process(pid: u32) {
    // Opening by PID has no preconditions.
    let raw = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
    if raw.is_null() {
        return;
    }
    // A new handle nothing else owns, closed here.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    // The handle is open with PROCESS_TERMINATE and SYNCHRONIZE; ending an
    // ended process only fails.
    unsafe { TerminateProcess(process.as_raw_handle(), 1) };
    let milliseconds = u32::try_from(EXIT_WAIT.as_millis()).unwrap_or(u32::MAX);
    let waited = unsafe { WaitForSingleObject(process.as_raw_handle(), milliseconds) };
    if waited != WAIT_OBJECT_0 {
        eprintln!("process {pid} did not exit within {EXIT_WAIT:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever `TEMP` is, the folder is in `%LOCALAPPDATA%\c2t`, leaves a
    /// socket path within budget, and is gone once dropped.
    #[test]
    fn the_folder_is_in_c2t_short_enough_and_removed() {
        let folder = local_app_data();
        let path = folder.path().to_owned();

        assert_eq!(path.parent(), Some(parent().as_path()));
        assert!(path.as_os_str().len() + SOCKET_SUFFIX <= SOCKET_PATH_MAX);
        assert!(path.is_dir());
        drop(folder);
        assert!(!path.exists());
    }

    /// A sweep removes only what is at least an hour old.
    #[test]
    fn a_sweep_removes_only_stale_entries() {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("folder");
        fs::create_dir_all(folder.join("inner")).unwrap();
        let file = parent.path().join("file");
        fs::write(&file, "").unwrap();

        sweep(parent.path(), SystemTime::now());
        assert!(folder.is_dir() && file.is_file());

        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!folder.exists() && !file.exists());
    }

    #[test]
    fn ending_a_process_waits_for_its_exit() {
        let mut child = std::process::Command::new("cmd")
            .args(["/d", "/c", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();

        end_process(child.id());

        assert!(child.try_wait().unwrap().is_some(), "it still runs");
    }
}
