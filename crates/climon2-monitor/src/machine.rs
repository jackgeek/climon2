//! Which machine a session runs on (#502; #195, "Identity"). A local
//! session is named by its session ID alone; a remote session by the pinned
//! public key of the relay it is reached through, plus its session ID on
//! that relay. The two are never compared, so a remote session is never
//! confused with a local one, or with one on another relay, even when
//! their session IDs are the same.
use climon2_core::SessionId;
use climon2_link::PublicKey;
use std::fmt;

/// The machine a session runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Machine {
    /// This machine.
    Local,
    /// The machine of the relay with this pinned public key.
    Relay(PublicKey),
}

/// One session, by its machine and its session ID there. Ordered by
/// session ID first, so rows that tie on everything else keep the
/// existing session ID order.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionAt {
    pub id: SessionId,
    pub machine: Machine,
}

impl SessionAt {
    /// The local session `id`.
    pub fn local(id: SessionId) -> Self {
        Self {
            id,
            machine: Machine::Local,
        }
    }

    /// The session `id` on the relay with `relay`'s key.
    pub fn remote(relay: PublicKey, id: SessionId) -> Self {
        Self {
            id,
            machine: Machine::Relay(relay),
        }
    }

    /// Its session ID, when it is a local session.
    pub fn local_id(&self) -> Option<&SessionId> {
        (self.machine == Machine::Local).then_some(&self.id)
    }
}

impl From<SessionId> for SessionAt {
    fn from(id: SessionId) -> Self {
        Self::local(id)
    }
}

/// A local session as its session ID; a remote one with its relay's key
/// fingerprint, for logs.
impl fmt::Display for SessionAt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.machine {
            Machine::Local => write!(formatter, "{}", self.id),
            Machine::Relay(relay) => write!(formatter, "{} on {}", self.id, relay.fingerprint()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(last: u8) -> SessionId {
        format!("0d5c0000-0000-4000-8000-0000000000{last:02x}")
            .parse()
            .unwrap()
    }

    /// ADV-05: one session ID locally and on two relays names three
    /// sessions; each is equal only to itself.
    #[test]
    fn adv_05_one_session_id_on_three_machines_is_three_sessions() {
        let (first, second) = (
            PublicKey::from_bytes([1; 32]),
            PublicKey::from_bytes([2; 32]),
        );
        let local = SessionAt::local(id(1));
        let on_first = SessionAt::remote(first, id(1));
        let on_second = SessionAt::remote(second, id(1));
        assert_ne!(local, on_first);
        assert_ne!(on_first, on_second);
        assert_eq!(on_first, SessionAt::remote(first, id(1)));
        assert_eq!(local.local_id(), Some(&id(1)));
        assert_eq!(on_first.local_id(), None);
        assert_eq!(SessionAt::from(id(1)), local);
    }

    /// The session ID orders first: the machine breaks only a tie.
    #[test]
    fn sessions_order_by_session_id_then_machine() {
        let relay = PublicKey::from_bytes([1; 32]);
        let mut sessions = vec![
            SessionAt::remote(relay, id(1)),
            SessionAt::local(id(2)),
            SessionAt::local(id(1)),
        ];
        sessions.sort();
        assert_eq!(
            sessions,
            [
                SessionAt::local(id(1)),
                SessionAt::remote(relay, id(1)),
                SessionAt::local(id(2)),
            ]
        );
    }

    #[test]
    fn a_remote_session_is_logged_with_its_relays_fingerprint() {
        let relay = PublicKey::from_bytes([1; 32]);
        assert_eq!(SessionAt::local(id(1)).to_string(), id(1).to_string());
        assert_eq!(
            SessionAt::remote(relay, id(1)).to_string(),
            format!("{} on {}", id(1), relay.fingerprint())
        );
    }
}
