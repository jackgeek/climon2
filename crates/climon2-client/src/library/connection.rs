//! An instance's daemon connection, run on its owner thread by `run`: its
//! one Open (session metadata, the frozen hello, then daemon wire revision
//! 1), the commands on its role (#94), the records the daemon sends, Leave,
//! and shutdown. One poll loop
//! waits on the socket and the instance's waker; submitted work arrives
//! through the instance's request queue and records leave through its event
//! queue, so `submit`, `drain` and `release_delivery` never touch the
//! connection.

use super::{
    failure::{CANCELLED, Failed, Fault, require},
    handshake::{self, Expected, Peer},
    instance::{Descriptor, Next, Shared, Taken},
    link::{self, Link, TIMEOUT},
    records::{Event, Response},
    request::{Act, Command, Open, Operation, Submitted},
    stream::{Departure, Effect, Stream},
};
use climon2_core::{
    protocol::HELLO_TIMEOUT,
    wire::{self, Frame},
};
use std::time::{Duration, Instant};

const INVALID_STATE: Fault = Fault::library(
    "invalid_state",
    "the instance's connection is not in a state that allows this request",
);
const STALE_GENERATION: Fault = Fault::library(
    "stale_generation",
    "the request names another connection generation",
);
const OVERSIZED: Fault = Fault::library(
    "resource_limit",
    "a record from the session daemon exceeds metadata_record_bytes or chunk_bytes",
);
const UNFOCUSED: Fault = Fault::library(
    "unsupported_capability",
    "focus-v1 is not selected on this connection",
);
/// A Display's header: its period and restoration ID.
const DISPLAY_HEADER: u64 = 16;

/// Runs the connection until its stream ends, and returns the Leave
/// evidence. Whatever ends it, the requests it took and the socket it held
/// are no longer counted once it returns or unwinds.
pub(super) fn run(shared: &Shared) -> Departure {
    let mut driver = Driver::new(shared);
    while !driver.ended {
        driver.turn();
    }
    driver.departure
}

/// The open role's socket, counted while it lives, and its frame ceilings.
struct Connection<'a> {
    link: Link,
    peer: Peer,
    _counted: Descriptor<'a>,
}

/// The instance's connection as its owner thread drives it: the protocol
/// state, the open role, deadlines and what waits for room.
struct Driver<'a> {
    shared: &'a Shared,
    /// The requests taken from the queue and not answered yet.
    taken: Taken<'a>,
    stream: Stream,
    connection: Option<Connection<'a>>,
    /// The Open's start deadline, until its Initial metadata arrives.
    start: Option<Instant>,
    /// A record waiting for room in the event allowance; nothing is read
    /// meanwhile.
    parked: Option<Event>,
    shutdown: bool,
    departure: Departure,
    ended: bool,
}

impl<'a> Driver<'a> {
    fn new(shared: &'a Shared) -> Self {
        Self {
            shared,
            taken: Taken::new(shared),
            stream: Stream::new(shared.limits().title_bytes),
            connection: None,
            start: None,
            parked: None,
            shutdown: false,
            departure: Departure::NOT_REQUIRED,
            ended: false,
        }
    }

    fn turn(&mut self) {
        match self.taken.next(self.shutdown, self.ready()) {
            Next::Stop => self.ended = true,
            Next::Shutdown => self.shut_down(),
            Next::Request(request) => self.execute(request),
            Next::Wait => self.wait(),
        }
    }

    /// A Leave or a command executes unless the Open is still under way.
    fn ready(&self) -> bool {
        self.connection.is_none() || self.stream.synchronized()
    }

    /// Ends the stream with a failure.
    fn end(&mut self, failed: Failed) {
        self.shared.fail(failed);
        self.ended = true;
    }

    /// The connection broke: a Leave under way is unconfirmed.
    fn broken(&mut self, fault: Fault) {
        self.lost(self.stream.leaving(), fault);
    }

    fn lost(&mut self, leaving: bool, fault: Fault) {
        if leaving {
            self.departure = Departure::lost(fault.clone());
        }
        self.end(Failed::passive(fault));
    }

    fn shut_down(&mut self) {
        self.shutdown = true;
        self.ended = self.connection.is_none();
        self.depart();
    }

    /// Shutdown's own Leave, once the role is open and no Leave is under way.
    fn depart(&mut self) {
        if self.shutdown && self.stream.synchronized() && !self.stream.leaving() {
            self.send_leave(None);
        }
    }

    fn execute(&mut self, request: Submitted) {
        match request.operation {
            Operation::Open(open) if self.connection.is_none() => self.open(request.id, &open),
            Operation::Open(_) => self.reject(request.id, INVALID_STATE),
            Operation::Leave(generation) => self.leave(request.id, generation),
            Operation::Command(command) => self.command(request.id, command),
        }
    }

    /// Why a request naming `generation` cannot go on this role, if it
    /// cannot: another generation, no role, or a Leave under way.
    fn refusal(&self, generation: u64) -> Option<Fault> {
        let refused = [
            (generation != self.shared.generation(), STALE_GENERATION),
            (
                self.connection.is_none() || self.stream.leaving(),
                INVALID_STATE,
            ),
        ];
        refused
            .into_iter()
            .find_map(|(refused, fault)| refused.then_some(fault))
    }

    /// A command goes on the role unless refused; Focus needs `focus-v1`.
    fn command(&mut self, id: u64, command: Command) {
        let unfocused = matches!(command.act, Act::Focus(..)) && !self.stream.focus();
        let refusal = self
            .refusal(command.generation)
            .or(unfocused.then_some(UNFOCUSED));
        match refusal {
            Some(fault) => self.reject(id, fault),
            None => self.send_command(id, command.act),
        }
    }

    /// Sends a command, answered within `operation_ms`; one that cannot fit
    /// the frame ceilings is never sent.
    fn send_command(&mut self, id: u64, act: Act) {
        let operation_ms = Duration::from_millis(self.shared.limits().operation_ms.into());
        let deadline = Instant::now() + operation_ms;
        let connection = self.connection.as_mut().expect("a command needs a role");
        match self.stream.command(id, act, connection.peer.send, deadline) {
            Ok(frame) => connection.link.send(&frame),
            Err(fault) => self.reject(id, fault),
        }
    }

    fn reject(&mut self, id: u64, fault: Fault) {
        self.taken.answer(Response::rejected(id, fault));
    }

    fn leave(&mut self, id: u64, generation: u64) {
        match self.refusal(generation) {
            Some(fault) => self.reject(id, fault),
            None => self.send_leave(Some(id)),
        }
    }

    /// Sends a Leave, the caller's or shutdown's: nothing calls for a retry
    /// after it, whatever its fate.
    fn send_leave(&mut self, id: Option<u64>) {
        self.shared.conclude(true);
        let leave_ms = Duration::from_millis(self.shared.limits().leave_ms.into());
        let deadline = Instant::now() + leave_ms;
        let sent = self.connection.as_mut().map(|connection| {
            let frame = self.stream.leave(id, connection.peer.send, deadline);
            frame.map(|frame| connection.link.send(&frame))
        });
        if let Some(Err(fault)) = sent {
            self.broken(fault);
        }
    }

    fn open(&mut self, id: u64, open: &Open) {
        let started = Instant::now();
        let start_ms = Duration::from_millis(self.shared.limits().start_ms.into());
        self.start = Some(started + start_ms);
        let deadline = (started + HELLO_TIMEOUT).min(started + start_ms);
        match self.handshake(open, deadline) {
            Ok(_) if self.shared.shutting_down() => self.reject(id, CANCELLED),
            Ok((link, peer)) => self.request(id, open, link, peer),
            Err(failed) => self.refuse(id, failed),
        }
    }

    fn handshake(&self, open: &Open, deadline: Instant) -> Result<(Link, Peer), Failed> {
        let expected = self.expected();
        let socket = handshake::socket(&expected)?;
        handshake::hello(&socket, &expected, open, deadline)
    }

    fn expected(&self) -> Expected<'_> {
        let limits = self.shared.limits();
        Expected {
            session: self.shared.session(),
            metadata: self.shared.metadata_path(),
            bytes: limits.client_bytes,
            wire_frame_bytes: limits.wire_frame_bytes,
        }
    }

    /// The Open never reached the daemon: rejected, and the stream ends.
    fn refuse(&mut self, id: u64, failed: Failed) {
        self.reject(id, failed.fault.clone());
        self.end(failed);
    }

    fn request(&mut self, id: u64, open: &Open, mut link: Link, peer: Peer) {
        let offered = super::families::offered(&peer.advertisement, &open.requested);
        match self.stream.open(id, (open.mode, offered), peer.send) {
            Ok(frame) => {
                link.send(&frame);
                self.connection = Some(Connection {
                    link,
                    peer,
                    _counted: self.shared.connected(),
                });
            }
            Err(fault) => self.refuse(id, Failed::permanent(fault)),
        }
    }

    fn wait(&mut self) {
        self.unpark();
        let reading = self.parked.is_none();
        let link = self
            .connection
            .as_ref()
            .map(|connection| &connection.link)
            .filter(|link| reading || link.writing());
        link::wait(Some(self.shared.waker()), link, self.deadline());
        self.service();
    }

    fn unpark(&mut self) {
        if let Some(event) = self.parked.take() {
            self.parked = self.shared.offer(event);
        }
    }

    /// The earliest deadline that applies: an unanswered request's, or the
    /// Open's until its Initial metadata.
    fn deadline(&self) -> Option<Instant> {
        let start = self.start.filter(|_| !self.stream.synchronized());
        self.stream.deadline().into_iter().chain(start).min()
    }

    fn service(&mut self) {
        self.flush();
        self.read();
        self.expire();
    }

    fn flush(&mut self) {
        let flushed = self
            .connection
            .as_mut()
            .map(|connection| connection.link.flush());
        if let Some(Err(fault)) = flushed {
            self.broken(fault);
        }
    }

    /// Handles every whole frame the socket holds, until a record waits for
    /// room or the stream ends.
    fn read(&mut self) {
        while let Some(frame) = self.frame() {
            self.handle(&frame);
        }
    }

    fn frame(&mut self) -> Option<Frame> {
        let connection = self
            .connection
            .as_mut()
            .filter(|_| !self.ended && self.parked.is_none())?;
        connection
            .link
            .receive(connection.peer.receive)
            .unwrap_or_else(|fault| {
                self.broken(fault);
                None
            })
    }

    fn handle(&mut self, frame: &Frame) {
        super::hook("receive");
        match self.decode(frame) {
            Ok(effect) => {
                self.shared.conclude(effect.concludes());
                self.apply(effect);
            }
            Err(fault) => self.broken(fault),
        }
    }

    /// Every record fits `metadata_record_bytes`, type byte included, and
    /// a Display's bytes `chunk_bytes` (#94); none is truncated into a
    /// healthy one.
    fn decode(&mut self, frame: &Frame) -> Result<Effect, Fault> {
        let limits = self.shared.limits();
        let body = frame.body().len() as u64;
        let fits = if frame.code() == wire::Display::CODE {
            body <= DISPLAY_HEADER + u64::from(limits.chunk_bytes)
        } else {
            body < limits.metadata_record_bytes.into()
        };
        require(fits, OVERSIZED)?;
        self.stream.receive(frame)
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::Record(event) => self.offer(event),
            Effect::Opened(response, opened) => self.opened(response, opened),
            Effect::Answered(response) => self.taken.answer(response),
            Effect::Left(response, departure) => self.left(response, departure),
            Effect::Fatal(fault) => self.broken(fault),
        }
    }

    fn offer(&mut self, event: Event) {
        self.parked = self.shared.offer(event);
        self.depart();
    }

    fn opened(&mut self, response: Response, opened: Result<(), Failed>) {
        self.taken.answer(response);
        if let Err(failed) = opened {
            self.end(failed);
        }
    }

    /// A rejected Leave keeps the registration, and the stream goes on
    /// unless shutdown awaits it; any other answer ends the stream.
    fn left(&mut self, response: Option<Response>, departure: Departure) {
        if let Some(response) = response {
            self.taken.answer(response);
        }
        if !departure.kept() || self.shutdown {
            self.departure = departure;
            self.ended = true;
        }
    }

    fn expire(&mut self) {
        let expired = self
            .deadline()
            .is_some_and(|deadline| link::remaining(deadline).is_none());
        if expired && !self.ended {
            self.timed_out();
        }
    }

    /// The request a deadline bounded reports `timeout`, as indeterminate:
    /// it was sent. The connection then fails.
    fn timed_out(&mut self) {
        let leaving = self.stream.leaving();
        if let Some(id) = self.stream.expire() {
            self.taken.answer(Response::uncertain(id, TIMEOUT));
        }
        self.lost(leaving, TIMEOUT);
    }
}
