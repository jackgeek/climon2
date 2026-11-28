//! The records every link connection carries on stream 0 whatever else it
//! does: Hello, the first record each way, and Close, the last. The layouts
//! are those of the protocol's Hello and Close pages.

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
const TOKENS_MOST: u32 = 64;

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

    /// The Hello record: its tag, then its body.
    pub(crate) fn encode(&self) -> Vec<u8> {
        let mut record = vec![HELLO];
        for text in [&self.release, &self.name] {
            record.extend((text.len() as u32).to_be_bytes());
            record.extend(text.as_bytes());
        }
        record.extend(0_u32.to_be_bytes());
        record.extend(self.largest_record.to_be_bytes());
        record
    }

    /// The Hello whose body is `body`, or nothing when the body breaks the
    /// layout or a limit in any way.
    pub(crate) fn decode(body: &[u8]) -> Option<Self> {
        let mut body = Body(body);
        let hello = body.hello()?;
        body.0.is_empty().then_some(hello)
    }
}

/// What is left to read of a record's body.
struct Body<'a>(&'a [u8]);

impl<'a> Body<'a> {
    /// The fields of a Hello, up to the end of its layout.
    fn hello(&mut self) -> Option<Hello> {
        let (release, name) = self.names()?;
        let largest_record = self.tokens().and_then(|()| self.u32())?;
        Hello::new(release, name, largest_record)
    }

    /// The release and the name.
    fn names(&mut self) -> Option<(&'a str, &'a str)> {
        Some((self.text()?, self.text()?))
    }

    /// The next `tokens`: at most 64, each read and ignored.
    fn tokens(&mut self) -> Option<()> {
        let count = self.u32()?;
        (count <= TOKENS_MOST).then_some(())?;
        (0..count).try_for_each(|_| self.text().map(drop))
    }

    /// The next `u32`.
    fn u32(&mut self) -> Option<u32> {
        let (value, rest) = self.0.split_first_chunk()?;
        self.0 = rest;
        Some(u32::from_be_bytes(*value))
    }

    /// The next `text` of at most 64 bytes.
    fn text(&mut self) -> Option<&'a str> {
        let size = self.size()?;
        let (text, rest) = self.0.split_at_checked(size)?;
        self.0 = rest;
        std::str::from_utf8(text).ok()
    }

    /// The length of the next `text`, when it is at most 64 bytes.
    fn size(&mut self) -> Option<usize> {
        let size = usize::try_from(self.u32()?).ok()?;
        (size <= TEXT_MOST).then_some(size)
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
