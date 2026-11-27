//! A test's own `LOCALAPPDATA`, short enough for its sessions' socket paths
//! (#422). A socket is `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock`, 54 bytes more
//! than `LOCALAPPDATA`, and its path has 107 bytes, so `LOCALAPPDATA` may
//! have 53. A folder in `TEMP` does not leave that room: the mutation gate's
//! `TEMP` is 38 bytes, and a `tempfile` folder in it with `\local` is 55.
//!
//! The folders go in `%LOCALAPPDATA%\c2t`, the real one, which only its user
//! can use and whose length does not depend on `TEMP`; where that is too long,
//! `CLIMON2_TEST_SHORT_ROOT` names a folder to use in place of
//! `%LOCALAPPDATA%`, and they go in its `c2t`. Each folder holds a lock file
//! open while it is in use. A run that is killed leaves its folder behind, so
//! each new folder first sweeps, from `c2t` only, the test folders older than
//! any test runs whose lock no one holds. A folder's lock is removed last, and
//! kept while anything else in it cannot be removed, so that a later sweep
//! tries again (#434). A `c2t` that is a symbolic link or junction is refused
//! when checked, before it is made and before each sweep; a test folder that
//! is one is skipped when checked, before its lock is opened and before it is
//! emptied.

use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use windows_sys::Win32::{
    Foundation::WAIT_OBJECT_0,
    Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
    System::Threading::{
        OpenProcess, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
    },
};

/// Names a folder to use in place of `%LOCALAPPDATA%`: test folders then go
/// in its `c2t`.
pub const SHORT_ROOT: &str = "CLIMON2_TEST_SHORT_ROOT";
/// The longest folder this makes: a socket path, at most 107 bytes, adds 54
/// to `LOCALAPPDATA`, and the socket-length test sets `LOCALAPPDATA` to the
/// folder plus at least `\r`.
const FOLDER_MAX: usize = 107 - 54 - 2;
/// What a test folder adds to the root it is in: `\c2t\` and six random
/// characters.
const FOLDER_SUFFIX: usize = 4 + 1 + 6;
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
    /// Taken when dropped, and closed only once the rest of the folder is gone.
    lock: Option<File>,
    path: PathBuf,
}

impl LocalAppData {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for LocalAppData {
    fn drop(&mut self) {
        if let Some(lock) = self.lock.take() {
            remove(&self.path, lock);
        }
    }
}

/// `c2t` in [`SHORT_ROOT`] when it is set, else in `%LOCALAPPDATA%`. Only
/// this folder is ever swept, never the root it is in.
fn parent() -> PathBuf {
    let root = match std::env::var_os(SHORT_ROOT) {
        Some(root) => {
            let root = PathBuf::from(root);
            assert!(
                root.is_absolute(),
                "{SHORT_ROOT} must name an absolute folder"
            );
            root
        }
        None => std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .filter(|base| base.is_absolute())
            .unwrap_or_else(|| {
                panic!("LOCALAPPDATA must name an absolute folder, or set {SHORT_ROOT}")
            }),
    };
    root.join("c2t")
}

/// A new folder in [`parent`], to set as a test's `LOCALAPPDATA` whatever
/// `TEMP` is.
pub fn local_app_data() -> LocalAppData {
    let parent = parent();
    ready(&parent, SystemTime::now());
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
        lock: Some(lock),
        path: folder.keep(),
    }
}

/// Makes `parent` where it is missing, then sweeps it. A `parent` that is a
/// reparse point, such as a symbolic link or junction another user made first,
/// is refused when checked: before it is made and before it is swept.
fn ready(parent: &Path, now: SystemTime) {
    refuse_reparse_point(parent);
    fs::create_dir_all(parent)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    sweep(parent, now);
}

/// Panics if `folder` is a reparse point; a missing one is no error.
fn refuse_reparse_point(folder: &Path) {
    match is_reparse_point(folder) {
        Ok(false) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Ok(true) => panic!(
            "{} is a symbolic link or junction, which test folders are not made in or \
             swept through; remove it, and if {SHORT_ROOT} is set, name a folder only you \
             can write to",
            folder.display()
        ),
        Err(error) => panic!("cannot check {}: {error}", folder.display()),
    }
}

/// Whether `path` itself, not what it may point to, is a reparse point.
fn is_reparse_point(path: &Path) -> io::Result<bool> {
    Ok(fs::symlink_metadata(path)?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
}

/// Why `folder` cannot be a test's `LOCALAPPDATA`, if it is too long.
fn too_long(folder: &Path) -> Option<String> {
    let length = folder.as_os_str().len();
    (length > FOLDER_MAX).then(|| {
        format!(
            "{} is {length} bytes, over the {FOLDER_MAX} a 107-byte socket path leaves; \
             set {SHORT_ROOT} to a folder of at most {} bytes, as test folders add \
             `\\c2t\\` and six characters to it",
            folder.display(),
            FOLDER_MAX - FOLDER_SUFFIX
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

/// Removes the test folders in `parent` made [`STALE`] or longer before `now`
/// whose lock no one holds. A test folder is a folder holding a [`LOCK`] file;
/// anything else, files and other folders, is left alone.
fn sweep(parent: &Path, now: SystemTime) {
    // Checked again right before it is read, in case one was made in its
    // place since; making a folder where one already is makes nothing.
    refuse_reparse_point(parent);
    let Ok(entries) = fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            continue;
        }
        let Ok(made) = metadata.created().or_else(|_| metadata.modified()) else {
            continue;
        };
        if !now.duration_since(made).is_ok_and(|age| age >= STALE) {
            continue;
        }
        let path = entry.path();
        // Checked again by name right before its lock is opened through it.
        if is_reparse_point(&path).unwrap_or(true) {
            continue;
        }
        // Fails while the lock is held, and where there is none.
        if let Ok(lock) = take(&path.join(LOCK)) {
            remove(&path, lock);
        }
    }
}

/// Opens the lock file at `path`, which must exist, with no sharing, so it
/// fails while the folder's test holds it.
fn take(path: &Path) -> io::Result<File> {
    OpenOptions::new().read(true).share_mode(0).open(path)
}

/// Removes test `folder`, whose `lock` is held here: everything but the lock
/// first, then, only if all of that went, the lock and the folder. Where a
/// leftover process still holds a file in it, or writes one after it was
/// emptied, the lock stays or is made again, so a later sweep tries again.
/// A `folder` that is a reparse point is left alone.
fn remove(folder: &Path, lock: File) {
    if is_reparse_point(folder).unwrap_or(true) {
        return;
    }
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut left = false;
    for entry in entries {
        let Ok(entry) = entry else {
            left = true;
            continue;
        };
        if entry.file_name() == LOCK {
            continue;
        }
        let path = entry.path();
        // Neither follows a symbolic link or junction; it removes the link.
        // Both remove read-only files on Windows.
        left |= fs::remove_dir_all(&path)
            .or_else(|_| fs::remove_file(&path))
            .is_err();
    }
    drop(lock);
    if !left {
        let _ = fs::remove_file(folder.join(LOCK));
        if fs::remove_dir(folder).is_err() && folder.exists() {
            let _ = File::create(folder.join(LOCK));
        }
    }
}

/// Ends process `pid`, then, where it may wait on it, waits up to ten seconds
/// for it to exit, so that the folders it used can be removed. A process
/// already gone is no error.
pub fn end_process(pid: u32) {
    let Some(process) = open(pid, PROCESS_TERMINATE) else {
        return;
    };
    // The handle is open with PROCESS_TERMINATE; ending an ended process only
    // fails.
    unsafe { TerminateProcess(process.as_raw_handle(), 1) };
    let Some(process) = open(pid, PROCESS_SYNCHRONIZE) else {
        return;
    };
    let milliseconds = u32::try_from(EXIT_WAIT.as_millis()).unwrap_or(u32::MAX);
    // The handle is open with SYNCHRONIZE.
    let waited = unsafe { WaitForSingleObject(process.as_raw_handle(), milliseconds) };
    if waited != WAIT_OBJECT_0 {
        eprintln!("process {pid} did not exit within {EXIT_WAIT:?}");
    }
}

fn open(pid: u32, access: u32) -> Option<OwnedHandle> {
    // Opening by PID has no preconditions.
    let raw = unsafe { OpenProcess(access, 0, pid) };
    // A new handle nothing else owns, closed when dropped.
    (!raw.is_null()).then(|| unsafe { OwnedHandle::from_raw_handle(raw) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

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
        assert!(refusal.contains("at most 40 bytes"), "{refusal}");
    }

    /// A sweep removes only test folders at least an hour old and not locked.
    #[test]
    fn a_sweep_removes_only_stale_unlocked_test_folders() {
        let parent = tempfile::tempdir().unwrap();
        let unlocked = parent.path().join("unlocked");
        fs::create_dir_all(unlocked.join("inner")).unwrap();
        File::create(unlocked.join(LOCK)).unwrap();
        let locked = parent.path().join("locked");
        fs::create_dir_all(locked.join("inner")).unwrap();
        let held = hold(&locked.join(LOCK)).unwrap();

        sweep(parent.path(), SystemTime::now());
        assert!(unlocked.is_dir() && locked.is_dir());

        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!unlocked.exists());
        assert!(locked.join("inner").is_dir() && locked.join(LOCK).is_file());

        drop(held);
        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!locked.exists());
    }

    /// Old files, and old folders without a lock file, are not test folders.
    #[test]
    fn a_sweep_leaves_files_and_folders_without_a_lock() {
        let parent = tempfile::tempdir().unwrap();
        let file = parent.path().join("file.txt");
        fs::write(&file, "unrelated").unwrap();
        let folder = parent.path().join("folder");
        fs::create_dir_all(folder.join("inner")).unwrap();
        fs::write(folder.join("data.txt"), "unrelated").unwrap();

        sweep(parent.path(), SystemTime::now() + STALE * 24);

        assert_eq!(fs::read_to_string(&file).unwrap(), "unrelated");
        assert!(folder.join("inner").is_dir() && folder.join("data.txt").is_file());
    }

    /// A stale folder holding a file a leftover process still has open, with
    /// no sharing, keeps its lock, so a later sweep removes it once the file
    /// is closed.
    #[test]
    fn a_folder_a_sweep_cannot_empty_keeps_its_lock_for_a_later_sweep() {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("stale");
        fs::create_dir_all(folder.join("inner")).unwrap();
        File::create(folder.join(LOCK)).unwrap();
        let log = hold(&folder.join("daemon.log")).unwrap();

        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!folder.join("inner").exists());
        assert!(folder.join(LOCK).is_file() && folder.join("daemon.log").is_file());

        drop(log);
        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!folder.exists());
    }

    /// A dropped folder that cannot be emptied keeps its lock in the same way.
    #[test]
    fn a_folder_its_drop_cannot_empty_keeps_its_lock_for_a_later_sweep() {
        let folder = local_app_data();
        let path = folder.path().to_owned();
        let log = hold(&path.join("daemon.log")).unwrap();

        drop(folder);
        assert!(path.join(LOCK).is_file() && path.join("daemon.log").is_file());

        drop(log);
        remove(&path, take(&path.join(LOCK)).unwrap());
        assert!(!path.exists());
    }

    /// A read-only file does not stop a sweep.
    #[test]
    fn a_sweep_removes_a_read_only_file() {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("stale");
        fs::create_dir(&folder).unwrap();
        File::create(folder.join(LOCK)).unwrap();
        let file = folder.join("read-only.txt");
        File::create(&file).unwrap();
        let mut permissions = fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&file, permissions).unwrap();

        sweep(parent.path(), SystemTime::now() + STALE);

        assert!(!folder.exists());
    }

    /// Where the folder itself cannot be removed once emptied, as when a
    /// leftover process writes a file in it meanwhile, its lock is made again
    /// so a later sweep tries again. A handle on the folder without delete
    /// sharing stands in for that file.
    #[test]
    fn a_folder_that_outlives_its_lock_gets_it_back() {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("stale");
        fs::create_dir(&folder).unwrap();
        File::create(folder.join(LOCK)).unwrap();
        let open = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&folder)
            .unwrap();

        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(folder.join(LOCK).is_file());

        drop(open);
        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!folder.exists());
    }

    /// A `c2t` that is a junction or symbolic link is refused, whether or not
    /// its target exists, and nothing is made or removed through it.
    #[test]
    fn a_junction_or_symbolic_link_at_c2t_is_refused_and_its_target_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("target");
        let stale = target.join("stale");
        fs::create_dir_all(&stale).unwrap();
        File::create(stale.join(LOCK)).unwrap();
        let junction = |name: &str, to: &Path| {
            let link = root.path().join(name).join("c2t");
            fs::create_dir(link.parent().unwrap()).unwrap();
            let made = std::process::Command::new("cmd")
                .args(["/c", "mklink", "/J"])
                .arg(&link)
                .arg(to)
                .output()
                .unwrap();
            assert!(made.status.success(), "{made:?}");
            link
        };

        assert_refused(&junction("junction", &target));
        assert!(stale.join(LOCK).is_file());

        let missing = root.path().join("missing");
        assert_refused(&junction("dangling", &missing));
        assert!(!missing.exists());

        let link = root.path().join("symbolic").join("c2t");
        fs::create_dir(link.parent().unwrap()).unwrap();
        // Making one needs Developer Mode or an administrator.
        match std::os::windows::fs::symlink_dir(&target, &link) {
            Ok(()) => {
                assert_refused(&link);
                assert!(stale.join(LOCK).is_file());
            }
            Err(error) => eprintln!("no symbolic link to test: {error}"),
        }
    }

    fn assert_refused(c2t: &Path) {
        let payload = std::panic::catch_unwind(|| ready(c2t, SystemTime::now() + STALE))
            .expect_err("it was not refused");
        let refusal = payload.downcast::<String>().unwrap();
        assert!(
            refusal.contains(&format!("{} is a symbolic link or junction", c2t.display())),
            "{refusal}"
        );
    }

    /// Test folders go in `c2t` in the root, never in the root itself.
    #[test]
    fn test_folders_go_in_c2t() {
        assert_eq!(parent().file_name(), Some(std::ffi::OsStr::new("c2t")));
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
