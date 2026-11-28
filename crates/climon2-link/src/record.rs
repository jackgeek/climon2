//! The records every link connection carries on stream 0 whatever else it
//! does: Hello, the first record each way, and Close, the last. The layouts
//! are those of the protocol's Hello and Close pages.

use crate::layout::{self, Field, layout};

/// The tag of a Hello record.
pub(crate) const HELLO: u8 = 0x01;
/// The tag of a Close record.
pub(crate) const CLOSE: u8 = 0x02;
/// The longest Hello record, tag included.
pub(crate) const HELLO_MOST: usize = 4_096;
/// The smallest "largest record" an end may state.
const CEILING_LEAST: u32 = 4_096;
/// The longest release version, name or capability token, in bytes.
const TEXT_MOST: usize = 64;
/// The most capability tokens one Hello lists.
const TOKENS_MOST: usize = 64;

/// What each end states in its Hello: its release, its suggested name and
/// the longest record it accepts. Link revision 1 publishes no capability
/// token, so a Hello made here lists none and tokens received are ignored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hello {
    release: String,
    name: String,
    largest_record: u32,
}

impl Hello {
    /// A Hello, or nothing when it breaks a limit: `release` is 1 to 64
    /// bytes, `name` at most 64 bytes and `largest_record` at least 4,096.
    pub fn new(release: &str, name: &str, largest_record: u32) -> Option<Self> {
        let valid = !release.is_empty()
            && release.len() <= TEXT_MOST
            && name.len() <= TEXT_MOST
            && largest_record >= CEILING_LEAST;
        valid.then(|| Self {
            release: release.to_owned(),
            name: name.to_owned(),
            largest_record,
        })
    }

    /// The sender's climon2 release version. Hostile text when received.
    pub fn release(&self) -> &str {
        &self.release
    }

    /// The sender's suggested display name. Hostile text when received.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The longest record, tag included, that the sender accepts.
    pub fn largest_record(&self) -> u32 {
        self.largest_record
    }

    /// The Hello record: its tag, then its body. It lists no capability.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut record = vec![HELLO];
        HelloLayout {
            release: self.release.clone(),
            name: self.name.clone(),
            capabilities: Vec::new(),
            largest_record: self.largest_record,
        }
        .put(&mut record);
        record
    }

    /// The Hello whose body is `body`, or nothing when the body breaks the
    /// layout or a limit in any way. The capabilities it lists are read
    /// and ignored.
    pub(crate) fn decode(body: &[u8]) -> Option<Self> {
        let hello = layout::decode(body, HelloLayout::take)?;
        let tokens = &hello.capabilities;
        let listed = tokens.len() <= TOKENS_MOST && tokens.iter().all(|t| t.len() <= TEXT_MOST);
        listed
            .then(|| Hello::new(&hello.release, &hello.name, hello.largest_record))
            .flatten()
    }
}

layout! {
    /// A Hello body as it crosses the link.
    struct HelloLayout {
        release: String,
        name: String,
        capabilities: Vec<String>,
        largest_record: u32,
    }
}

/// The reason in a Close record, as one end may send it.
pub(crate) trait Reason: Copy + 'static {
    /// "Protocol violation", which either end sends, and which stands for
    /// every code this end's peer may not send.
    const VIOLATION: Self;

    /// Every reason this end may send.
    const ALL: &'static [Self];

    /// The reason's code on the wire.
    fn code(self) -> u16;

    /// The reason with this code, when the end may send it.
    fn from_code(code: u16) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|reason| reason.code() == code)
    }

    /// The Close record that gives this reason: its tag, then its body.
    fn encode(self) -> Vec<u8> {
        let [high, low] = self.code().to_be_bytes();
        vec![CLOSE, high, low]
    }

    /// The reason a Close record with the body `body` gives, or nothing
    /// when the body is not one code. A code the sender may not send is
    /// handled as "protocol violation".
    fn decode(body: &[u8]) -> Option<Self> {
        let code = u16::from_be_bytes(body.try_into().ok()?);
        Some(Self::from_code(code).unwrap_or(Self::VIOLATION))
    }
}

/// Why a relay ends or refuses a link. Each reason's value is its code on
/// the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum RelayClose {
    /// Cut by the relay's user.
    Cut = 1,
    /// The relay is stopping.
    Stopping = 2,
    /// The relay is restarting for an upgrade.
    Restarting = 3,
    /// The relay does not remember the hub monitor's key.
    NotPaired = 4,
    /// The relay is on hold.
    OnHold = 5,
    /// The relay is busy with another hub monitor.
    Busy = 6,
    /// A newly paired hub monitor replaced this one.
    Replaced = 7,
    /// The other end broke the protocol.
    ProtocolViolation = 8,
    /// The hub monitor's own newer connection is the link now.
    Superseded = 9,
}

impl Reason for RelayClose {
    const VIOLATION: Self = Self::ProtocolViolation;

    const ALL: &'static [Self] = &[
        Self::Cut,
        Self::Stopping,
        Self::Restarting,
        Self::NotPaired,
        Self::OnHold,
        Self::Busy,
        Self::Replaced,
        Self::ProtocolViolation,
        Self::Superseded,
    ];

    fn code(self) -> u16 {
        self as u16
    }
}

/// Why a hub monitor ends a link. Each reason's value is its code on the
/// wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum HubClose {
    /// The other end broke the protocol.
    ProtocolViolation = 8,
    /// The hub monitor's user disconnected the relay.
    Disconnected = 32,
    /// The hub monitor forgot the relay.
    Forgotten = 33,
    /// The hub monitor stopped.
    Stopped = 34,
}

impl Reason for HubClose {
    const VIOLATION: Self = Self::ProtocolViolation;

    const ALL: &'static [Self] = &[
        Self::ProtocolViolation,
        Self::Disconnected,
        Self::Forgotten,
        Self::Stopped,
    ];

    fn code(self) -> u16 {
        self as u16
    }
}
