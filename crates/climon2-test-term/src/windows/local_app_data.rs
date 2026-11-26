//! A test's own `LOCALAPPDATA`, short enough for its sessions' socket paths
//! (#422). A socket is `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock`, 54 bytes more
//! than `LOCALAPPDATA`, and its path has 107 bytes, so `LOCALAPPDATA` may
//! have 53. A folder in `TEMP` does not leave that room: the mutation gate's
//! `TEMP` is 38 bytes, and a `tempfile` folder in it with `\local` is 55.
//!
//! The folders go in `%LOCALAPPDATA%\c2t`, the real one, which only its user
//! can use and whose length does not depend on `TEMP`; `CLIMON2_TEST_SHORT_ROOT`
//! names another parent where that is too long. Each folder holds a lock file
//! open while it is in use. A run that is killed leaves its folder behind, so
//! each new folder first sweeps those older than any test runs whose lock no
//! one holds.

use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
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

/// Names the folder to make test folders in, instead of `%LOCALAPPDATA%\c2t`.
pub const SHORT_ROOT: &str = "CLIMON2_TEST_SHORT_ROOT";
/// The longest folder this makes: a socket path, at most 107 bytes, adds 54
/// to `LOCALAPPDATA`, and the socket-length test sets `LOCALAPPDATA` to the
/// folder plus at least `\r`.
const FOLDER_MAX: usize = 107 - 54 - 2;
/// No test runs this long, so a folder this old belongs to a run that ended
/// without removing it.
const STALE: Duration = Duration::from_secs(60 * 60);
/// The file each folder holds open while it is in use. Windows refuses to
/// delete a file that is open without delete sharing.
const LOCK: &str = ".lock";
/// How long [`end_process`] waits for a process to exit.
const EXIT_WAIT: Duration = Duration::from_secs(10);

/// A test's `LOCALAPPDATA`: a new, empty folder but for its lock file, removed
/// when dropped.
pub struct LocalAppData {
    // Declared first, so it is closed before the folder is removed.
    _lock: File,
    folder: TempDir,
}

impl LocalAppData {
    pub fn path(&self) -> &Path {
        self.folder.path()
    }
}

/// [`SHORT_ROOT`] when set, else `%LOCALAPPDATA%\c2t`.
fn parent() -> PathBuf {
    if let Some(root) = std::env::var_os(SHORT_ROOT) {
        let root = PathBuf::from(root);
        assert!(
            root.is_absolute(),
            "{SHORT_ROOT} must name an absolute folder"
        );
        return root;
    }
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|base| base.is_absolute())
        .unwrap_or_else(|| {
            panic!("LOCALAPPDATA must name an absolute folder, or set {SHORT_ROOT}")
        });
    base.join("c2t")
}

/// A new folder in [`parent`], to set as a test's `LOCALAPPDATA` whatever
/// `TEMP` is.
pub fn local_app_data() -> LocalAppData {
    let parent = parent();
    fs::create_dir_all(&parent)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    sweep(&parent, SystemTime::now());
    let folder = tempfile::Builder::new()
        .prefix("")
        .tempdir_in(&parent)
        .unwrap_or_else(|error| panic!("cannot create a folder in {}: {error}", parent.display()));
    if let Some(refusal) = too_long(folder.path()) {
        panic!("{refusal}");
    }
    let lock = hold(&folder.path().join(LOCK)).unwrap_or_else(|error| {
        panic!(
            "cannot create a lock in {}: {error}",
            folder.path().display()
        )
    });
    LocalAppData {
        _lock: lock,
        folder,
    }
}

/// Why `folder` cannot be a test's `LOCALAPPDATA`, if it is too long.
fn too_long(folder: &Path) -> Option<String> {
    let length = folder.as_os_str().len();
    (length > FOLDER_MAX).then(|| {
        format!(
            "{} is {length} bytes, over the {FOLDER_MAX} a 107-byte socket path leaves; \
             set {SHORT_ROOT} to a shorter folder",
            folder.display()
        )
    })
}

/// Creates the lock file at `path` and holds it open with no sharing, so no
/// one else can open or delete it until it is closed.
fn hold(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .share_mode(0)
        .open(path)
}

/// Removes what `parent` holds that was made [`STALE`] or longer before
/// `now` and whose lock no one holds. Anything else in use cannot be removed
/// and stays.
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
        if !now.duration_since(made).is_ok_and(|age| age >= STALE) {
            continue;
        }
        let path = entry.path();
        let _ = if metadata.is_dir() {
            match fs::remove_file(path.join(LOCK)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => continue,
                _ => fs::remove_dir_all(&path),
            }
        } else {
            fs::remove_file(&path)
        };
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

    /// Whatever `TEMP` is, the folder is in the parent, leaves a socket path
    /// within budget, and is gone once dropped.
    #[test]
    fn the_folder_is_in_the_parent_short_enough_and_removed() {
        let folder = local_app_data();
        let path = folder.path().to_owned();

        assert_eq!(path.parent(), Some(parent().as_path()));
        assert!(path.as_os_str().len() <= FOLDER_MAX);
        assert!(path.join(LOCK).is_file());
        drop(folder);
        assert!(!path.exists());
    }

    #[test]
    fn a_folder_too_long_is_refused_naming_its_length_the_limit_and_the_override() {
        let folder = PathBuf::from(format!("C:\\{}", "x".repeat(FOLDER_MAX - 3)));
        assert_eq!(too_long(&folder), None);

        let longer = folder.with_file_name("x".repeat(FOLDER_MAX - 2));
        let refusal = too_long(&longer).unwrap();

        assert!(
            refusal.contains(&format!("is {} bytes", FOLDER_MAX + 1)),
            "{refusal}"
        );
        assert!(refusal.contains("over the 51"), "{refusal}");
        assert!(refusal.contains(SHORT_ROOT), "{refusal}");
    }

    /// A sweep removes only what is at least an hour old and not locked.
    #[test]
    fn a_sweep_removes_only_stale_unlocked_entries() {
        let parent = tempfile::tempdir().unwrap();
        let unlocked = parent.path().join("unlocked");
        fs::create_dir_all(unlocked.join("inner")).unwrap();
        File::create(unlocked.join(LOCK)).unwrap();
        let locked = parent.path().join("locked");
        fs::create_dir_all(locked.join("inner")).unwrap();
        let held = hold(&locked.join(LOCK)).unwrap();
        let file = parent.path().join("file");
        fs::write(&file, "").unwrap();

        sweep(parent.path(), SystemTime::now());
        assert!(unlocked.is_dir() && locked.is_dir() && file.is_file());

        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!unlocked.exists() && !file.exists());
        assert!(locked.join("inner").is_dir() && locked.join(LOCK).is_file());

        drop(held);
        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!locked.exists());
    }

    #[test]
    fn ending_a_process_waits_for_its_exit() {
        let mut child = std::process::Command::new("ping")
            .args(["-n", "30", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();

        end_process(child.id());

        assert!(child.try_wait().unwrap().is_some(), "it still runs");
    }
}
