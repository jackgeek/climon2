//! The key storage seam (ADR 0003): the owner-only folder that holds a
//! climon2 home's pairing records and private keys, `links/`. Keys are plain
//! owner-only files, as SSH keys are; there is no passphrase and no
//! operating-system keychain.
//!
//! - [`KeyStore::open`] creates the folder when missing and requires it to
//!   be private.
//! - [`KeyStore::store`] replaces a file's whole contents atomically.
//! - [`KeyStore::load`] reads a file back, and reports a damaged one as an
//!   error, never as an absent or empty one.
//!
//! What the bytes mean is the caller's: contents this seam accepts can
//! still be malformed for their reader.

use crate::private::{create_private_file, private_directory};
use std::{
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Component, Path, PathBuf},
};

/// One home's key files, in a folder only its user can reach.
#[derive(Debug)]
pub struct KeyStore {
    directory: PathBuf,
}

impl KeyStore {
    /// Opens the key folder at `directory`, creating it owner-only when it
    /// is missing. A folder anyone else could change is refused with the
    /// "unsafe runtime directory" error and never repaired.
    pub fn open(directory: &Path) -> io::Result<Self> {
        private_directory(directory)?;
        Ok(Self {
            directory: directory.to_path_buf(),
        })
    }

    /// Replaces the whole contents of the key file `name`. The contents go
    /// to a new owner-only temporary file beside it, which is flushed to
    /// disk and renamed into place, so a reader or a crash finds the old
    /// contents or the new ones and nothing between. A link or a looser
    /// file already at `name` is replaced, never written through. Empty
    /// contents are refused, because [`KeyStore::load`] reads an empty file
    /// as damaged.
    ///
    /// A process killed mid-write leaves its temporary file behind: it is
    /// owner-only, named `.<name>.<random>.tmp`, and never read.
    pub fn store(&self, name: &str, contents: &[u8]) -> io::Result<()> {
        let path = self.path(name)?;
        if contents.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("no contents to store in key file {}", path.display()),
            ));
        }
        let temporary = self.temporary(name)?;
        let published =
            write_new(&temporary, contents).and_then(|()| fs::rename(&temporary, &path));
        if published.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        published?;
        // The rename itself reaches the disk with the folder.
        File::open(&self.directory)?.sync_all()
    }

    /// The contents of the key file `name`, or `None` when no such file
    /// exists. Anything else at that name is damage and an `InvalidData`
    /// error naming the file: a file that cannot be read, a link, a folder
    /// or other special file, a file another user owns or can reach, and an
    /// empty file.
    pub fn load(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        let path = self.path(name)?;
        // Never through a link.
        let opened = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path);
        let mut file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(damaged(&path, &error)),
        };
        let metadata = file.metadata().map_err(|error| damaged(&path, &error))?;
        if let Some(problem) = problem(&metadata) {
            return Err(damaged(&path, &problem));
        }
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)
            .map_err(|error| damaged(&path, &error))?;
        Ok(Some(contents))
    }

    /// Where the key file `name` lives. A name is one plain file name: no
    /// separator, and neither `.` nor `..`, so no name leaves the folder.
    fn path(&self, name: &str) -> io::Result<PathBuf> {
        let mut parts = Path::new(name).components();
        match (parts.next(), parts.next()) {
            (Some(Component::Normal(part)), None) if part == name => Ok(self.directory.join(name)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{name:?} is not a key file name"),
            )),
        }
    }

    /// A temporary name no other writer of `name` picks, in the same folder
    /// so the rename never crosses a file system.
    fn temporary(&self, name: &str) -> io::Result<PathBuf> {
        let mut unique = [0; 8];
        crate::randomness::fill(&mut unique)?;
        let unique = u64::from_ne_bytes(unique);
        Ok(self.directory.join(format!(".{name}.{unique:016x}.tmp")))
    }
}

/// Writes `contents` to a new owner-only file and flushes them to disk.
fn write_new(path: &Path, contents: &[u8]) -> io::Result<()> {
    let mut file = create_private_file(path)?;
    file.write_all(contents)?;
    file.sync_all()
}

/// What makes an opened key file damaged, if anything.
fn problem(metadata: &Metadata) -> Option<&'static str> {
    if !metadata.is_file() {
        Some("not a regular file")
    } else if metadata.uid() != crate::peer::effective_uid() {
        Some("wrong owner")
    } else if metadata.mode() & 0o077 != 0 {
        Some("group or other permissions")
    } else if metadata.len() == 0 {
        Some("empty")
    } else {
        None
    }
}

fn damaged(path: &Path, problem: &dyn std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("damaged key file {}: {problem}", path.display()),
    )
}
