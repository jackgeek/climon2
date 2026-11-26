//! A test's own `LOCALAPPDATA`, short enough for its sessions' socket paths
//! (#422). A socket is `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock`, 54 bytes more
//! than `LOCALAPPDATA`, and its path has 107 bytes, so `LOCALAPPDATA` may
//! have 53. A folder in `TEMP` does not leave that room: the mutation gate's
//! `TEMP` is 38 bytes, and a `tempfile` folder in it with `\local` is 55.

use std::path::PathBuf;
use tempfile::TempDir;

/// The bytes `%LOCALAPPDATA%\c2\<hash8>\<uuid>.sock` adds to `LOCALAPPDATA`.
const SOCKET_SUFFIX: usize = 54;
/// `sun_path` holds 108 bytes of UTF-8 with its NUL.
const SOCKET_PATH_MAX: usize = 107;

/// A new, empty folder in the system drive's root, removed when dropped, to
/// set as a test's `LOCALAPPDATA` whatever `TEMP` is. The root's inherited
/// grant lets any signed-in account create it, as `cross_account_windows.rs`
/// relies on too.
pub fn local_app_data() -> TempDir {
    let drive = PathBuf::from(format!(
        "{}\\",
        std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into())
    ));
    let folder = tempfile::Builder::new()
        .prefix("c2-")
        .tempdir_in(&drive)
        .unwrap_or_else(|error| panic!("cannot create a folder in {}: {error}", drive.display()));
    let length = folder.path().as_os_str().len();
    assert!(
        length + SOCKET_SUFFIX <= SOCKET_PATH_MAX,
        "{} is {length} bytes, too long for a {SOCKET_PATH_MAX}-byte socket path",
        folder.path().display()
    );
    folder
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever `TEMP` is, the folder is in the system drive's root, leaves a
    /// socket path within budget, and is gone once dropped.
    #[test]
    fn the_folder_is_in_the_drive_root_short_enough_and_removed() {
        let folder = local_app_data();
        let path = folder.path().to_owned();
        let drive = format!("{}\\", std::env::var("SystemDrive").unwrap());

        assert_eq!(path.parent(), Some(std::path::Path::new(&drive)));
        assert!(path.as_os_str().len() + SOCKET_SUFFIX <= SOCKET_PATH_MAX);
        assert!(path.is_dir());
        drop(folder);
        assert!(!path.exists());
    }
}
