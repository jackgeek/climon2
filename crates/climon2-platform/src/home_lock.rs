//! The home lock seam (ADR 0003): a lock one process at a time holds on a
//! file in a climon2 home, so that a second relay in the same home refuses
//! to start and can say which relay is running.
//!
//! - [`take`] holds the lock, or reports the note its holder left.
//! - [`Held::note`] leaves that note.
//!
//! The lock goes when its [`Held`] is dropped, and the operating system
//! drops it for a process that ends any other way, so no stale lock
//! survives a crash. The file itself stays: only the lock on it means
//! anything.
//!
//! Unix only, as the other monitor link seams are.

use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::Path,
};

/// The lock, for as long as this value lives.
#[derive(Debug)]
pub struct Held(File);

/// What became of an attempt to take the lock.
#[derive(Debug)]
pub enum Taken {
    /// This process holds the lock now.
    Held(Held),
    /// Another holder has it, and left this note: empty when it has left
    /// none yet.
    Busy(String),
}

/// Takes the lock on the file at `path`, creating the file owner-only when
/// it is missing and never opening it through a link. The folder must
/// exist. A new holder starts with no note, whatever the last one left.
pub fn take(path: &Path) -> io::Result<Taken> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    if !lock(&file)? {
        return Ok(Taken::Busy(note(&mut file)));
    }
    Held::emptied(file).map(Taken::Held)
}

/// The note in `file`. A note that cannot be read as text is no note: a
/// failed read leaves it empty.
fn note(file: &mut File) -> String {
    let mut note = String::new();
    let _ = file.read_to_string(&mut note);
    note
}

impl Held {
    /// The lock on `file`, with no note yet.
    fn emptied(file: File) -> io::Result<Self> {
        file.set_len(0)?;
        Ok(Self(file))
    }

    /// Replaces the note a later taker is given.
    pub fn note(&mut self, text: &str) -> io::Result<()> {
        self.0.set_len(0)?;
        self.0.seek(SeekFrom::Start(0))?;
        self.0.write_all(text.as_bytes())
    }
}

/// Whether `file` is now locked by this open of it; false when another
/// holder has the lock.
fn lock(file: &File) -> io::Result<bool> {
    loop {
        if let Some(locked) = try_lock(file)? {
            return Ok(locked);
        }
    }
}

/// One attempt at the lock: whether this open of `file` now holds it, or
/// `None` when a signal interrupted the attempt.
fn try_lock(file: &File) -> io::Result<Option<bool>> {
    // flock only reads the descriptor `file` owns.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(true));
    }
    refused(io::Error::last_os_error())
}

/// What a refused attempt means: another holder has the lock, the attempt
/// was interrupted, or it failed.
fn refused(error: io::Error) -> io::Result<Option<bool>> {
    if error.kind() == io::ErrorKind::WouldBlock {
        return Ok(Some(false));
    }
    crate::syscall::retry_if_interrupted(error).map(|()| None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_is_busy_interrupted_or_a_failure() {
        let busy = io::Error::from(io::ErrorKind::WouldBlock);
        assert_eq!(refused(busy).unwrap(), Some(false));
        let interrupted = io::Error::from(io::ErrorKind::Interrupted);
        assert_eq!(refused(interrupted).unwrap(), None);
        let failed = io::Error::from_raw_os_error(libc::EBADF);
        assert_eq!(
            refused(failed).unwrap_err().raw_os_error(),
            Some(libc::EBADF)
        );
    }
}
