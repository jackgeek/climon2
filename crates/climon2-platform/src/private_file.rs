//! Files only their owner may read or write.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use std::{
        fs::{File, OpenOptions},
        io,
        os::unix::fs::OpenOptionsExt,
        path::Path,
    };

    /// Creates `path` as a new file, writable and readable only by its owner.
    /// Anything already there, a symbolic link included, refuses it.
    pub fn create(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    #[cfg(test)]
    mod tests {
        use super::create;
        use std::{fs, io, os::unix::fs::PermissionsExt};

        /// A new file is its owner's alone; a path already taken, by a file
        /// or by a symbolic link, is refused and left as it was.
        #[test]
        fn creates_only_new_owner_only_files() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("log");
            let file = create(&path).unwrap();
            let mode = file.metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let taken = create(&path).unwrap_err();
            assert_eq!(taken.kind(), io::ErrorKind::AlreadyExists);
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(directory.path().join("absent"), &link).unwrap();
            assert_eq!(
                create(&link).unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
            assert!(!directory.path().join("absent").exists());
            assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 2);
        }
    }
}
