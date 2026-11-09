//! Enumerating and validating session metadata (ADR 0005). Invalid entries are
//! reported, never deleted.
use climon2_core::{
    ContractError,
    metadata::{SessionMetadata, metadata_session_id},
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

/// A `.pty` entry that failed the frozen metadata validation.
#[derive(Debug)]
pub struct InvalidMetadata {
    pub path: PathBuf,
    pub reason: InvalidReason,
}

/// Why an entry is not valid session metadata.
#[derive(Debug)]
pub enum InvalidReason {
    /// Its name is not a session ID, or its contents fail the frozen fields or name the
    /// wrong session.
    Malformed(ContractError),
    /// It cannot be read.
    Unreadable(io::Error),
}

#[derive(Default)]
pub(crate) struct Entries {
    pub(crate) valid: Vec<SessionMetadata>,
    pub(crate) invalid: Vec<InvalidMetadata>,
}

impl Entries {
    fn add(&mut self, path: PathBuf) {
        match read(&path) {
            Ok(metadata) => self.valid.push(metadata),
            Err(reason) => self.invalid.push(InvalidMetadata { path, reason }),
        }
    }
}

/// Every `.pty` entry of the home's sessions directory; other names are not metadata.
pub(crate) fn entries(home: &Path) -> io::Result<Entries> {
    let mut entries = Entries::default();
    for path in candidates(home)?
        .into_iter()
        .filter(|path| is_metadata(path))
    {
        entries.add(path);
    }
    Ok(entries)
}

fn candidates(home: &Path) -> io::Result<Vec<PathBuf>> {
    match fs::read_dir(home.join("sessions")) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn is_metadata(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "pty")
}

fn read(path: &Path) -> Result<SessionMetadata, InvalidReason> {
    metadata_session_id(path).map_err(InvalidReason::Malformed)?;
    let bytes = fs::read(path).map_err(InvalidReason::Unreadable)?;
    SessionMetadata::from_json(path, &bytes).map_err(InvalidReason::Malformed)
}
