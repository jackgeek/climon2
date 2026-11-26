//! Private files and folders (ADR 0003): every file climon2 creates and every
//! folder it manages for one user. A new file is never opened through a link
//! or over an existing file, and a folder that anyone but its user could
//! change is refused, with the "unsafe runtime directory" error, rather than
//! repaired.
//!
//! - [`create_new_file`] creates a new file only its user can read and write.
//! - [`create_private_file`] does the same, owner-only whatever the umask.
//! - [`private_directory`] creates a managed folder when missing, then
//!   requires it to be private.
//! - [`open_plain_file`] opens a file to read only when it is a plain file,
//!   never through a link.

pub use native::{create_new_file, create_private_file, open_plain_file, private_directory};

#[cfg(unix)]
mod native {
    use std::{
        fs::{self, DirBuilder, File, OpenOptions},
        io,
        os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
        path::Path,
    };

    pub fn private_directory(path: &Path) -> io::Result<()> {
        create_directory(path)?;
        check_directory(path)
    }

    /// A new file only this user can read and write once the umask is
    /// applied, never opened through a symlink or over an existing file:
    /// every file climon2 creates is created here.
    pub fn create_new_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    /// An existing file to read, never through a symlink and only when it is a
    /// regular file: a directory or FIFO at the name is refused unopened or
    /// unread.
    pub fn open_plain_file(path: &Path) -> io::Result<File> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        if !file.metadata()?.is_file() {
            return Err(super::not_plain(path));
        }
        Ok(file)
    }

    /// As [`create_new_file`], made exactly owner-only whatever the umask.
    /// A file that cannot be made private is removed again.
    pub fn create_private_file(path: &Path) -> io::Result<File> {
        let file = create_new_file(path)?;
        if let Err(error) = file.set_permissions(fs::Permissions::from_mode(0o600)) {
            let _ = fs::remove_file(path);
            return Err(error);
        }
        Ok(file)
    }

    fn create_directory(path: &Path) -> io::Result<()> {
        match DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn check_directory(path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(unsafe_directory(path, "not a real directory"));
        }
        if metadata.uid() != crate::peer::effective_uid() {
            return Err(unsafe_directory(path, "wrong owner"));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(unsafe_directory(path, "group or other permissions"));
        }
        Ok(())
    }

    fn unsafe_directory(path: &Path, problem: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("unsafe runtime directory {}: {problem}", path.display()),
        )
    }
}

/// The refusal of a name that is not a plain file.
fn not_plain(path: &std::path::Path) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        format!("{} is not a plain file", path.display()),
    )
}

#[cfg(windows)]
#[path = "private/windows.rs"]
mod native;
