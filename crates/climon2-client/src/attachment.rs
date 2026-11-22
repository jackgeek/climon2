//! Ordinary attach (#101) through the executable's own release's client
//! library: one interactive Open that requires `daemon-wire-v1` and asks
//! for `status-v1` alone, optionally (#102), then the commands the bridge
//! decides (Input, Resize, Acquire, Ready, Set viewing, Acknowledge and
//! Leave), offered in order. The library's records are parsed once, here,
//! into the events the bridge acts on; it interprets no key, and the
//! executable keeps the host terminal.

use crate::{
    embedded::{self, Admission, Drained, Embedded, NativeError},
    polling::{interrupted, invalid},
};
use climon2_core::{
    exit::{CLIMON2_ERROR, ExitStatus},
    metadata::SessionMetadata,
    native::{self, RecordData},
    protocol::TerminalSize,
};
use climon2_platform::terminal::Signals;
use std::{
    collections::VecDeque, io, num::NonZeroU32, os::fd::BorrowedFd, path::Path, slice,
    time::Duration,
};

/// The public ID of the instance's one Open; later requests follow it.
const OPEN: u64 = 1;

/// How long one wait for the Open's answer lasts before signals are
/// checked again.
const POLL: Duration = Duration::from_millis(25);

/// The only family ordinary attach requires.
const CORE: &[u8] = b"daemon-wire-v1";

/// The only family ordinary attach asks for optionally, for viewing and
/// acknowledgement (#102). Without it the library refuses both locally,
/// sending nothing.
const STATUS: &[u8] = b"status-v1";

/// The grant reasons of an explicit attach or takeover, whose readiness
/// acknowledges pending attention (#102).
const EXPLICIT: [u32; 2] = [native::ATTACH, native::TAKE_CONTROL];

/// The failures that mean the connection to the session was lost: its
/// transport closed or broke, mid-frame or between frames.
const LOST: [&str; 2] = ["io_error", "truncated_frame"];

/// A control period and the restoration within it, as the daemon named
/// them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Authority {
    period: u64,
    restoration: u64,
}

impl Authority {
    fn native(self) -> native::Authority {
        native::Authority {
            generation: embedded::GENERATION,
            period: self.period,
            restoration_id: self.restoration,
        }
    }
}

/// What the bridge asks of the session.
pub(crate) enum Command {
    /// An act under the authority the daemon named.
    Held(Authority, Held),
    /// The take-control key.
    Acquire,
    /// Detach.
    Leave,
}

/// An act under a named authority.
pub(crate) enum Held {
    Input(Vec<u8>),
    Resize(TerminalSize),
    /// Set viewing (#102).
    Viewing(bool),
    /// A request naming the authority alone.
    Confirm(Confirm),
}

/// The requests that name the authority alone.
#[derive(Clone, Copy)]
pub(crate) enum Confirm {
    Ready,
    /// Acknowledge pending attention (#102).
    Acknowledge,
}

impl Command {
    /// The request, borrowing this command's bytes, with public ID `id`.
    fn request(&self, id: u64) -> native::Request {
        let (tag, args) = self.arguments();
        native::Request {
            tag,
            reserved: 0,
            request_id: id,
            args,
        }
    }

    fn arguments(&self) -> (u32, native::RequestArgs) {
        match self {
            Self::Held(authority, held) => held.arguments(authority.native()),
            Self::Acquire => (native::ACQUIRE, generation()),
            Self::Leave => (native::LEAVE, generation()),
        }
    }
}

impl Held {
    fn arguments(&self, authority: native::Authority) -> (u32, native::RequestArgs) {
        match self {
            Self::Input(bytes) => (native::INPUT, input(authority, bytes)),
            Self::Resize(size) => (native::RESIZE, resize(authority, *size)),
            Self::Viewing(viewing) => (native::VIEWING, flag(authority, *viewing)),
            Self::Confirm(confirm) => (confirm.tag(), native::RequestArgs { authority }),
        }
    }
}

impl Confirm {
    fn tag(self) -> u32 {
        match self {
            Self::Ready => native::READY,
            Self::Acknowledge => native::ACKNOWLEDGE,
        }
    }
}

fn input(authority: native::Authority, input: &[u8]) -> native::RequestArgs {
    native::RequestArgs {
        input: native::Input {
            authority,
            bytes: bytes(input),
        },
    }
}

fn resize(authority: native::Authority, size: TerminalSize) -> native::RequestArgs {
    native::RequestArgs {
        resize: native::Resize {
            authority,
            geometry: geometry(size),
        },
    }
}

fn flag(authority: native::Authority, value: bool) -> native::RequestArgs {
    native::RequestArgs {
        flag: native::Flag {
            authority,
            value: value.into(),
            reserved: [0; 7],
        },
    }
}

fn generation() -> native::RequestArgs {
    native::RequestArgs {
        generation: embedded::GENERATION,
    }
}

fn geometry(size: TerminalSize) -> native::Geometry {
    native::Geometry {
        cols: size.cols.into(),
        rows: size.rows.into(),
    }
}

fn bytes(bytes: &[u8]) -> native::Bytes {
    native::Bytes {
        data: bytes.as_ptr(),
        length: bytes.len() as u64,
    }
}

/// How the instance's stream ended.
pub(crate) enum Ending {
    /// A Leave detached the attachment.
    Detached,
    Stopped,
    Failed(io::Error),
}

impl Ending {
    /// Why the stream ended without a detach or a known result.
    pub(crate) fn error(self) -> io::Error {
        match self {
            Self::Failed(error) => error,
            _ => invalid("session result not confirmed"),
        }
    }
}

/// What a record tells the bridge.
pub(crate) enum Event {
    /// The Open succeeded: the attachment is admitted.
    Opened,
    /// The Open was refused: its error.
    Refused(io::Error),
    /// The command's status, from its final result.
    Known(u32),
    /// Control granted under this authority, by an explicit attach or
    /// takeover or not.
    Granted {
        authority: Authority,
        explicit: bool,
    },
    /// A restoration begun under this authority.
    Restoring(Authority),
    Display(Vec<u8>),
    /// The restoration under this authority is complete.
    Restored(Authority),
    /// Control revoked; `ending` when the session is ending.
    Revoked {
        ending: bool,
    },
    Ended(Ending),
}

/// One record's event, if it carries one the bridge needs. Each parser reads
/// the member its tag makes active.
fn event(record: &native::Record) -> Option<Event> {
    PARSERS
        .iter()
        .find(|(tag, _)| *tag == record.tag)
        // SAFETY: the record's tag names its active member, whose buffers
        // live as long as the record.
        .and_then(|(_, parse)| unsafe { parse(&record.data) })
}

type Parser = unsafe fn(&RecordData) -> Option<Event>;

const PARSERS: [(u32, Parser); 8] = [
    (native::RESPONSE, response),
    (native::INITIAL_METADATA, metadata),
    (native::FINAL, final_result),
    (native::GRANTED, granted),
    (native::RESTORATION_BEGIN, restoration),
    (native::DISPLAY, display),
    (native::RESTORATION_COMPLETE, restored),
    (native::REVOKED, revoked),
];

/// The Open's answer; other requests' answers change nothing the bridge
/// shows.
unsafe fn response(data: &RecordData) -> Option<Event> {
    // SAFETY (each): guaranteed by the caller.
    let response = unsafe { &data.response };
    (response.request_id == OPEN).then(|| match response.outcome {
        native::OK => Event::Opened,
        _ => Event::Refused(io::Error::other(
            unsafe { NativeError::copy(&response.error) }.message,
        )),
    })
}

unsafe fn metadata(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller.
    let command = unsafe { &data.metadata.command_result };
    (command.present == 1).then(|| Event::Known(status(&command.value)))
}

unsafe fn final_result(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller.
    Some(Event::Known(status(unsafe {
        &data.r#final.command_result
    })))
}

unsafe fn granted(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller.
    let granted = unsafe { &data.granted };
    Some(Event::Granted {
        authority: Authority {
            period: granted.period,
            restoration: granted.restoration_id,
        },
        explicit: EXPLICIT.contains(&granted.reason),
    })
}

unsafe fn restoration(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller.
    let restoration = unsafe { &data.restoration };
    Some(Event::Restoring(Authority {
        period: restoration.period,
        restoration: restoration.restoration_id,
    }))
}

unsafe fn display(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller; a Display's bytes are never empty.
    let display = unsafe { &data.display.bytes };
    let bytes = unsafe { slice::from_raw_parts(display.data, display.length as usize) };
    Some(Event::Display(bytes.to_vec()))
}

unsafe fn restored(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller.
    let barrier = unsafe { &data.barrier };
    Some(Event::Restored(Authority {
        period: barrier.period,
        restoration: barrier.restoration_id,
    }))
}

unsafe fn revoked(data: &RecordData) -> Option<Event> {
    // SAFETY: guaranteed by the caller.
    let reason = unsafe { data.revoked.reason };
    Some(Event::Revoked {
        ending: reason == native::SESSION_ENDING,
    })
}

/// Today's status for a command result: its exit code, or 128 plus its
/// signal's number.
fn status(result: &native::CommandResult) -> u32 {
    let status = match (result.kind, NonZeroU32::new(result.value)) {
        (native::SIGNAL, Some(signal)) => ExitStatus::Signal(signal),
        _ => ExitStatus::Command(result.value),
    };
    status.code().unwrap_or(CLIMON2_ERROR)
}

fn ending(ended: embedded::Ended) -> Ending {
    match (ended.detached, ended.failure) {
        (true, _) => Ending::Detached,
        (false, None) => Ending::Stopped,
        (false, Some(failure)) => Ending::Failed(lost(failure)),
    }
}

/// A lost connection is unexpected EOF, which names the session when
/// reported; any other failure keeps the library's or the daemon's text.
fn lost(failure: NativeError) -> io::Error {
    let lost = failure.origin == native::ORIGIN_TRANSPORT && LOST.contains(&failure.code.as_str());
    let kind = if lost {
        io::ErrorKind::UnexpectedEof
    } else {
        io::ErrorKind::Other
    };
    io::Error::new(kind, failure.message)
}

/// The executable's attachment: its library instance, the commands not
/// accepted yet, and the public ID of the next one.
pub(crate) struct Attachment {
    instance: Embedded,
    queue: VecDeque<Command>,
    next: u64,
}

impl Attachment {
    /// Opens an interactive attachment at `size` to the session `metadata`
    /// describes. A library that is missing, of another
    /// release or faulty fails here, before any connection.
    pub(crate) fn open(
        metadata: &SessionMetadata,
        session: &embedded::Session,
        home: &Path,
        size: TerminalSize,
    ) -> io::Result<Self> {
        let instance = embedded::session_instance(metadata, session, home)?;
        let (required, optional) = ([bytes(CORE)], [bytes(STATUS)]);
        instance.submit(&open(size, &required, &optional))?;
        Ok(Self {
            instance,
            queue: VecDeque::new(),
            next: OPEN + 1,
        })
    }

    /// Waits for the Open's answer, returning what followed its success.
    /// A refusal, or an end before any answer, is the error; HUP, TERM or
    /// INT meanwhile interrupts.
    pub(crate) fn opened(&self, signals: &Signals) -> io::Result<Vec<Event>> {
        loop {
            if let Some(events) = self.answer(signals)? {
                return Ok(events);
            }
        }
    }

    /// One round of waiting for the Open's answer: until the instance has
    /// something or `POLL` pass, then one delivery.
    fn answer(&self, signals: &Signals) -> io::Result<Option<Vec<Event>>> {
        interrupted(signals)?;
        self.instance.wait(POLL)?;
        answered(self.receive()?)
    }

    /// The descriptor that is readable while there is something to receive.
    pub(crate) fn descriptor(&self) -> BorrowedFd<'_> {
        // SAFETY: the library keeps the notification descriptor open while
        // the instance's communication handle lives, as long as `self`.
        unsafe { BorrowedFd::borrow_raw(self.instance.notification()) }
    }

    /// One delivery's events, none, or the end of the stream.
    pub(crate) fn receive(&self) -> io::Result<Vec<Event>> {
        let mut events = Vec::new();
        let drained = self
            .instance
            .drain_once(&mut |record| events.extend(event(record)))?;
        if let Drained::Ended(ended) = drained {
            events.push(Event::Ended(ending(ended)));
        }
        Ok(events)
    }

    pub(crate) fn queue(&mut self, command: Command) {
        self.queue.push_back(command);
    }

    /// Whether every command was accepted.
    pub(crate) fn idle(&self) -> bool {
        self.queue.is_empty()
    }

    /// Offers the waiting commands in order until one finds no room. True
    /// once the stream has closed: nothing more will be accepted.
    pub(crate) fn send(&mut self) -> io::Result<bool> {
        loop {
            match self.offer()? {
                Some(Admission::Accepted) => self.accepted(),
                admission => return Ok(admission == Some(Admission::Closed)),
            }
        }
    }

    /// Offers the first waiting command, if any.
    fn offer(&self) -> io::Result<Option<Admission>> {
        let request = self.queue.front().map(|command| command.request(self.next));
        request
            .map(|request| self.instance.submit(&request))
            .transpose()
    }

    /// A rejected offer never consumes its ID.
    fn accepted(&mut self) {
        self.queue.pop_front();
        self.next += 1;
    }
}

/// The interactive Open at `size`, requiring `required` and asking for
/// `optional`.
fn open(
    size: TerminalSize,
    required: &[native::Bytes],
    optional: &[native::Bytes],
) -> native::Request {
    native::Request {
        tag: native::OPEN,
        reserved: 0,
        request_id: OPEN,
        args: native::RequestArgs {
            open: native::Open {
                mode: native::INTERACTIVE,
                reserved: 0,
                geometry: geometry(size),
                required: tokens(required),
                optional: tokens(optional),
            },
        },
    }
}

fn tokens(tokens: &[native::Bytes]) -> native::Tokens {
    native::Tokens {
        data: tokens.as_ptr(),
        count: tokens.len() as u64,
    }
}

/// The events after the Open's success, `None` while it is unanswered, or
/// why it failed. Its answer or the end of the stream comes first.
fn answered(events: Vec<Event>) -> io::Result<Option<Vec<Event>>> {
    let mut events = events.into_iter();
    match events.next() {
        None => Ok(None),
        Some(Event::Opened) => Ok(Some(events.collect())),
        Some(other) => Err(refusal(other)),
    }
}

fn refusal(event: Event) -> io::Error {
    match event {
        Event::Refused(error) => error,
        Event::Ended(ending) => ending.error(),
        _ => invalid("unexpected record before the Open's answer"),
    }
}
