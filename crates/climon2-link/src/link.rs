//! A link connection after its opening exchange: the pinned-key handshake,
//! Hello each way, then records until a Close or a loss. The hub monitor's
//! half and the relay's half differ up to the two Hellos and share the
//! conversation that follows.

use crate::keys::{KEY_SIZE, PublicKey, StaticKey};
use crate::liveness::Liveness;
use crate::noise::{self, Initiator, Transport};
use crate::opening::{Opening, Purpose, Transcript};
use crate::record::{CLOSE, HELLO, HELLO_MOST, Hello, HubClose, Reason, RelayClose};
use crate::rules::{Rules, Verdict};
use crate::stream::{HubRecord, RelayRecord, Spoken, Voice};
use crate::unit::{Allowed, CONTROL, Fault, Received, Units};
use crate::{Input, Output, Randomness, Time};
use std::time::Duration;

/// The size of a handshake initiation behind its length.
const INITIATION_SIZE: usize = 96;
/// The size of a handshake reply behind its length.
const REPLY_SIZE: usize = 48;
/// How long a relay allows from accepting a connection to receiving the hub
/// monitor's Hello, and a hub monitor from dialling to receiving the
/// relay's Hello or Close.
const HELLO_WAIT: Duration = Duration::from_secs(30);
/// The most units an established end sends in one step. An end with more
/// to send names the present moment as its deadline, so its caller steps
/// it again as soon as it has sent these; a record queued meanwhile on
/// stream 0, or on a stream with less to send, goes ahead of the rest.
const UNITS_PER_STEP: usize = 8;

/// The other end of an established link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peer {
    /// The static public key it proved it holds: the one this end pinned.
    pub key: PublicKey,
    /// What it stated in its Hello.
    pub hello: Hello,
}

/// How far one end's link has got. `Ours` is the reason this end may close
/// with and `Theirs` the reason the other end may.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link<Ours, Theirs> {
    /// The link is not established: the opening exchange, the handshake or
    /// the two Hellos are not done. A relay treats nothing as linked here.
    Pending,
    /// Both Hellos have crossed and the link is established.
    Established(Peer),
    /// The connection is over.
    Ended(Ended<Ours, Theirs>),
}

/// How a link connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ended<Ours, Theirs> {
    /// It ended before the link was established and without a Close: the
    /// handshake failed, or the other end was late, hung up or sent a unit
    /// that did not decrypt. For a hub monitor this is a failed dial. A
    /// relay sent nothing in answer to what failed.
    Failed,
    /// This end sent Close with this reason: its caller closed the link,
    /// the relay refused it, or the other end broke the protocol.
    Left(Ours),
    /// The other end sent Close with this reason.
    Closed(Theirs),
    /// The established link ended without a Close: a unit did not decrypt
    /// or the byte stream closed. A lost link.
    Lost,
}

/// A hub monitor's link.
pub type HubLink = Link<HubClose, RelayClose>;
/// A relay's link.
pub type RelayLink = Link<RelayClose, HubClose>;

/// Why an end did not take something to send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The link is not established, or is over.
    NotLinked,
    /// The record is longer than the other end accepts.
    TooLong,
    /// The record breaks an order rule: a stream number out of sequence, a
    /// record on a stream that has not begun, has ended or may not carry it
    /// yet, or an advertisement the relay may not make.
    OutOfTurn,
}

/// What an end does when the other end's Hello arrives.
enum Role<Ours> {
    /// The hub monitor: its own Hello is sent, and the relay's Hello or
    /// Close answers it.
    Hub,
    /// The relay: it answers the hub monitor's Hello with its own Hello,
    /// or with a Close alone that gives `refusal`.
    Relay { refusal: Option<Ours> },
}

/// A link connection from the end of its handshake on.
struct Conversation<Ours: Voice, Theirs> {
    units: Units,
    role: Role<Ours>,
    /// This end's Hello.
    own: Hello,
    /// The other end's static public key.
    key: PublicKey,
    link: Link<Ours, Theirs>,
    /// The order rules every record after Hello keeps, either way.
    rules: Rules,
    /// Records received and not yet taken by the caller.
    inbox: Vec<Ours::Hears>,
    /// When this end last sent a unit and last heard one.
    liveness: Liveness,
}

impl<Ours: Reason + Voice, Theirs: Reason> Conversation<Ours, Theirs> {
    fn new(transport: Transport, role: Role<Ours>, own: Hello, key: PublicKey) -> Self {
        let mut conversation = Self {
            units: Units::new(transport),
            role,
            own,
            key,
            link: Link::Pending,
            rules: Rules::default(),
            inbox: Vec::new(),
            liveness: Liveness::new(),
        };
        if matches!(conversation.role, Role::Hub) {
            conversation.say(conversation.own.encode());
        }
        conversation
    }

    /// Queues one stream-0 record.
    fn say(&mut self, record: Vec<u8>) {
        self.units.send(CONTROL, record);
    }

    /// Ends the connection; the next output closes the byte stream, and
    /// what the session streams have still to send is never sent.
    fn end(&mut self, ended: Ended<Ours, Theirs>) {
        self.link = Link::Ended(ended);
        self.units.drop_streams();
    }

    /// Ends the connection with a Close that gives `reason`.
    fn leave(&mut self, reason: Ours) {
        self.say(reason.encode());
        self.end(Ended::Left(reason));
    }

    /// Ends a connection that the other end cut off without a Close.
    fn cut_off(&mut self) {
        self.end(if self.established() {
            Ended::Lost
        } else {
            Ended::Failed
        });
    }

    fn over(&self) -> bool {
        matches!(self.link, Link::Ended(_))
    }

    /// What the other end may send now. Until the link is established that
    /// is one Hello or Close, on stream 0.
    fn allowed(&self) -> Allowed {
        match self.link {
            Link::Established(_) => Allowed {
                record: self.own.largest_record() as usize,
                streams: true,
            },
            _ => Allowed {
                record: HELLO_MOST,
                streams: false,
            },
        }
    }

    /// Takes one input at `now` and gives what to send. The output closes
    /// the byte stream once the connection is over, after everything stream
    /// 0 has still to send, or at once, sending nothing, when the link is
    /// lost to silence.
    fn step(&mut self, now: Time, input: Input<'_>) -> Output {
        self.take(input);
        if self.units.heard() {
            self.liveness.heard(now);
        }
        if self.silent(now) {
            return silent_close();
        }
        let bytes = self.flush(now);
        Output {
            bytes,
            close: self.over(),
            deadline: self.deadline(now),
        }
    }

    /// Takes one step's input.
    fn take(&mut self, input: Input<'_>) {
        match input {
            Input::Tick => {}
            Input::Closed => self.cut_off(),
            Input::Bytes(bytes) => {
                self.units.receive(bytes);
                self.read();
            }
        }
    }

    /// Whether the established link has heard no unit for too long at
    /// `now`, which ends it as lost.
    fn silent(&mut self, now: Time) -> bool {
        let silent = self.established() && self.liveness.silent(now);
        if silent {
            self.end(Ended::Lost);
        }
        silent
    }

    /// The units to send at `now`: up to 8 of what is queued, everything
    /// stream 0 holds once the connection is over, or a keepalive when an
    /// established end has nothing to send and has sent nothing for long
    /// enough.
    fn flush(&mut self, now: Time) -> Vec<u8> {
        let most = if self.over() {
            usize::MAX
        } else {
            UNITS_PER_STEP
        };
        let mut bytes = self.units.flush(most);
        if bytes.is_empty() && self.established() && self.liveness.keepalive_due(now) {
            bytes = self.units.keepalive_unit();
        }
        if !bytes.is_empty() {
            self.liveness.sent(now);
        }
        bytes
    }

    /// The deadline at `now`: the present moment while units are queued,
    /// and otherwise the next keepalive or the silence limit of an
    /// established link.
    fn deadline(&self, now: Time) -> Option<Time> {
        if self.units.pending() {
            return Some(now);
        }
        self.established().then(|| self.liveness.deadline())
    }

    /// Handles every whole record received, until the connection is over.
    fn read(&mut self) {
        while !self.over() {
            let Some(next) = self.units.next(self.allowed()) else {
                return;
            };
            self.handle(next);
        }
    }

    /// Handles one whole record, or the fault that came in its place.
    fn handle(&mut self, next: Result<Received, Fault>) {
        match next {
            Ok((stream, record)) => self.record(stream, &record),
            Err(Fault::Undecryptable) => self.cut_off(),
            Err(Fault::Violation) => self.leave(Ours::VIOLATION),
        }
    }

    fn established(&self) -> bool {
        matches!(self.link, Link::Established(_))
    }

    fn record(&mut self, stream: u32, record: &[u8]) {
        // A record holds at least its tag: no unit that ends one is empty.
        let (tag, body) = (record[0], &record[1..]);
        match (stream, tag) {
            (CONTROL, HELLO) => self.hello(body),
            (CONTROL, CLOSE) => self.closed(body),
            _ => self.other(stream, record),
        }
    }

    /// Any record but Hello and Close, tag included: the caller's, once the
    /// link is established, when it is one the other end sends on `stream`
    /// and in turn. One that crossed the ending of its stream, or arrived
    /// on a stream this end closed, is dropped. Anything else is a protocol
    /// violation.
    fn other(&mut self, stream: u32, record: &[u8]) {
        let heard = Ours::Hears::decode(stream, record).filter(|_| self.established());
        let Some(heard) = heard else {
            return self.leave(Ours::VIOLATION);
        };
        match heard.judge(&mut self.rules) {
            Verdict::Accept => self.inbox.push(heard),
            Verdict::Broken => self.leave(Ours::VIOLATION),
            Verdict::Crossed | Verdict::Discarded => {}
        }
    }

    /// The other end's Hello arrived. Only the first record may be one.
    fn hello(&mut self, body: &[u8]) {
        let first = !self.established();
        let Some(hello) = Hello::decode(body).filter(|_| first) else {
            return self.leave(Ours::VIOLATION);
        };
        self.answer(hello);
    }

    /// Answers the other end's valid Hello and establishes the link, or
    /// refuses it with Close.
    fn answer(&mut self, hello: Hello) {
        match self.role {
            Role::Relay {
                refusal: Some(reason),
            } => return self.leave(reason),
            Role::Relay { refusal: None } => self.say(self.own.encode()),
            Role::Hub => {}
        }
        self.link = Link::Established(Peer {
            key: self.key,
            hello,
        });
    }

    /// The other end's Close arrived. A relay may send it in place of its
    /// Hello; a hub monitor sends its Hello first.
    fn closed(&mut self, body: &[u8]) {
        let in_turn = self.established() || matches!(self.role, Role::Hub);
        match Theirs::decode(body).filter(|_| in_turn) {
            Some(reason) => self.end(Ended::Closed(reason)),
            None => self.leave(Ours::VIOLATION),
        }
    }

    /// Queues `record`, when the link is established, the other end
    /// accepts a record that long and the record is in turn.
    fn send(&mut self, record: &Ours::Says) -> Result<(), Refused> {
        let Link::Established(peer) = &self.link else {
            return Err(Refused::NotLinked);
        };
        let (stream, bytes) = record.encode();
        if bytes.len() > peer.hello.largest_record() as usize {
            return Err(Refused::TooLong);
        }
        if !record.judge(&mut self.rules).sendable() {
            return Err(Refused::OutOfTurn);
        }
        self.units.send(stream, bytes);
        Ok(())
    }

    fn close(&mut self, reason: Ours) -> Result<(), Refused> {
        if !self.established() {
            return Err(Refused::NotLinked);
        }
        self.leave(reason);
        Ok(())
    }
}

/// A handshake message as it arrives: a length that is always the same,
/// then that many bytes, and nothing behind them until this end has
/// answered.
struct Message<const N: usize>(Vec<u8>);

/// What became of a handshake message after a step.
enum Arrived<'a> {
    /// Part of it is still to come.
    Partial,
    /// All of it is here: the bytes behind the length.
    Whole(&'a [u8]),
    /// It never will be: it states another length, bytes follow it out of
    /// turn, or the byte stream closed.
    Broken,
}

/// The length field of every handshake message.
const LENGTH_SIZE: usize = 2;

impl<const N: usize> Message<N> {
    /// Takes one step's input.
    fn take(&mut self, input: Input<'_>) -> Arrived<'_> {
        match input {
            Input::Bytes(bytes) => self.0.extend(bytes),
            Input::Closed => return Arrived::Broken,
            Input::Tick => {}
        }
        self.arrived()
    }

    fn arrived(&self) -> Arrived<'_> {
        if self.broken() {
            return Arrived::Broken;
        }
        match self.0.len() == LENGTH_SIZE + N {
            true => Arrived::Whole(&self.0[LENGTH_SIZE..]),
            false => Arrived::Partial,
        }
    }

    /// Whether the bytes so far state another length or run past the end.
    fn broken(&self) -> bool {
        let length = (N as u16).to_be_bytes();
        let stated = self.0.len().min(LENGTH_SIZE);
        self.0[..stated] != length[..stated] || self.0.len() > LENGTH_SIZE + N
    }
}

/// A handshake message behind its length.
fn framed(message: &[u8]) -> Vec<u8> {
    let mut bytes = (message.len() as u16).to_be_bytes().to_vec();
    bytes.extend(message);
    bytes
}

/// An output that closes the byte stream and sends nothing.
fn silent_close() -> Output {
    Output {
        close: true,
        ..Output::default()
    }
}

/// One end's run of a link connection, as its deadline sees it.
trait Run {
    /// Whether the link is still to be established.
    fn pending(&self) -> bool;

    /// Ends the connection before any link, sending nothing more.
    fn fail(&mut self) -> Output;
}

/// Steps `run` with `advance`. A link still pending at `deadline` fails
/// there, and each output names the deadline until the link is established.
fn in_time<R: Run>(
    run: &mut R,
    now: Time,
    deadline: Time,
    advance: impl FnOnce(&mut R) -> Output,
) -> Output {
    if !run.pending() {
        return advance(run);
    }
    if now > deadline {
        return run.fail();
    }
    let output = advance(run);
    if !run.pending() {
        return output;
    }
    if now == deadline {
        return run.fail();
    }
    Output {
        deadline: Some(deadline),
        ..output
    }
}

/// An ephemeral private key, drawn from the caller's randomness.
fn ephemeral(randomness: &mut dyn Randomness) -> [u8; KEY_SIZE] {
    let mut key = [0; KEY_SIZE];
    randomness.fill(&mut key);
    key
}

/// What a hub monitor needs to link: its static key for this relay, the
/// relay's pinned key and its own Hello.
pub(crate) struct HubRun {
    own: StaticKey,
    relay: PublicKey,
    hello: Hello,
    stage: HubStage,
    /// The last moment at which the relay's Hello or Close is in time.
    deadline: Time,
    /// The link while no conversation holds it.
    early: HubLink,
}

enum HubStage {
    /// The opening exchange has not chosen a revision.
    Opening,
    /// The initiation is sent and the reply awaited.
    Replying(Box<Initiator>, Message<REPLY_SIZE>),
    Talking(Box<Conversation<HubClose, RelayClose>>),
    Over,
}

impl HubRun {
    pub(crate) fn new(own: StaticKey, relay: PublicKey, hello: Hello) -> Self {
        Self {
            own,
            relay,
            hello,
            stage: HubStage::Opening,
            deadline: Time::since_start(Duration::ZERO),
            early: Link::Pending,
        }
    }

    pub(crate) fn link(&self) -> &HubLink {
        match &self.stage {
            HubStage::Talking(conversation) => &conversation.link,
            _ => &self.early,
        }
    }

    /// Whether the opening exchange is behind this run.
    pub(crate) fn begun(&self) -> bool {
        !matches!(self.stage, HubStage::Opening)
    }

    /// The dial failed: nothing more is sent.
    pub(crate) fn fail(&mut self) -> Output {
        Run::fail(self)
    }

    /// What follows the opening exchange `opening` of a dial made at
    /// `dialled`: the handshake, or nothing when no revision was chosen.
    pub(crate) fn follow(
        &mut self,
        opening: &Opening,
        dialled: Time,
        randomness: &mut dyn Randomness,
    ) -> Output {
        match opening {
            Opening::Opened(opened) => self.begin(&opened.transcript, dialled, randomness),
            _ => self.fail(),
        }
    }

    /// Sends the handshake initiation, bound to the opening exchange that
    /// `transcript` records. The relay's Hello or Close is due 30 seconds
    /// after `dialled`.
    fn begin(
        &mut self,
        transcript: &Transcript,
        dialled: Time,
        randomness: &mut dyn Randomness,
    ) -> Output {
        let deadline = Time::since_start(dialled.elapsed().saturating_add(HELLO_WAIT));
        self.deadline = deadline;
        let begun = Initiator::begin(
            transcript.as_bytes(),
            &self.own,
            &self.relay,
            ephemeral(randomness),
            &[],
        );
        let Some((initiator, initiation)) = begun else {
            return self.fail();
        };
        self.stage = HubStage::Replying(Box::new(initiator), Message(Vec::new()));
        Output {
            bytes: framed(&initiation),
            close: false,
            deadline: Some(deadline),
        }
    }

    pub(crate) fn step(&mut self, now: Time, input: Input<'_>) -> Output {
        in_time(self, now, self.deadline, |run| run.advance(now, input))
    }

    fn advance(&mut self, now: Time, input: Input<'_>) -> Output {
        match std::mem::replace(&mut self.stage, HubStage::Over) {
            HubStage::Replying(initiator, message) => self.replying(initiator, message, now, input),
            HubStage::Talking(mut conversation) => {
                let output = conversation.step(now, input);
                self.stage = HubStage::Talking(conversation);
                output
            }
            stage => {
                self.stage = stage;
                Output::default()
            }
        }
    }

    /// Takes one step's input while the handshake reply is awaited.
    fn replying(
        &mut self,
        initiator: Box<Initiator>,
        mut message: Message<REPLY_SIZE>,
        now: Time,
        input: Input<'_>,
    ) -> Output {
        match message.take(input) {
            Arrived::Whole(reply) => self.finish(*initiator, reply, now),
            Arrived::Broken => self.fail(),
            Arrived::Partial => {
                self.stage = HubStage::Replying(initiator, message);
                Output::default()
            }
        }
    }

    /// Completes the handshake with the relay's reply and sends Hello.
    fn finish(&mut self, initiator: Initiator, reply: &[u8], now: Time) -> Output {
        let Some((_, transport)) = initiator.finish(reply) else {
            return self.fail();
        };
        let mut conversation =
            Conversation::new(transport, Role::Hub, self.hello.clone(), self.relay);
        let output = conversation.step(now, Input::Tick);
        self.stage = HubStage::Talking(Box::new(conversation));
        output
    }

    pub(crate) fn send(&mut self, record: &HubRecord) -> Result<(), Refused> {
        match &mut self.stage {
            HubStage::Talking(conversation) => conversation.send(record),
            _ => Err(Refused::NotLinked),
        }
    }

    pub(crate) fn close(&mut self, reason: HubClose) -> Result<(), Refused> {
        match &mut self.stage {
            HubStage::Talking(conversation) => conversation.close(reason),
            _ => Err(Refused::NotLinked),
        }
    }

    pub(crate) fn records(&mut self) -> Vec<RelayRecord> {
        match &mut self.stage {
            HubStage::Talking(conversation) => std::mem::take(&mut conversation.inbox),
            _ => Vec::new(),
        }
    }
}

impl Run for HubRun {
    fn pending(&self) -> bool {
        matches!(self.link(), Link::Pending) && !matches!(self.stage, HubStage::Over)
    }

    fn fail(&mut self) -> Output {
        self.stage = HubStage::Over;
        self.early = Link::Ended(Ended::Failed);
        silent_close()
    }
}

/// What a relay needs to link: its identity key, the keys of the hub
/// monitors it remembers and its own Hello.
pub(crate) struct RelayRun {
    identity: StaticKey,
    remembered: Vec<PublicKey>,
    hello: Hello,
    stage: RelayStage,
    /// The last moment at which the hub monitor's Hello is in time.
    deadline: Time,
    /// The link while no conversation holds it.
    early: RelayLink,
}

enum RelayStage {
    /// The opening exchange has not chosen a revision.
    Opening,
    /// The initiation is awaited, bound to this opening exchange.
    Initiating(Transcript, Message<INITIATION_SIZE>),
    Talking(Box<Conversation<RelayClose, HubClose>>),
    Over,
}

impl RelayRun {
    pub(crate) fn new(identity: StaticKey, remembered: Vec<PublicKey>, hello: Hello) -> Self {
        Self {
            identity,
            remembered,
            hello,
            stage: RelayStage::Opening,
            deadline: Time::since_start(Duration::ZERO),
            early: Link::Pending,
        }
    }

    pub(crate) fn link(&self) -> &RelayLink {
        match &self.stage {
            RelayStage::Talking(conversation) => &conversation.link,
            _ => &self.early,
        }
    }

    /// Whether the opening exchange is behind this run.
    pub(crate) fn begun(&self) -> bool {
        !matches!(self.stage, RelayStage::Opening)
    }

    /// The connection failed before any link: nothing is sent in answer.
    pub(crate) fn fail(&mut self) -> Output {
        Run::fail(self)
    }

    /// What follows the opening exchange `opening` of a connection
    /// accepted at `accepted`: the wait for the handshake on a link
    /// connection, nothing on a pairing connection, and no link when no
    /// revision was chosen. Gives the deadline the wait names.
    pub(crate) fn follow(&mut self, opening: &Opening, accepted: Time) -> Option<Time> {
        match opening {
            Opening::Opened(opened) if opened.purpose == Purpose::Link => {
                Some(self.begin(opened.transcript.clone(), accepted))
            }
            Opening::Opened(_) => None,
            _ => {
                self.fail();
                None
            }
        }
    }

    /// Starts the wait for the handshake initiation, bound to the opening
    /// exchange that `transcript` records. The hub monitor's Hello is due
    /// 30 seconds after `accepted`.
    fn begin(&mut self, transcript: Transcript, accepted: Time) -> Time {
        self.stage = RelayStage::Initiating(transcript, Message(Vec::new()));
        self.deadline = Time::since_start(accepted.elapsed().saturating_add(HELLO_WAIT));
        self.deadline
    }

    pub(crate) fn step(
        &mut self,
        now: Time,
        input: Input<'_>,
        randomness: &mut dyn Randomness,
    ) -> Output {
        in_time(self, now, self.deadline, |run| {
            run.advance(now, input, randomness)
        })
    }

    fn advance(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        match std::mem::replace(&mut self.stage, RelayStage::Over) {
            RelayStage::Initiating(transcript, message) => {
                self.initiating(transcript, message, input, randomness)
            }
            RelayStage::Talking(mut conversation) => {
                let output = conversation.step(now, input);
                self.stage = RelayStage::Talking(conversation);
                output
            }
            stage => {
                self.stage = stage;
                Output::default()
            }
        }
    }

    /// Takes one step's input while the handshake initiation is awaited.
    fn initiating(
        &mut self,
        transcript: Transcript,
        mut message: Message<INITIATION_SIZE>,
        input: Input<'_>,
        randomness: &mut dyn Randomness,
    ) -> Output {
        match message.take(input) {
            Arrived::Whole(initiation) => self.answer(&transcript, initiation, randomness),
            Arrived::Broken => self.fail(),
            Arrived::Partial => {
                self.stage = RelayStage::Initiating(transcript, message);
                Output::default()
            }
        }
    }

    /// Answers a whole handshake initiation: the reply to any message that
    /// decrypts, whether or not its key is remembered.
    fn answer(
        &mut self,
        transcript: &Transcript,
        initiation: &[u8],
        randomness: &mut dyn Randomness,
    ) -> Output {
        let answered = noise::answer(
            transcript.as_bytes(),
            &self.identity,
            ephemeral(randomness),
            initiation,
            &[],
        );
        let Some((_, answered)) = answered else {
            return self.fail();
        };
        let remembered = self.remembered.contains(&answered.hub);
        let role = Role::Relay {
            refusal: (!remembered).then_some(RelayClose::NotPaired),
        };
        self.stage = RelayStage::Talking(Box::new(Conversation::new(
            answered.transport,
            role,
            self.hello.clone(),
            answered.hub,
        )));
        Output {
            bytes: framed(&answered.reply),
            ..Output::default()
        }
    }

    pub(crate) fn send(&mut self, record: &RelayRecord) -> Result<(), Refused> {
        match &mut self.stage {
            RelayStage::Talking(conversation) => conversation.send(record),
            _ => Err(Refused::NotLinked),
        }
    }

    pub(crate) fn close(&mut self, reason: RelayClose) -> Result<(), Refused> {
        match &mut self.stage {
            RelayStage::Talking(conversation) => conversation.close(reason),
            _ => Err(Refused::NotLinked),
        }
    }

    pub(crate) fn records(&mut self) -> Vec<HubRecord> {
        match &mut self.stage {
            RelayStage::Talking(conversation) => std::mem::take(&mut conversation.inbox),
            _ => Vec::new(),
        }
    }
}

impl Run for RelayRun {
    fn pending(&self) -> bool {
        matches!(self.link(), Link::Pending) && !matches!(self.stage, RelayStage::Over)
    }

    fn fail(&mut self) -> Output {
        self.stage = RelayStage::Over;
        self.early = Link::Ended(Ended::Failed);
        silent_close()
    }
}
