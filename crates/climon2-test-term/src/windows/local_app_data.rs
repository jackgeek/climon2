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
//! any test runs whose lock no one holds. A folder's lock is removed last,
//! through its own handle and only once the folder can be opened to delete it
//! too, and kept while anything else in it cannot be removed, so that a later
//! sweep tries again (#434, #454). A `c2t` that is a symbolic link or junction
//! is refused when checked, before it is made, once it is made and before
//! each sweep; one that cannot be checked once made is refused too, and is
//! not swept. A test folder that is one is skipped
//! when checked, before its lock is opened and before it is emptied.

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
    Foundation::{FALSE, WAIT_OBJECT_0},
    Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FileDispositionInfo, SetFileInformationByHandle,
    },
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
/// Opens a folder itself, not what it may point to.
const FOLDER_FLAGS: u32 = FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT;
/// Shares a folder opened to delete it with anyone else who shares delete.
const FOLDER_SHARING: u32 = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
const DISPOSITION_SIZE: u32 = size_of::<FILE_DISPOSITION_INFO>() as u32;
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
    let folder = folder_in(&parent);
    if let Some(refusal) = too_long(folder.path()) {
        panic!("{refusal}");
    }
    folder
}

/// A new test folder in `parent`, holding its lock.
fn folder_in(parent: &Path) -> LocalAppData {
    let folder = tempfile::Builder::new()
        .prefix("")
        .tempdir_in(parent)
        .unwrap_or_else(|error| panic!("cannot create a folder in {}: {error}", parent.display()));
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
/// is refused when checked: before it is made, once it is made and before it
/// is swept.
fn ready(parent: &Path, now: SystemTime) {
    // A check that fails as `parent` is missing, or its name is not one,
    // leaves making it to report any real problem.
    match refuse_reparse_point(parent) {
        Err(error)
            if !matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::InvalidFilename
            ) =>
        {
            panic!("cannot check {}: {error}", parent.display())
        }
        _ => {}
    }
    fs::create_dir_all(parent)
        .unwrap_or_else(|error| panic!("cannot create {}: {error}", parent.display()));
    // No test folder is made in one that cannot be checked once it is made.
    refuse_reparse_point(parent)
        .unwrap_or_else(|error| panic!("cannot check {}: {error}", parent.display()));
    sweep(parent, now);
}

/// Panics if `folder` is a reparse point; fails where it cannot be checked.
fn refuse_reparse_point(folder: &Path) -> io::Result<()> {
    if is_reparse_point(folder)? {
        panic!(
            "{} is a symbolic link or junction, which test folders are not made in or \
             swept through; remove it, and if {SHORT_ROOT} is set, name a folder only you \
             can write to",
            folder.display()
        );
    }
    Ok(())
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
/// one else can open or delete it until it is closed. It is opened only to
/// delete it through the handle: it is never read or written. It is always
/// new, in a folder just made.
fn hold(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .access_mode(DELETE)
        .share_mode(0)
        .open(path)
}

/// Removes the test folders in `parent` made [`STALE`] or longer before `now`
/// whose lock no one holds. A test folder is a folder holding a [`LOCK`] file;
/// anything else, files and other folders, is left alone.
fn sweep(parent: &Path, now: SystemTime) {
    // Checked again right before it is read, in case one was made in its
    // place since; making a folder where one already is makes nothing. One
    // that cannot be checked is not swept.
    if let Err(error) = refuse_reparse_point(parent) {
        eprintln!(
            "cannot check {}, so it is not swept: {error}",
            parent.display()
        );
        return;
    }
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
/// fails while the folder's test holds it. As in [`hold`], only to delete it.
fn take(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .access_mode(DELETE)
        .share_mode(0)
        .open(path)
}

/// Removes test `folder`, whose `lock` is held here: everything but the lock
/// first, then, only if all of that went and the folder can be opened to
/// delete it, the lock and the folder, each through its own handle. Where a
/// leftover process still holds a file in it, or holds the folder itself, the
/// lock stays; where one writes a file in it after the lock went, the lock is
/// made again. Either way a later sweep tries again. A `folder` that is a
/// reparse point is left alone.
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
    if left {
        return;
    }
    // Fails while anyone has the folder open without delete sharing, as a
    // process working in it; an indexer or scanner that shares it does not
    // stop it.
    let Ok(this) = OpenOptions::new()
        .access_mode(DELETE)
        .share_mode(FOLDER_SHARING)
        .custom_flags(FOLDER_FLAGS)
        .open(folder)
    else {
        return;
    };
    if delete_on_close(&lock).is_err() {
        return;
    }
    drop(lock);
    if delete_on_close(&this).is_err() {
        restore(folder);
    }
}

/// Marks the file or empty folder open as `handle`, with delete access, to be
/// deleted once its last handle is closed.
fn delete_on_close(handle: &File) -> io::Result<()> {
    let info = FILE_DISPOSITION_INFO { DeleteFile: true };
    // The handle is open with DELETE, and `info` is a FILE_DISPOSITION_INFO
    // of the size given.
    let set = unsafe {
        SetFileInformationByHandle(
            handle.as_raw_handle(),
            FileDispositionInfo,
            (&raw const info).cast(),
            DISPOSITION_SIZE,
        )
    };
    if set == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Makes the lock of `folder` again, where the folder outlived it, so a later
/// sweep tries again; where that fails, says so, as no sweep will.
fn restore(folder: &Path) {
    if let Err(error) = File::create(folder.join(LOCK)) {
        eprintln!(
            "cannot make the lock in {} again, so no sweep will remove it: {error}",
            folder.display()
        );
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
        let parent = tempfile::tempdir().unwrap();
        let folder = folder_in(parent.path());
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

    /// Where the folder itself cannot be removed once emptied, as while a
    /// leftover process has it open as its working folder, its lock is never
    /// removed, so a later sweep tries again. A handle on the folder without
    /// delete sharing stands in for that process; the lock's contents show it
    /// is the same file, not one made again. A handle that shares delete does
    /// not stop the removal.
    #[test]
    fn a_folder_that_cannot_be_removed_keeps_its_own_lock() {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("stale");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join(LOCK), "original").unwrap();
        let open = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&folder)
            .unwrap();

        sweep(parent.path(), SystemTime::now() + STALE);
        assert_eq!(fs::read_to_string(folder.join(LOCK)).unwrap(), "original");

        // One that shares delete, as an indexer or scanner does, does not.
        drop(open);
        let shared = OpenOptions::new()
            .read(true)
            .share_mode(FOLDER_SHARING)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&folder)
            .unwrap();
        sweep(parent.path(), SystemTime::now() + STALE);
        drop(shared);
        assert!(!folder.exists());
    }

    /// Where removing the folder fails only after its lock went, as when a
    /// leftover process writes a file in it meanwhile, the lock is made again
    /// so a later sweep tries again. A read-only folder stands in for that
    /// file: it can be opened to delete, but not deleted.
    #[test]
    fn a_folder_that_outlives_its_lock_gets_it_back() {
        let parent = tempfile::tempdir().unwrap();
        let folder = parent.path().join("stale");
        fs::create_dir(&folder).unwrap();
        File::create(folder.join(LOCK)).unwrap();
        let read_only = ReadOnly::set(&folder);

        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(folder.join(LOCK).is_file());

        drop(read_only);
        sweep(parent.path(), SystemTime::now() + STALE);
        assert!(!folder.exists());
    }

    /// Marks a folder read-only until dropped, so that a failed assertion
    /// still leaves it removable.
    struct ReadOnly<'a>(&'a Path);

    impl<'a> ReadOnly<'a> {
        fn set(path: &'a Path) -> Self {
            set_read_only(path, true).unwrap();
            Self(path)
        }
    }

    impl Drop for ReadOnly<'_> {
        fn drop(&mut self) {
            let _ = set_read_only(self.0, false);
        }
    }

    fn set_read_only(path: &Path, read_only: bool) -> io::Result<()> {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_readonly(read_only);
        fs::set_permissions(path, permissions)
    }

    /// A `c2t` that cannot be checked for some reason other than being
    /// missing is no refusal: making it reports the real problem.
    #[test]
    fn a_c2t_that_cannot_be_checked_is_reported_by_making_it() {
        let root = tempfile::tempdir().unwrap();
        let c2t = root.path().join("bad<name").join("c2t");

        let payload =
            std::panic::catch_unwind(|| ready(&c2t, SystemTime::now())).expect_err("it was made");
        let refusal = payload.downcast::<String>().unwrap();

        assert!(
            refusal.starts_with(&format!("cannot create {}: ", c2t.display())),
            "{refusal}"
        );
    }

    /// A missing `c2t` is made.
    #[test]
    fn a_missing_c2t_is_made() {
        let root = tempfile::tempdir().unwrap();
        let c2t = root.path().join("c2t");

        ready(&c2t, SystemTime::now());

        assert!(c2t.is_dir());
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
