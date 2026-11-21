//! Files only their owner may read or write, created where nothing exists yet.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("private file seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use std::{
        fs::{self, File, OpenOptions},
        io,
        os::unix::fs::{OpenOptionsExt, PermissionsExt},
        path::Path,
    };

    /// Creates `path` for writing, owner-only; fails if anything, a symbolic
    /// link included, is already there.
    pub fn create_new(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    /// Makes `file` owner-only whatever the creation mask removed or kept.
    pub fn restrict(file: &File) -> io::Result<()> {
        file.set_permissions(fs::Permissions::from_mode(0o600))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::MetadataExt;

        #[test]
        fn a_new_file_is_owner_only_and_writable() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("new");
            let file = create_new(&path).unwrap();
            restrict(&file).unwrap();
            io::Write::write_all(&mut &file, b"x").unwrap();
            assert_eq!(fs::read(&path).unwrap(), b"x");
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        }

        #[test]
        fn restricting_removes_every_other_permission() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("wide");
            let file = File::create(&path).unwrap();
            file.set_permissions(fs::Permissions::from_mode(0o666))
                .unwrap();
            restrict(&file).unwrap();
            assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        }

        #[test]
        fn an_existing_file_is_never_reused() {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("existing");
            fs::write(&path, b"kept").unwrap();
            let error = create_new(&path).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(fs::read(&path).unwrap(), b"kept");
        }

        #[test]
        fn a_dangling_symbolic_link_is_never_followed() {
            let directory = tempfile::tempdir().unwrap();
            let target = directory.path().join("target");
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(create_new(&link).is_err());
            assert!(!target.exists());
        }
    }
}
