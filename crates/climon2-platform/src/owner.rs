//! The home owner (ADR 0011, "One home, one host"): one climon2 home belongs
//! to one host, which `<home>/owner` records: its stable machine ID, or on
//! Linux without one, its PID namespace. Every command that uses the home
//! checks it first, here.
//!
//! - A home without a record is claimed by the first check that can read its
//!   own origin. The record is written to an owner-only file of its own and
//!   linked into place, so it appears whole and once: when checks race, the
//!   one whose record is in place owns the home, and the others are judged
//!   against it. Where the file system cannot link, the record is created in
//!   place, exclusively, so racing claims still agree.
//! - A record is read only when it is a plain file, never through a link, and
//!   only when it is valid; a bad one is refused and never replaced.
//! - Without its own origin, a check carries on unclaimed, with a warning,
//!   while the home has no owner, and refuses once it has one.
use crate::{origin, private};
use climon2_core::{
    metadata::SessionOrigin,
    owner::{self, OWNER_FILE, OWNER_RECORD_LIMIT, Refusal},
};
use std::{
    fs,
    hash::{BuildHasher, Hasher},
    io::{self, Read, Write},
    path::Path,
};

/// How a staged record is linked into place: `fs::hard_link`, or a test's.
pub type Link = fn(&Path, &Path) -> io::Result<()>;

/// Whether this host may use the climon2 home at `home`, claiming it first
/// when it has no owner.
pub fn check(home: &Path) -> Result<(), Refusal> {
    check_as(home, origin::current())
}

/// [`check`] for a host whose origin is `here`.
pub fn check_as(home: &Path, here: io::Result<SessionOrigin>) -> Result<(), Refusal> {
    check_linking(home, here, |from, to| fs::hard_link(from, to))
}

/// [`check_as`], linking a claim's record into place with `link`.
pub fn check_linking(
    home: &Path,
    here: io::Result<SessionOrigin>,
    link: Link,
) -> Result<(), Refusal> {
    match recorded(home)? {
        Some(owner) => judge(owner, here),
        None => claim(home, here, link),
    }
}

fn judge(owner: SessionOrigin, here: io::Result<SessionOrigin>) -> Result<(), Refusal> {
    match here {
        Ok(here) if here.is_same_place(&owner) => Ok(()),
        Ok(_) => Err(Refusal::Foreign(owner)),
        Err(error) => Err(Refusal::Unchecked { owner, error }),
    }
}

fn claim(home: &Path, here: io::Result<SessionOrigin>, link: Link) -> Result<(), Refusal> {
    match here {
        Ok(here) => claim_as(home, here, link),
        Err(error) => {
            tracing::warn!(
                %error,
                errno = error.raw_os_error(),
                "cannot read this host's origin; the climon2 home is not claimed"
            );
            Ok(())
        }
    }
}

/// Whoever's record is in place after this host's attempt owns the home; with
/// none in place, the home goes on unclaimed.
fn claim_as(home: &Path, here: SessionOrigin, link: Link) -> Result<(), Refusal> {
    let written = write(home, &here, link);
    match recorded(home)? {
        Some(owner) => judge(owner, Ok(here)),
        None => {
            written.unwrap_or_else(|error| {
                tracing::warn!(
                    %error,
                    errno = error.raw_os_error(),
                    "cannot record the climon2 home's owner; the home is not claimed"
                )
            });
            Ok(())
        }
    }
}
/// The owner the home records, if it records one.
fn recorded(home: &Path) -> Result<Option<SessionOrigin>, Refusal> {
    read(home).map_err(|error| Refusal::Unreadable {
        path: home.join(OWNER_FILE),
        error,
    })
}

/// The owner the home at `home` records, without checking or claiming
/// anything: `None` when it records none, and an error when its record is not
/// a plain, valid file. For `c2 home`.
pub fn read(home: &Path) -> io::Result<Option<SessionOrigin>> {
    match load(&home.join(OWNER_FILE)) {
        Ok(owner) => Ok(Some(owner)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// What `release` removed.
#[derive(Debug)]
pub enum Released {
    /// The home had no owner record.
    Nothing,
    /// The record of this owner.
    Owner(SessionOrigin),
    /// A record that was not a plain, valid file, for this reason: a symlink is
    /// removed itself, never its target, and an empty folder is removed too.
    Unreadable(io::Error),
}

/// Removes the home's owner record, whoever it names, so the next command
/// claims the home afresh. Nothing is checked first: this is how a home is
/// moved, or recovered when its owner is gone, and how a bad record is
/// cleared.
pub fn release(home: &Path) -> io::Result<Released> {
    let released = match read(home) {
        Ok(Some(owner)) => Released::Owner(owner),
        Ok(None) => return Ok(Released::Nothing),
        Err(error) => Released::Unreadable(error),
    };
    remove(&home.join(OWNER_FILE))?;
    Ok(released)
}

/// Removes the record, or a link or an empty folder standing in for it. A
/// folder that cannot be removed is left for the user to delete.
fn remove(path: &Path) -> io::Result<()> {
    if !fs::symlink_metadata(path)?.is_dir() {
        return fs::remove_file(path);
    }
    fs::remove_dir(path).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "{} is a folder that cannot be removed ({error}); delete it by hand",
                path.display()
            ),
        )
    })
}

fn load(path: &Path) -> io::Result<SessionOrigin> {
    let mut bytes = Vec::new();
    private::open_plain_file(path)?
        .take(OWNER_RECORD_LIMIT)
        .read_to_end(&mut bytes)?;
    owner::from_json(&bytes).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Links a whole record into place unless one is there already. Each attempt
/// stages it in a file of its own, named by this process and a random number,
/// so nothing a crash left behind is in the way. Where the link fails, as on a
/// file system without links, the record is created in place exclusively,
/// which still fails when another claim got there first; a reader may then
/// meet it half written, and refuses it until it is whole.
fn write(home: &Path, here: &SessionOrigin, link: Link) -> io::Result<()> {
    let bytes = owner::to_json(here).map_err(io::Error::other)?;
    let path = home.join(OWNER_FILE);
    let temporary = home.join(format!(
        ".owner.{}.{:016x}.tmp",
        std::process::id(),
        random()
    ));
    let linked = stage(&temporary, &bytes).and_then(|()| link(&temporary, &path));
    // Linked or not, the temporary name has done its work; one left behind
    // only costs space.
    let _ = fs::remove_file(&temporary);
    linked.or_else(|_| stage(&path, &bytes))
}

/// A new owner-only file at `path` holding `bytes`, never through a link.
fn stage(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = private::create_private_file(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// A number no earlier call is likely to have given: the standard library
/// seeds each hasher randomly.
fn random() -> u64 {
    std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish()
}
