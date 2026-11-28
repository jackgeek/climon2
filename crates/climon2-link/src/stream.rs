//! The records a link carries after Hello (#480): advertisement on stream 0
//! and the records of session streams. The layouts are those of the
//! protocol's record pages, built by the same mechanical rule as the client
//! interface's records.

use crate::client::{Error, Event, Request};
use crate::layout::{self, Field, Reader, layout, tagged};
use crate::record::{HubClose, RelayClose};
use crate::rules::{Rules, Verdict};
use crate::unit::CONTROL;

/// A session's ID on the relay's machine.
pub type SessionId = [u8; 16];

/// The only client interface revision link revision 1 carries.
const INTERFACE_REVISION: u32 = 1;

layout! {
    /// One session the relay advertises: what discovery found, and nothing
    /// of the relay's machine beyond it. No socket path, process ID or
    /// status crosses the link. The hub monitor treats every field as
    /// hostile.
    pub struct Session {
        pub id: SessionId,
        /// How the command, arguments and working directory are encoded:
        /// the client interface's native path encodings.
        pub encoding: u32,
        pub command: Vec<u8>,
        pub arguments: Vec<Vec<u8>>,
        /// Where the session started. Display text only.
        pub working_directory: Vec<u8>,
        pub depth: u32,
        /// The client interface's UTC timestamp form.
        pub created_at: [u8; 24],
        /// The climon2 release the session runs.
        pub release: String,
    }
}

tagged! {
    /// What the relay tells the hub monitor about its sessions, on stream 0.
    pub enum Advertisement: u8 {
        /// One session, new or changed: it replaces what the hub monitor
        /// held for its ID.
        Session(Session) = 0x10,
        /// A session discovery proved stale, by its ID.
        Withdrawn(SessionId) = 0x11,
        /// Every session the first discovery pass found has been
        /// advertised.
        SnapshotComplete = 0x12,
    }
}

/// What the hub monitor asks for when it opens a session stream. The
/// stream carries client interface revision 1, the only one link revision
/// 1 allows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenStream {
    /// The session, by the ID its advertisement gave.
    pub session: SessionId,
    /// The hub monitor's connection generation for the stream's client.
    pub generation: u64,
    /// The stream's first credit, in bytes.
    pub credit: u32,
}

layout! {
    /// An Open stream body as it crosses the link.
    struct OpenStreamLayout {
        session: SessionId,
        generation: u64,
        interface_revision: u32,
        credit: u32,
    }
}

impl Field for OpenStream {
    fn put(&self, out: &mut Vec<u8>) {
        OpenStreamLayout {
            session: self.session,
            generation: self.generation,
            interface_revision: INTERFACE_REVISION,
            credit: self.credit,
        }
        .put(out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        let open = OpenStreamLayout::take(reader)?;
        (open.interface_revision == INTERFACE_REVISION).then_some(Self {
            session: open.session,
            generation: open.generation,
            credit: open.credit,
        })
    }
}

layout! {
    /// The client started: the capability list of its library's
    /// description, unchanged.
    pub struct StreamOpened {
        pub capabilities: Vec<String>,
    }
}

/// Why the relay could not start a stream's client. Each cause's value is
/// its code on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum Cause {
    /// The session is not advertised.
    NotAdvertised = 1,
    /// The session's version folder is missing.
    VersionMissing = 2,
    /// The client library failed to load.
    LibraryFailed = 3,
    /// The client interface revision is not supported.
    UnsupportedRevision = 4,
    /// The relay is out of resources.
    OutOfResources = 5,
    /// The client failed to start.
    ClientFailed = 6,
}

impl Cause {
    const ALL: [Self; 6] = [
        Self::NotAdvertised,
        Self::VersionMissing,
        Self::LibraryFailed,
        Self::UnsupportedRevision,
        Self::OutOfResources,
        Self::ClientFailed,
    ];
}

/// A cause the receiver does not know breaks the layout.
impl Field for Cause {
    fn put(&self, out: &mut Vec<u8>) {
        (*self as u16).put(out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        let code = u16::take(reader)?;
        Self::ALL.into_iter().find(|cause| *cause as u16 == code)
    }
}

layout! {
    /// The client could not start; the stream has ended.
    pub struct StreamRefused {
        pub cause: Cause,
        /// The library's or loader's error, when there is one.
        pub error: Error,
    }
}

/// Why a stream's client ended. Each reason's value is its code on the
/// wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum EndReason {
    /// The client ended, by itself or after Close stream.
    ClientEnded = 1,
    /// The stream had no credit for too long, and the relay left.
    FellBehind = 2,
    /// The hub monitor broke a rule of the stream.
    StreamViolation = 3,
}

impl EndReason {
    const ALL: [Self; 3] = [Self::ClientEnded, Self::FellBehind, Self::StreamViolation];
}

/// A reason the receiver does not know breaks the layout.
impl Field for EndReason {
    fn put(&self, out: &mut Vec<u8>) {
        (*self as u8).put(out);
    }

    fn take(reader: &mut Reader<'_>) -> Option<Self> {
        let code = u8::take(reader)?;
        Self::ALL.into_iter().find(|reason| *reason as u8 == code)
    }
}

layout! {
    /// The stream's client has ended: why, and its run report, with the
    /// values the client interface defines. Always the stream's last
    /// record.
    pub struct StreamEnded {
        pub reason: EndReason,
        pub workers_cleanup: u32,
        pub generation: u64,
        pub client_outcome: u32,
        pub retry: u32,
        pub leave: u32,
        pub attachment_detached: u8,
        pub connection_closed: u8,
        /// The client's final error, when there is one.
        pub error: Error,
    }
}

layout! {
    /// The client library did not accept a request when the relay
    /// submitted it.
    pub struct RequestRefused {
        pub request_id: u64,
        /// The call result the library returned.
        pub code: u32,
        pub error: Error,
    }
}

tagged! {
    /// What the hub monitor sends on a session stream.
    pub enum HubStream: u8 {
        /// The stream's first record: start a client for one session.
        Open(OpenStream) = 0x20,
        /// Stop the stream's client.
        Close = 0x23,
        /// Grant the stream this many more bytes of events.
        Credit(u32) = 0x25,
        ;
        /// One client interface request.
        Request(Request) in 0x40..=0x7f,
    }
}

tagged! {
    /// What the relay sends on a session stream.
    pub enum RelayStream: u8 {
        /// The client started.
        Opened(StreamOpened) = 0x21,
        /// The client could not start; the stream has ended.
        Refused(StreamRefused) = 0x22,
        /// The client has ended; the stream's last record.
        Ended(StreamEnded) = 0x24,
        /// A request the client library did not accept.
        RequestRefused(RequestRefused) = 0x26,
        ;
        /// One client interface event.
        Event(Event) in 0x80..=0xff,
    }
}

/// One record a hub monitor sends: every one is on a session stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubRecord {
    /// The stream, numbered by the hub monitor from 1.
    pub stream: u32,
    pub record: HubStream,
}

/// One record a relay sends: advertisement on stream 0, or a record on a
/// session stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayRecord {
    Advertisement(Advertisement),
    Stream { stream: u32, record: RelayStream },
}

/// A record one end sends after Hello, as it crosses the link.
pub(crate) trait Spoken: Sized {
    /// The stream it goes on and the record, tag included.
    fn encode(&self) -> (u32, Vec<u8>);

    /// The record that arrived as `record` on `stream`, or nothing when it
    /// breaks its layout or is not one its sender sends there.
    fn decode(stream: u32, record: &[u8]) -> Option<Self>;

    /// What the order rules make of it, when it is `length` bytes long,
    /// tag included.
    fn judge(&self, length: usize, rules: &mut Rules) -> Verdict;
}

impl Spoken for HubRecord {
    fn encode(&self) -> (u32, Vec<u8>) {
        (self.stream, layout::record(&self.record))
    }

    /// Stream 0 carries no record of the hub monitor's but Hello and Close.
    fn decode(stream: u32, record: &[u8]) -> Option<Self> {
        let record = layout::from_record(record).filter(|_| stream != CONTROL)?;
        Some(Self { stream, record })
    }

    fn judge(&self, _: usize, rules: &mut Rules) -> Verdict {
        rules.hub(self)
    }
}

impl Spoken for RelayRecord {
    fn encode(&self) -> (u32, Vec<u8>) {
        match self {
            Self::Advertisement(advertisement) => (CONTROL, layout::record(advertisement)),
            Self::Stream { stream, record } => (*stream, layout::record(record)),
        }
    }

    fn decode(stream: u32, record: &[u8]) -> Option<Self> {
        match stream {
            CONTROL => layout::from_record(record).map(Self::Advertisement),
            _ => layout::from_record(record).map(|record| Self::Stream { stream, record }),
        }
    }

    fn judge(&self, length: usize, rules: &mut Rules) -> Verdict {
        rules.relay(self, length)
    }
}

/// What one end says and hears after Hello. Each end is named by the
/// reasons it closes with.
pub(crate) trait Voice {
    /// The records this end sends.
    type Says: Spoken;
    /// The records the other end sends.
    type Hears: Spoken;
}

impl Voice for HubClose {
    type Says = HubRecord;
    type Hears = RelayRecord;
}

impl Voice for RelayClose {
    type Says = RelayRecord;
    type Hears = HubRecord;
}
