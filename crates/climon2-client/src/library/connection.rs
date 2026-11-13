//! An instance's daemon connection, run on its owner thread by `run`: its
//! one Open (session metadata, the frozen hello, then daemon wire revision
//! 1), the records the daemon sends, Leave, and shutdown. One poll loop
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
    request::{Open, Operation, Submitted},
    stream::{Departure, Effect, Stream},
};
use climon2_core::{protocol::HELLO_TIMEOUT, wire::Frame};
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
    "a record from the session daemon exceeds metadata_record_bytes",
);

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
    /// The deadline of an unanswered Leave.
    leave: Option<Instant>,
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
            leave: None,
            parked: None,
            shutdown: false,
            departure: Departure::NOT_REQUIRED,
            ended: false,
        }
    }

    fn turn(&mut self) {
        match self.taken.next(self.shutdown, self.leave_ready()) {
            Next::Stop => self.ended = true,
            Next::Shutdown => self.shut_down(),
            Next::Request(request) => self.execute(request),
            Next::Wait => self.wait(),
        }
    }

    /// A Leave executes unless the Open is still under way.
    fn leave_ready(&self) -> bool {
        self.connection.is_none() || self.stream.synchronized()
    }

    /// Ends the stream with a failure.
    fn end(&mut self, failed: Failed) {
        self.shared.fail(failed);
        self.ended = true;
    }

    /// The connection broke: a Leave under way is unconfirmed.
    fn broken(&mut self, fault: Fault) {
        if self.leave.is_some() {
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
        }
    }

    fn reject(&mut self, id: u64, fault: Fault) {
        self.taken.answer(Response::rejected(id, fault));
    }

    fn leave(&mut self, id: u64, generation: u64) {
        let refused = [
            (generation != self.shared.generation(), STALE_GENERATION),
            (
                self.connection.is_none() || self.stream.leaving(),
                INVALID_STATE,
            ),
        ];
        match refused.into_iter().find(|(refused, _)| *refused) {
            Some((_, fault)) => self.reject(id, fault),
            None => self.send_leave(Some(id)),
        }
    }

    /// Sends a Leave, the caller's or shutdown's: nothing calls for a retry
    /// after it, whatever its fate.
    fn send_leave(&mut self, id: Option<u64>) {
        self.shared.conclude(true);
        let leave_ms = Duration::from_millis(self.shared.limits().leave_ms.into());
        self.leave = Some(Instant::now() + leave_ms);
        let sent = self.connection.as_mut().map(|connection| {
            let frame = self.stream.leave(id, connection.peer.send);
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
        let offered = super::families::offered(&peer.advertisement, &open.optional);
        match self.stream.open(id, open, offered, peer.send) {
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

    /// The deadline that applies: an unanswered Leave's, else the Open's
    /// until its Initial metadata.
    fn deadline(&self) -> Option<Instant> {
        self.leave
            .or(self.start.filter(|_| !self.stream.synchronized()))
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

    /// Every record fits `metadata_record_bytes`, type byte included; none
    /// is truncated into a healthy one.
    fn decode(&mut self, frame: &Frame) -> Result<Effect, Fault> {
        let length = frame.body().len() as u64 + 1;
        let limit = self.shared.limits().metadata_record_bytes.into();
        require(length <= limit, OVERSIZED)?;
        self.stream.receive(frame)
    }

    fn apply(&mut self, effect: Effect) {
        match effect {
            Effect::Record(event) => self.offer(event),
            Effect::Opened(response, opened) => self.opened(response, opened),
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
        self.leave = None;
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
        if let Some(id) = self.stream.expire() {
            self.taken.answer(Response::uncertain(id, TIMEOUT));
        }
        self.broken(TIMEOUT);
    }
}
