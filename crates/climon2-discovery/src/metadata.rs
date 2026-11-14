//! Enumerating and validating session metadata (ADR 0005). Invalid entries are
//! reported, never deleted.
use climon2_core::{
    ContractError,
    layout::HomeLayout,
    metadata::{SessionMetadata, metadata_session_id},
};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

/// Per-file bounds on reading session metadata; a `None` bound is not applied.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetadataLimits {
    /// The largest accepted file, in bytes.
    pub file_bytes: Option<u64>,
    /// The deepest accepted nesting of JSON objects and arrays, the outer object being 1.
    pub json_depth: Option<usize>,
}

impl MetadataLimits {
    /// What the CLI has always read: whole files, nested as deeply as the JSON parser accepts.
    pub const UNBOUNDED: Self = Self {
        file_bytes: None,
        json_depth: None,
    };
}

/// A `.pty` entry that failed the caller's limits or the frozen metadata validation.
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
    /// It is larger than `MetadataLimits::file_bytes`.
    TooLarge,
    /// It nests deeper than `MetadataLimits::json_depth`.
    TooDeep,
}

#[derive(Default)]
pub(crate) struct Entries {
    pub(crate) valid: Vec<SessionMetadata>,
    pub(crate) invalid: Vec<InvalidMetadata>,
}

impl Entries {
    fn add(&mut self, path: PathBuf, limits: MetadataLimits) {
        match read(&path, limits) {
            Ok(metadata) => self.valid.push(metadata),
            Err(reason) => self.invalid.push(InvalidMetadata { path, reason }),
        }
    }
}

/// Every `.pty` entry of the home's sessions directory; other names are not metadata.
pub(crate) fn entries(home: &HomeLayout, limits: MetadataLimits) -> io::Result<Entries> {
    let mut entries = Entries::default();
    for path in candidates(home)?
        .into_iter()
        .filter(|path| is_metadata(path))
    {
        entries.add(path, limits);
    }
    Ok(entries)
}

fn candidates(home: &HomeLayout) -> io::Result<Vec<PathBuf>> {
    match fs::read_dir(home.sessions()) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

pub(crate) fn is_metadata(path: &Path) -> bool {
    path.extension().is_some_and(|extension| extension == "pty")
}

fn read(path: &Path, limits: MetadataLimits) -> Result<SessionMetadata, InvalidReason> {
    read_counted(path, limits).1
}

/// The entry at `path`, with how many bytes of it were read.
pub(crate) fn read_counted(
    path: &Path,
    limits: MetadataLimits,
) -> (u64, Result<SessionMetadata, InvalidReason>) {
    if let Err(error) = metadata_session_id(path) {
        return (0, Err(InvalidReason::Malformed(error)));
    }
    match contents(path, limits.file_bytes) {
        Ok(bytes) => (bytes.len() as u64, parse(path, &bytes, limits)),
        Err(reason) => (0, Err(reason)),
    }
}

fn parse(
    path: &Path,
    bytes: &[u8],
    limits: MetadataLimits,
) -> Result<SessionMetadata, InvalidReason> {
    within_depth(bytes, limits.json_depth)?;
    SessionMetadata::from_json(path, bytes).map_err(InvalidReason::Malformed)
}

fn contents(path: &Path, file_bytes: Option<u64>) -> Result<Vec<u8>, InvalidReason> {
    match file_bytes {
        None => fs::read(path).map_err(InvalidReason::Unreadable),
        Some(limit) => bounded(path, limit),
    }
}

/// Reads at most one byte past the limit, so an oversized file is never read whole.
fn bounded(path: &Path, limit: u64) -> Result<Vec<u8>, InvalidReason> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(limit.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(InvalidReason::Unreadable)?;
    (limit >= bytes.len() as u64)
        .then_some(bytes)
        .ok_or(InvalidReason::TooLarge)
}

fn within_depth(bytes: &[u8], json_depth: Option<usize>) -> Result<(), InvalidReason> {
    match json_depth {
        Some(limit) if nesting(bytes) > limit => Err(InvalidReason::TooDeep),
        _ => Ok(()),
    }
}

/// The deepest nesting of objects and arrays outside strings. Malformed JSON is left
/// to the parser, which rejects it afterwards.
fn nesting(bytes: &[u8]) -> usize {
    let mut scan = Scan::default();
    bytes
        .iter()
        .map(|byte| scan.step(*byte))
        .max()
        .unwrap_or_default()
}

#[derive(Default)]
struct Scan {
    depth: usize,
    string: bool,
    escaped: bool,
}

impl Scan {
    /// The depth after `byte`.
    fn step(&mut self, byte: u8) -> usize {
        if self.string {
            self.in_string(byte);
        } else {
            self.structural(byte);
        }
        self.depth
    }

    fn in_string(&mut self, byte: u8) {
        self.string = self.escaped || byte != b'"';
        self.escaped = !self.escaped && byte == b'\\';
    }

    fn structural(&mut self, byte: u8) {
        self.string = byte == b'"';
        match byte {
            b'{' | b'[' => self.depth += 1,
            b'}' | b']' => self.depth = self.depth.saturating_sub(1),
            _ => {}
        }
    }
}
