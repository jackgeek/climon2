//! What the relay advertises to its linked hub monitor (#502; #195, "Advertisement"):
//! pure, fed what each discovery pass finds. Every live session is
//! advertised once, then again only when what crosses the link changes,
//! except the session the relay itself runs in (U3, X14). A session is
//! withdrawn only once discovery proves it stale; an uncertain one stays.
//! Snapshot complete follows the first pass.

use climon2_core::{SessionId, metadata::SessionMetadata};
use climon2_link::{Advertisement, Session};
use std::collections::BTreeMap;

/// The client interface's native path encoding for native Unix bytes: the
/// relay runs on macOS and Linux only.
const UNIX_BYTES: u32 = 1;

/// The most sessions a link holds at once (#412, LM3). An honest relay
/// advertises the first 128 it finds; the rest wait for a free place.
pub(crate) const SESSIONS_MOST: usize = 128;

/// What one discovery probe found, as the advertisement needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Found {
    Live(Box<SessionMetadata>),
    /// Neither proven live nor proven stale.
    Uncertain,
    /// Proven stale: the session has ended.
    Stale(SessionId),
}

/// The sessions one link connection has advertised and not withdrawn.
pub(crate) struct Advertiser {
    /// The session the relay runs in, never advertised.
    own: Option<SessionId>,
    advertised: BTreeMap<SessionId, Session>,
    complete: bool,
}

impl Advertiser {
    /// A link connection's advertisement, which starts with nothing held.
    pub(crate) fn new(own: Option<SessionId>) -> Self {
        Self {
            own,
            advertised: BTreeMap::new(),
            complete: false,
        }
    }

    /// What `found` changes in the advertisement, if anything.
    pub(crate) fn found(&mut self, found: Found) -> Option<Advertisement> {
        match found {
            Found::Live(metadata) => self.live(&metadata),
            Found::Uncertain => None,
            Found::Stale(id) => self.stale(&id),
        }
    }

    /// A live session is advertised when it is new and there is room, or
    /// when what crosses the link changed.
    fn live(&mut self, metadata: &SessionMetadata) -> Option<Advertisement> {
        let id = &metadata.session_id;
        let session = session(metadata);
        let held = self.advertised.get(id);
        let room = held.is_some() || self.advertised.len() < SESSIONS_MOST;
        let fresh = held != Some(&session) && room && self.own.as_ref() != Some(id);
        fresh.then(|| {
            self.advertised.insert(id.clone(), session.clone());
            Advertisement::Session(session)
        })
    }

    fn stale(&mut self, id: &SessionId) -> Option<Advertisement> {
        self.advertised
            .remove(id)
            .map(|_| Advertisement::Withdrawn(id.bytes()))
    }

    /// A pass ended: the first one completes the snapshot.
    pub(crate) fn passed(&mut self) -> Option<Advertisement> {
        (!std::mem::replace(&mut self.complete, true)).then_some(Advertisement::SnapshotComplete)
    }
}

/// What crosses the link of one session (ADV-02): its ID, command,
/// arguments, working directory, depth, creation time and release, and
/// nothing of the machine beyond them.
fn session(metadata: &SessionMetadata) -> Session {
    let bytes = |text: &str| text.as_bytes().to_vec();
    let created_at = metadata.created_at.as_str().as_bytes();
    Session {
        id: metadata.session_id.bytes(),
        encoding: UNIX_BYTES,
        command: bytes(&metadata.command),
        arguments: metadata.args.iter().map(|arg| bytes(arg)).collect(),
        working_directory: bytes(metadata.cwd.as_str()),
        depth: metadata.depth.get(),
        created_at: created_at.try_into().expect("a timestamp is 24 bytes"),
        release: metadata.climon2_version.to_string(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use climon2_core::metadata::LinuxProcessIdentity;
    use std::num::NonZeroU32;

    pub(crate) fn id(last: u8) -> SessionId {
        format!("0d5c0000-0000-4000-8000-0000000000{last:02x}")
            .parse()
            .unwrap()
    }

    /// A live session's metadata, with every field the link withholds set.
    pub(crate) fn metadata(last: u8, command: &str) -> SessionMetadata {
        SessionMetadata {
            format_version: NonZeroU32::MIN,
            session_id: id(last),
            socket_path: "/run/user/1000/climon2/s.sock".parse().unwrap(),
            daemon_pid: 4242,
            daemon_start_time: 17,
            daemon_linux_identity: Some(LinuxProcessIdentity {
                boot_id: "0d5c0000-0000-4000-8000-0000000000ee".to_owned(),
                start_ticks: 7,
            }),
            command: command.to_owned(),
            args: vec!["notes.txt".to_owned(), "-n".to_owned()],
            cwd: "/home/sam".parse().unwrap(),
            depth: NonZeroU32::new(2).unwrap(),
            created_at: "2026-11-27T09:30:00.000Z".parse().unwrap(),
            climon2_version: "2.4.0".parse().unwrap(),
        }
    }

    pub(crate) fn live(last: u8, command: &str) -> Found {
        Found::Live(Box::new(metadata(last, command)))
    }

    fn advertised(advertisement: Option<Advertisement>) -> Session {
        match advertisement {
            Some(Advertisement::Session(session)) => session,
            other => panic!("a session was advertised: {other:?}"),
        }
    }

    /// ADV-02: the session's ID, command, arguments, working directory,
    /// depth, creation time and release cross, as native Unix bytes, and
    /// nothing else: no socket path, daemon process or Linux identity.
    #[test]
    fn adv_02_a_session_carries_its_listing_and_nothing_of_the_machine() {
        let mut advertiser = Advertiser::new(None);
        let session = advertised(advertiser.found(live(1, "vim")));
        assert_eq!(
            session,
            Session {
                id: [
                    0x0d, 0x5c, 0, 0, 0, 0, 0x40, 0, 0x80, 0, 0, 0, 0, 0, 0, 0x01
                ],
                encoding: 1,
                command: b"vim".to_vec(),
                arguments: vec![b"notes.txt".to_vec(), b"-n".to_vec()],
                working_directory: b"/home/sam".to_vec(),
                depth: 2,
                created_at: *b"2026-11-27T09:30:00.000Z",
                release: "2.4.0".to_owned(),
            }
        );
    }

    /// ADV-01: a session is advertised when it appears and again only when
    /// it changes; the first pass then completes the snapshot, once.
    #[test]
    fn adv_01_appearances_and_changes_are_advertised_once_each() {
        let mut advertiser = Advertiser::new(None);
        assert_eq!(advertised(advertiser.found(live(1, "vim"))).command, b"vim");
        assert_eq!(advertiser.found(live(1, "vim")), None, "unchanged");
        assert_eq!(advertiser.passed(), Some(Advertisement::SnapshotComplete));
        assert_eq!(advertiser.passed(), None, "the snapshot completes once");
        assert_eq!(
            advertised(advertiser.found(live(1, "nvim"))).command,
            b"nvim"
        );
        assert_eq!(advertised(advertiser.found(live(2, "top"))).command, b"top");
    }

    /// ADV-04: only proof of staleness withdraws a session, and only one
    /// that was advertised.
    #[test]
    fn adv_04_only_a_session_proven_stale_is_withdrawn() {
        let mut advertiser = Advertiser::new(None);
        advertiser.found(live(1, "vim"));
        assert_eq!(advertiser.found(Found::Uncertain), None);
        assert_eq!(
            advertiser.found(Found::Stale(id(2))),
            None,
            "never advertised"
        );
        assert_eq!(
            advertiser.found(Found::Stale(id(1))),
            Some(Advertisement::Withdrawn(id(1).bytes()))
        );
        assert_eq!(
            advertiser.found(Found::Stale(id(1))),
            None,
            "withdrawn once"
        );
        assert_eq!(advertised(advertiser.found(live(1, "vim"))).command, b"vim");
    }

    /// ADV-03 (U3, X14): the session the relay runs in is never advertised.
    #[test]
    fn adv_03_the_relays_own_session_is_never_advertised() {
        let mut advertiser = Advertiser::new(Some(id(1)));
        assert_eq!(advertiser.found(live(1, "c2 relay")), None);
        assert_eq!(advertised(advertiser.found(live(2, "vim"))).command, b"vim");
    }

    /// LM3: the first 128 sessions found are advertised; a 129th waits
    /// until one is withdrawn, and a held one still changes.
    #[test]
    fn at_most_128_sessions_are_held_at_once() {
        let mut advertiser = Advertiser::new(None);
        for last in 0..128 {
            advertised(advertiser.found(live(last, "vim")));
        }
        assert_eq!(advertiser.found(live(200, "vim")), None);
        assert_eq!(
            advertised(advertiser.found(live(127, "top"))).command,
            b"top"
        );
        advertiser.found(Found::Stale(id(0)));
        assert_eq!(
            advertised(advertiser.found(live(200, "vim"))).id,
            id(200).bytes()
        );
    }
}
