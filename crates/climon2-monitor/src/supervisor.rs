//! The basic client supervisor (spec #73, "Client supervisor"; #106): one
//! passive monitoring client per live session, matched by full session ID.
//!
//! Its own thread loads each release on a worker thread through the shared
//! validated loader, one attempt per release and at most `scheduling.loads`
//! at once, and starts clients newest created first, then ascending session
//! ID, at most `scheduling.starts` at once. Each instance has a dedicated
//! owner thread (client embedding's `owner`) that creates it, submits
//! monitor Open, publishes its handles and runs it. The supervisor polls the borrowed
//! notification descriptors and drains every live instance each tick,
//! releasing each delivery at once (`client.rs`), and reports what the
//! registry needs. An instance that fails (its notification lookup, a drain,
//! or a run that returns without terminal evidence) is reported Unavailable,
//! asked to stop and drained until it is over; its handles are then released,
//! freeing its start slot and letting its owner destroy it.
//!
//! Recovery (#116) is passive. A client that is over, or a session whose
//! client could not start, is retried as its terminal evidence says (or as
//! the monitor says when it stopped the client itself, or its release or
//! create was refused): a recoverable failure only once its old instance's
//! handles are released, its run has returned and its owner has destroyed it
//! and been joined, after a backoff (`backoff.rs`), as a fresh instance in a
//! fresh connection generation. Nothing it held is replayed. A permanent
//! rejection, a known final outcome, a session discovery proved gone and
//! shutdown retry nothing.
//!
//! Admission and fair scheduling (#120): every load and every create first
//! reserves what it needs (client embedding's `admission`); a session that
//! cannot be admitted, or whose release cannot be, waits resource-blocked and is
//! reconsidered at most once per `retry.admission_ms`. Waiting sessions
//! start fairly (`queue.rs`): first starts newest first with the oldest
//! getting a turn after each burst of newer ones, retries taking turns with
//! them. A load still running after `deadlines.load_ms` is reported as
//! loading slowly, never aborted (owner decision on #120).
//!
//! A confirmed Delete (#117) submits one Kill through the session's open
//! client, or through a Kill-only instance when it has none, admitted and
//! started ahead of every waiting client; a final or gone session needs
//! none. The session is reported removed only once its end is known (or it
//! needs no Kill) and no instance of it is left: each was asked to stop,
//! its handles released and its owner joined. A refused Kill ends the
//! Delete; a lost or failed one is never sent again, and only a passive
//! client learns its outcome.
mod backlog;
mod backoff;
mod client;
mod queue;

use crate::registry::{Answer, Final, Outcome, Report, Termination, Verdict};
use crate::{
    machine::SessionAt,
    registry::ClientEvent,
    shutdown::{self, Evidence},
    terminal::{Presentation, Request},
};
pub(crate) use backlog::footprint;
use client::recovery;
use climon2_core::{
    SessionId, VersionName,
    config::{Admission, Backoff, Lanes},
    keys::ControlKey,
    layout::HomeLayout,
    metadata::SessionMetadata,
    native,
};
use climon2_embed::load::{Load, Loaded, Refusal};
use climon2_embed::{
    admission::{self, Capacity, Holder, Ledger},
    backlog::Backlog,
    instance::{Drain, Instance as _, KILL_ID, Local, Retry, Stop},
    owner::{self, Environment, Owner, Role, Signal, Startup},
    stack,
    stall::Stalls,
    stream::Stream,
};
#[cfg(test)]
use climon2_platform::library::Failure;
use climon2_platform::{
    library::Functions,
    notifier, process,
    waiting::{self, Watch},
};
use queue::{Condition, Entry, Lane, Queue};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long one wait on the notification descriptors lasts before commands
/// are read again.
const TICK: Duration = Duration::from_millis(20);

/// A local instance whose deliveries the supervisor reads as reports.
type Client = Local<ClientEvent>;

/// Where the monitor's own identifiers start (LIM-05): each session's
/// connection generations at 1, and each instance's request IDs right after
/// Open's. Test hooks start them near exhaustion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Identifiers {
    pub(crate) generation: u64,
    pub(crate) request: u64,
}

impl Default for Identifiers {
    fn default() -> Self {
        Self {
            generation: 1,
            request: client::OPEN_ID + 1,
        }
    }
}

/// What every instance's startup shares.
pub(crate) struct Context {
    pub(crate) load: Load,
    pub(crate) home: HomeLayout,
    pub(crate) runtime: PathBuf,
    pub(crate) escape: ControlKey,
    pub(crate) take_control: Option<ControlKey>,
    pub(crate) limits: native::Limits,
    pub(crate) filter: String,
    pub(crate) lanes: Lanes,
    pub(crate) shutdown: Duration,
    pub(crate) retry: Backoff,
    pub(crate) first: Identifiers,
    /// What admission reserves, and against what.
    pub(crate) admission: Admission,
    /// The bytes of `memory.total_bytes` the monitor holds outside
    /// admission: its one parser.
    pub(crate) outside: u64,
}

enum Command {
    Start(Box<SessionMetadata>),
    Request(SessionId, Request),
    Retire(Retirement),
    /// Shut down by this deadline.
    Shutdown(Instant),
}

/// Why a session's clients come to an end.
enum Retirement {
    /// Discovery proved its daemon gone: it is never retried.
    Gone(SessionId),
    /// A confirmed Delete, with whether it needs a Kill.
    Delete(Box<SessionMetadata>, bool),
    /// Its terminal model refused an update (#119): its open client fails
    /// with why, and is retried passively.
    Dropped(SessionId, String),
}

/// What the supervisor thread shares with the UI thread: its reports,
/// their backlog (#119), the bytes admission holds (#120) and which
/// sessions shutdown still waits for (#118).
struct Shared {
    report: Sender<(SessionId, ClientEvent)>,
    backlog: Arc<Backlog>,
    reserved: Arc<AtomicU64>,
    waits: Sender<Vec<SessionId>>,
}

/// The running supervisor, as the UI thread holds it.
pub(crate) struct Supervisor {
    commands: Sender<Command>,
    pub(crate) reports: Receiver<(SessionId, ClientEvent)>,
    /// What each client's reports not yet processed hold (#119).
    backlog: Arc<Backlog>,
    /// The bytes every admitted reservation holds (#120).
    reserved: Arc<AtomicU64>,
    /// The sessions shutdown still waits for, each round.
    waiting: Receiver<Vec<SessionId>>,
    /// `deadlines.shutdown_ms`.
    shutdown: Duration,
    thread: Option<JoinHandle<shutdown::Report>>,
}

impl Supervisor {
    pub(crate) fn start(context: Context, dispatch: tracing::Dispatch) -> std::io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let (report, reports) = mpsc::channel();
        let (waits, waiting) = mpsc::channel();
        let shutdown = context.shutdown;
        let backlog = Arc::new(Backlog::default());
        let reserved = Arc::new(AtomicU64::new(0));
        let shared = Shared {
            report,
            backlog: Arc::clone(&backlog),
            reserved: Arc::clone(&reserved),
            waits,
        };
        let thread = thread::Builder::new()
            .name("supervisor".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    State::new(context, shared).serve(&received)
                })
            })?;
        Ok(Self {
            commands,
            reports,
            backlog,
            reserved,
            waiting,
            shutdown,
            thread: Some(thread),
        })
    }

    /// The bytes of `memory.total_bytes` that admitted reservations hold:
    /// releases, load threads and instances (#120).
    pub(crate) fn reserved(&self) -> u64 {
        self.reserved.load(Ordering::Relaxed)
    }

    /// The bytes every client's reports not yet processed hold together.
    pub(crate) fn backlog(&self) -> usize {
        self.backlog.total()
    }

    /// The UI processed a report of `id`'s that held `bytes`
    /// ([`footprint`]): its client may be drained into them again.
    pub(crate) fn processed(&self, id: &SessionId, bytes: usize) {
        self.backlog.release(id, bytes);
    }

    /// Asks for a passive client for `metadata`'s session.
    pub(crate) fn start_client(&self, metadata: SessionMetadata) {
        let _ = self.commands.send(Command::Start(Box::new(metadata)));
    }

    /// Submits `request` through `id`'s client. Its answer, or why it could
    /// not be submitted, is reported as [`Presentation::Answered`].
    pub(crate) fn request(&self, id: SessionId, request: Request) {
        let _ = self.commands.send(Command::Request(id, request));
    }

    /// Submits `request` to `id`'s client when it is a local session's. A
    /// remote session's client is reached through its session stream
    /// (#503), never through this supervisor.
    pub(crate) fn route(&self, id: &SessionAt, request: Request) {
        if let Some(local) = id.local_id() {
            self.request(local.clone(), request);
        }
    }

    /// Discovery proved `id`'s daemon gone: nothing retries it any more
    /// (owner decision on #116, 2026-11-16).
    pub(crate) fn forget(&self, id: SessionId) {
        let _ = self.commands.send(Command::Retire(Retirement::Gone(id)));
    }

    /// A confirmed Delete of `metadata`'s session (#117): `kill` it first
    /// unless it is final or gone. [`ClientEvent::Removed`] reports its
    /// end; [`ClientEvent::Termination`] a Kill that did not end it.
    pub(crate) fn remove(&self, metadata: SessionMetadata, kill: bool) {
        let deleted = Retirement::Delete(Box::new(metadata), kill);
        let _ = self.commands.send(Command::Retire(deleted));
    }

    /// `id`'s terminal model refused an update (#119): its open client is
    /// failed with `why` and retried passively, like any client the monitor
    /// stops itself (owner decision on #119, 2026-11-16).
    pub(crate) fn drop_client(&self, id: SessionId, why: String) {
        let dropped = Retirement::Dropped(id, why);
        let _ = self.commands.send(Command::Retire(dropped));
    }

    /// Begins the coordinated shutdown (#118), which must end by the
    /// deadline returned, `deadlines.shutdown_ms` from now: retries and
    /// starts stop, accepted input is answered (at most
    /// `deadlines.operation_ms`, inside the same deadline; owner decision on
    /// #118), then every instance is asked to shut down, concurrently, and
    /// drained until it has ended, its handles are released and its owner
    /// is joined. Instances unconfirmed by the deadline are left, never
    /// destroyed, to the fail-stop.
    pub(crate) fn shut_down(&self) -> Instant {
        let deadline = Instant::now() + self.shutdown;
        let _ = self.commands.send(Command::Shutdown(deadline));
        deadline
    }

    /// The sessions shutdown last said it waits for, if it said anything
    /// since this was last asked.
    pub(crate) fn waiting(&self) -> Option<Vec<SessionId>> {
        self.waiting.try_iter().last()
    }

    /// What the shutdown proved, once the supervisor thread has finished
    /// and is joined; none while it runs, or once taken.
    pub(crate) fn report(&mut self) -> Option<shutdown::Report> {
        let thread = self.thread.take_if(|thread| thread.is_finished())?;
        Some(thread.join().unwrap_or_default())
    }
}

impl Context {
    /// What every instance's startup shares.
    fn environment(&self) -> Environment<'_> {
        Environment {
            home: &self.home,
            runtime: &self.runtime,
            escape: self.escape,
            take_control: self.take_control,
            limits: self.limits,
            filter: &self.filter,
        }
    }
}

/// What owner and load threads tell the supervisor.
enum Internal {
    Loaded(VersionName, Result<Loaded, Refusal>),
    /// The handles exist, with the first request's refusal, if refused.
    Published(SessionId, Client, Option<String>),
    /// Run returned, or create failed, with why if it failed and whether
    /// that is retried, and what its report says of its workers.
    Stopped(SessionId, Result<(), (String, Retry)>, Result<(), String>),
    /// The owner is done, with whether it destroyed the instance.
    OwnerDone(SessionId, Result<(), String>),
}

impl Internal {
    /// What `id`'s owner thread signalled.
    fn of(id: SessionId, signal: Signal<ClientEvent>) -> Self {
        match signal {
            Signal::Published(client, refused) => Self::Published(id, client, refused),
            Signal::Stopped(ran, workers) => Self::Stopped(id, ran, workers),
            Signal::Done(destroyed) => Self::OwnerDone(id, destroyed),
        }
    }
}

/// Where a release stands.
enum Release {
    /// Admission could not reserve it: its configured reservation before
    /// its load, or, once `library` is loaded (its handle stays), its
    /// qualified one. Reconsidered from `until`.
    Blocked {
        why: String,
        until: Instant,
        library: Option<Loaded>,
    },
    /// Loading since then; `slow` once that has lasted
    /// `deadlines.load_ms`.
    Loading {
        since: Instant,
        slow: bool,
    },
    /// Loaded and admitted at its qualified reservation, with what each of
    /// its instances reserves.
    Ready {
        functions: &'static Functions,
        instance: admission::Instance,
    },
    Refused(Refusal),
}

/// A live instance: its handles and, once looked up, its descriptor.
#[derive(Clone, Copy)]
struct Live {
    client: Client,
    descriptor: Option<i32>,
}

/// Where an instance is in its life. Only `Creating` and `Opening` hold a
/// start slot.
#[derive(Clone, Copy)]
enum Phase {
    Creating,
    Opening(Live),
    Open(Live),
    /// Reported as over; asked to stop and drained until its stream ends.
    Closing(Live),
    /// Its handles are released (or there were none); waiting for its owner.
    Done,
}

impl Phase {
    fn holds_slot(self) -> bool {
        matches!(self, Self::Creating | Self::Opening(_))
    }

    fn live(self) -> Option<Live> {
        match self {
            Self::Opening(live) | Self::Open(live) | Self::Closing(live) => Some(live),
            Self::Creating | Self::Done => None,
        }
    }

    /// Whether the row still follows this instance's reports.
    fn reporting(self) -> bool {
        matches!(self, Self::Opening(_) | Self::Open(_))
    }

    /// The phase after a report: Open's success settles an opening instance.
    fn after(self, event: &ClientEvent) -> Self {
        match (self, event) {
            (Self::Opening(live), ClientEvent::Opened { .. }) => Self::Open(live),
            (phase, _) => phase,
        }
    }

    /// What the row learns when the instance is over with `why`: an opened
    /// client ended, any other one is Unavailable; a closing one was
    /// already reported.
    fn over(self, why: String) -> Option<ClientEvent> {
        match self {
            Self::Open(_) => Some(ClientEvent::Ended),
            Self::Creating | Self::Opening(_) => Some(ClientEvent::Unavailable(why)),
            Self::Closing(_) | Self::Done => None,
        }
    }
}

/// An accepted request awaiting its answer.
enum Pending {
    Request(Request),
    /// A confirmed Delete's Kill, answered to the Delete, never a terminal.
    Kill,
}

/// A confirmed Delete under way (#117).
struct Removal {
    metadata: SessionMetadata,
    kill: Kill,
    /// The command's end is known, from a Kill's answer or a report.
    ended: bool,
    /// A start or retry was waiting when the Delete began: it waits again
    /// if the Kill is refused.
    resume: bool,
}

impl Removal {
    /// Whether the session needs nothing more but its cleanup: its end is
    /// known, or it needed no Kill.
    fn over(&self) -> bool {
        self.ended || self.kill == Kill::Skipped
    }
}

/// Where a confirmed Delete's Kill stands.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kill {
    /// None is needed: the session is final or gone.
    Skipped,
    /// Waiting for an instance to submit it through.
    Wanted,
    Pending,
    Answered,
    /// Refused without effect: the Delete ends once no instance is left.
    Refused,
    /// Failed or lost: never sent again; a passive client learns the
    /// outcome.
    Unconfirmed,
}

struct Instance {
    metadata: SessionMetadata,
    role: Role,
    phase: Phase,
    /// Whether it is retried, decided once it is over.
    retry: Option<Retry>,
    /// The command's final outcome came through it: nothing retries it.
    finished: bool,
    /// Accepted requests awaiting their answers, by request ID: at most
    /// [`State::ledger`].
    pending: BTreeMap<u64, Pending>,
    /// The next request ID (Open is 1), none once they are exhausted: they
    /// never wrap.
    next: Option<u64>,
    /// Its application input: chunks, pastes and what a truncated paste
    /// owes.
    stream: Stream,
    /// Its owner thread, told once its handles are released, so it may
    /// destroy.
    owner: Owner,
    /// What is known of its end, each kind of evidence on its own (#118).
    evidence: Evidence,
}

struct State {
    context: Arc<Context>,
    report: Sender<(SessionId, ClientEvent)>,
    /// What each client's reports the UI has not processed hold: a client
    /// is drained only while they fit its `queues.event_bytes`.
    backlog: Arc<Backlog>,
    /// Every admitted reservation (#120), and its bytes for the UI.
    admitted: Ledger,
    reserved: Arc<AtomicU64>,
    /// Since when each congested client has not been drained.
    congested: Stalls,
    /// Shutdown has begun: the UI processes nothing more, and every client
    /// is drained regardless, so shutdown stays serviceable.
    stopping: bool,
    /// How many polls there have been: where the next round's drains start.
    turn: usize,
    internal: Sender<Internal>,
    internals: Receiver<Internal>,
    /// Sessions waiting for a client, in their lanes.
    queue: Queue,
    releases: BTreeMap<VersionName, Release>,
    instances: BTreeMap<SessionId, Instance>,
    /// Sessions whose next attempt waits for its backoff to pass.
    retrying: BTreeMap<SessionId, (Instant, SessionMetadata)>,
    /// Each session's failed attempts since its client last opened.
    attempts: BTreeMap<SessionId, u32>,
    /// Each session's latest connection generation.
    generations: BTreeMap<SessionId, u64>,
    /// Sessions discovery proved gone: never retried.
    forgotten: BTreeSet<SessionId>,
    /// Confirmed Deletes under way.
    removing: BTreeMap<SessionId, Removal>,
    /// Sessions waiting for a Kill-only instance, from when it may be
    /// admitted: started ahead of every waiting client.
    killing: BTreeMap<SessionId, Instant>,
    /// The most requests one instance's pending ledger holds: every request
    /// its library may have accepted, queued or under way.
    ledger: usize,
    /// Each release load under way, on its own thread.
    loads: Vec<(VersionName, JoinHandle<()>)>,
    /// The evidence of each instance that ended during shutdown, and of
    /// each earlier one whose cleanup is unconfirmed.
    ended: Vec<(SessionId, Evidence)>,
    /// Where shutdown says which sessions it still waits for.
    waits: Sender<Vec<SessionId>>,
}

impl State {
    fn new(
        context: Context,
        Shared {
            report,
            backlog,
            reserved,
            waits,
        }: Shared,
    ) -> Self {
        let (internal, internals) = mpsc::channel();
        let limits = context.limits;
        let accepted = limits.request_records as usize + limits.operations as usize;
        Self {
            ledger: accepted,
            queue: Queue::new(context.lanes.newer_start_burst),
            context: Arc::new(context),
            report,
            backlog,
            admitted: Ledger::default(),
            reserved,
            congested: Stalls::default(),
            stopping: false,
            // Which instance a first round starts from is not observable.
            turn: usize::default(),
            internal,
            internals,
            releases: BTreeMap::new(),
            instances: BTreeMap::new(),
            retrying: BTreeMap::new(),
            attempts: BTreeMap::new(),
            generations: BTreeMap::new(),
            forgotten: BTreeSet::new(),
            removing: BTreeMap::new(),
            killing: BTreeMap::new(),
            loads: Vec::new(),
            ended: Vec::new(),
            waits,
        }
    }

    /// Rounds of commands, internal events, scheduling, then a bounded poll
    /// and drains, until shutdown (at once if the UI thread is gone), then
    /// the shutdown and what it proved.
    fn serve(mut self, commands: &Receiver<Command>) -> shutdown::Report {
        let deadline = loop {
            if let Some(deadline) = self.commands(commands) {
                break deadline;
            }
            self.internals();
            self.schedule();
            // A shutdown command sent meanwhile is read once this round's
            // wait and drains end, at most a tick each: its deadline is
            // fixed, so that only shortens the shutdown, and the UI's
            // handover grace covers the report.
            self.poll(None);
        };
        self.begin_shutdown();
        self.settle_input(deadline);
        self.shut_down(deadline)
    }

    /// Nothing is scheduled any more, so no start or retry follows. Every
    /// instance that has not ended yet may have an active connection, so
    /// its departure is judged; the UI thread learns which it waits for.
    fn begin_shutdown(&mut self) {
        self.stopping = true;
        for instance in self.instances.values_mut() {
            instance.evidence.current = !matches!(instance.phase, Phase::Done);
        }
        self.tell_waiting();
    }

    /// Before shutting down, waits until every input chunk an open client
    /// accepted is answered, at most `deadlines.operation_ms` (owner decision
    /// on #109) and never past the shutdown `deadline` (owner decision on
    /// #118): typed keys read with prefix+x still reach their session, and
    /// a stuck one holds exit no longer than one operation may take.
    fn settle_input(&mut self, deadline: Instant) {
        let bound = Duration::from_millis(self.context.limits.operation_ms.into());
        let deadline = deadline.min(Instant::now() + bound);
        while self.input_pending() && before(deadline) {
            self.internals();
            self.poll(Some(deadline));
        }
    }

    /// Whether an open client has accepted input not yet answered.
    fn input_pending(&self) -> bool {
        self.instances.values().any(|instance| {
            matches!(instance.phase, Phase::Open(_))
                && instance
                    .pending
                    .values()
                    .any(|pending| matches!(pending, Pending::Request(Request::Input(..))))
        })
    }

    /// Takes every pending command; once shutdown is asked for, its
    /// deadline, which is now if the UI thread is gone.
    fn commands(&mut self, commands: &Receiver<Command>) -> Option<Instant> {
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if let Some(deadline) = self.command(command) {
                        return Some(deadline);
                    }
                }
                Err(error) => return (error == TryRecvError::Disconnected).then(Instant::now),
            }
        }
    }

    /// Acts on one command; a shutdown's deadline.
    fn command(&mut self, command: Command) -> Option<Instant> {
        match command {
            Command::Start(metadata) => self.wait(*metadata),
            Command::Request(id, request) => self.submit(&id, request),
            Command::Retire(retirement) => self.retire(retirement),
            Command::Shutdown(deadline) => return Some(deadline),
        }
        None
    }

    fn retire(&mut self, retirement: Retirement) {
        match retirement {
            Retirement::Gone(id) => self.forget(id),
            Retirement::Delete(metadata, kill) => self.remove(*metadata, kill),
            Retirement::Dropped(id, why) => self.fail(&id, why),
        }
    }

    /// Retries of a session proven gone stop: one scheduled is dropped,
    /// and none is scheduled again.
    fn forget(&mut self, id: SessionId) {
        self.retrying.remove(&id);
        self.forgotten.insert(id);
    }

    /// A confirmed Delete: its waiting start or retry is held until the
    /// Delete ends, and it moves on as far as it can.
    fn remove(&mut self, metadata: SessionMetadata, kill: bool) {
        let id = metadata.session_id.clone();
        let retried = self.retrying.remove(&id).is_some();
        let held = self.queue.take(|entry| entry.metadata.session_id == id);
        let removal = Removal {
            metadata,
            kill: [Kill::Skipped, Kill::Wanted][usize::from(kill)],
            ended: false,
            resume: retried || !held.is_empty(),
        };
        self.removing.insert(id.clone(), removal);
        self.advance(&id);
    }

    /// Moves `id`'s confirmed Delete on as far as it can go now.
    fn advance(&mut self, id: &SessionId) {
        let Some(removal) = self.removing.get(id) else {
            return;
        };
        let (kill, over) = (removal.kill, removal.over());
        match (self.phase(id), over) {
            (None, true) => self.removed(id),
            (None, false) => self.unheld(id, kill),
            (Some(phase), _) => self.held(id, phase, kill, over),
        }
    }

    /// The Delete is over and no instance is left: the row is dismissed.
    fn removed(&mut self, id: &SessionId) {
        self.removing.remove(id);
        self.tell(id, ClientEvent::Removed);
    }

    /// A Delete not yet over with no instance left: its Kill waits for a
    /// Kill-only instance, a refused one ends it, and an unconfirmed one is
    /// learned passively.
    fn unheld(&mut self, id: &SessionId, kill: Kill) {
        if kill == Kill::Wanted {
            self.killing.entry(id.clone()).or_insert_with(Instant::now);
        } else {
            self.unanswered(id, kill);
        }
    }

    /// A Delete with no instance left whose Kill waits for none: a refused
    /// Kill ends it, an unconfirmed one is learned passively, and any other
    /// leaves it as it is.
    fn unanswered(&mut self, id: &SessionId, kill: Kill) {
        match kill {
            Kill::Refused => self.refused(id),
            Kill::Unconfirmed => self.relearn(id),
            Kill::Skipped | Kill::Wanted | Kill::Pending | Kill::Answered => {}
        }
    }

    /// A Delete whose session still has an instance: once over, the
    /// instance is asked to stop; a Kill waiting for an open client is
    /// submitted through it.
    fn held(&mut self, id: &SessionId, phase: Phase, kill: Kill, over: bool) {
        match (phase, kill) {
            (phase, _) if over => {
                if let Some(live) = phase.live() {
                    live.client.shutdown();
                }
            }
            (Phase::Open(live), Kill::Wanted) => self.submit_kill(id, live),
            _ => {}
        }
    }

    /// A refused Delete with no instance left ends; a start or retry it
    /// held waits again.
    fn refused(&mut self, id: &SessionId) {
        let removal = self.removing.remove(id).expect("a confirmed Delete");
        if removal.resume {
            self.restart(removal.metadata);
        }
    }

    /// An unconfirmed Kill's outcome is learned by a passive client after
    /// a backoff, never by another Kill.
    fn relearn(&mut self, id: &SessionId) {
        let metadata = self.removing[id].metadata.clone();
        self.backoff(metadata);
    }

    /// Submits the Delete's Kill through `id`'s open client.
    /// Its request IDs never wrap: once they are exhausted, the Kill is
    /// refused unsent.
    fn submit_kill(&mut self, id: &SessionId, live: Live) {
        let instance = self.instances.get_mut(id).expect("an open instance");
        let exhausted = || "the client's request IDs are exhausted".to_owned();
        let submitted = instance.next.ok_or_else(exhausted).and_then(|request_id| {
            live.client.kill(request_id)?;
            Ok(request_id)
        });
        match submitted {
            Ok(request_id) => {
                instance.next = request_id.checked_add(1);
                instance.pending.insert(request_id, Pending::Kill);
                self.set_kill(id, Kill::Pending);
            }
            Err(why) => self.killed(id, rejected(why)),
        }
    }

    fn set_kill(&mut self, id: &SessionId, kill: Kill) {
        if let Some(removal) = self.removing.get_mut(id) {
            removal.kill = kill;
        }
    }

    /// A Delete's Kill was answered: a known command result ends the
    /// session; otherwise the row learns that the Kill was refused, which
    /// ends the Delete at once while a monitoring client stays, or that it
    /// is unconfirmed.
    fn killed(&mut self, id: &SessionId, answer: Answer) {
        if let Some(outcome) = answer.ended {
            self.ended(id, outcome);
        }
        let kill = self.verdict(id, answer.verdict);
        self.set_kill(id, kill);
        self.forsake(id, kill);
        self.advance(id);
    }

    /// Where a Kill stands after its answer's verdict, telling the row how
    /// it fell short.
    fn verdict(&self, id: &SessionId, verdict: Verdict) -> Kill {
        match verdict {
            Verdict::Accepted => Kill::Answered,
            Verdict::Rejected(why) => self.unkilled(id, Termination::Refused(why), Kill::Refused),
            Verdict::Uncertain(why) => {
                self.unkilled(id, Termination::Unconfirmed(why), Kill::Unconfirmed)
            }
        }
    }

    /// A refused Kill ends its Delete at once while a monitoring client of
    /// the session lives on.
    fn forsake(&mut self, id: &SessionId, kill: Kill) {
        let monitored = self.role(id) == Some(Role::Monitor);
        if kill == Kill::Refused && monitored && !self.over(id) {
            self.removing.remove(id);
        }
    }

    /// Tells the row how its Kill fell short, unless its end is known.
    fn unkilled(&self, id: &SessionId, termination: Termination, kill: Kill) -> Kill {
        if !self.over(id) {
            self.tell(id, ClientEvent::Termination(termination));
        }
        kill
    }

    /// Whether `id`'s Delete knows its session's end.
    fn over(&self, id: &SessionId) -> bool {
        self.removing.get(id).is_some_and(|removal| removal.ended)
    }

    fn role(&self, id: &SessionId) -> Option<Role> {
        self.instances.get(id).map(|instance| instance.role)
    }

    /// A Kill's answer carried the command's result: the row learns it as
    /// the final outcome, and nothing retries the session.
    fn ended(&mut self, id: &SessionId, outcome: Outcome) {
        let ended = Final {
            outcome,
            changed: None,
        };
        self.tell(id, ClientEvent::Report(Report::Final(ended)));
        self.finished(id);
    }

    /// A Kill-only instance for each Delete waiting for one whose
    /// admission interval has passed, ahead of every waiting client, as its
    /// release and the start lane allow; a refused release refuses its
    /// Kill.
    fn kill_only(&mut self, now: Instant) {
        let due = self.killing.iter().filter(|(_, from)| passed(**from, now));
        let killing: Vec<_> = due.map(|(id, _)| id.clone()).collect();
        for id in killing {
            let metadata = self.removing[&id].metadata.clone();
            match self.releases.get(&metadata.climon2_version) {
                Some(Release::Refused(refusal)) => {
                    let why = refusal.message().to_owned();
                    self.killing.remove(&id);
                    self.killed(&id, rejected(why));
                }
                Some(&Release::Ready {
                    functions,
                    instance,
                }) if room(self.starting(), self.context.lanes.starts) => {
                    self.start_kill(metadata, functions, instance, now);
                }
                _ => {}
            }
        }
    }

    /// Reserves a Kill-only instance and starts it, or reconsiders it after
    /// the admission interval.
    fn start_kill(
        &mut self,
        metadata: SessionMetadata,
        functions: &'static Functions,
        instance: admission::Instance,
        now: Instant,
    ) {
        let id = metadata.session_id.clone();
        let holder = (Holder::Instance(id.clone()), instance.amount);
        if let Err(why) = self.admitted.hold(&[holder], self.capacity()) {
            tracing::info!(session = %id, %why, "Kill-only instance resource-blocked");
            self.killing.insert(id, now + self.interval());
            return;
        }
        self.killing.remove(&id);
        self.spawn_owner(functions, instance.owner_stack, metadata, Role::KillOnly);
    }

    /// `metadata`'s session waits for its first start.
    fn wait(&mut self, metadata: SessionMetadata) {
        self.queue.push(metadata, Lane::First);
    }

    /// Submits `request` through an open client's input [`Stream`]: input
    /// in chunks of at most `chunk_bytes`, in order, and a paste's parts
    /// between the application's markers. The first refusal is answered at
    /// once with why, carrying every byte not submitted, and nothing after
    /// it is submitted.
    fn submit(&mut self, id: &SessionId, request: Request) {
        let chunk = self.context.limits.chunk_bytes as usize;
        let mut stream = self.stream(id);
        let refused = stream.submit(request, chunk, &mut |request| self.submit_one(id, request));
        self.restore(id, stream);
        if let Some((request, why)) = refused {
            self.tell(
                id,
                ClientEvent::Presentation(Presentation::Answered(request, Err(why))),
            );
        }
    }

    /// After an answer the library may have room for what a truncated paste
    /// owes its application.
    fn settle(&mut self, id: &SessionId) {
        let chunk = self.context.limits.chunk_bytes as usize;
        let mut stream = self.stream(id);
        let _ = stream.settle(chunk, &mut |request| self.submit_one(id, request));
        self.restore(id, stream);
    }

    /// `id`'s input stream, taken from its instance while requests are
    /// submitted through it.
    fn stream(&mut self, id: &SessionId) -> Stream {
        let instance = self.instances.get_mut(id);
        instance.map_or_else(Stream::default, |instance| {
            std::mem::take(&mut instance.stream)
        })
    }

    fn restore(&mut self, id: &SessionId, stream: Stream) {
        if let Some(instance) = self.instances.get_mut(id) {
            instance.stream = stream;
        }
    }

    /// Submits one request and enters it in the pending ledger once the
    /// library accepts it. A request refused, here or by the library,
    /// consumes no request ID.
    fn submit_one(&mut self, id: &SessionId, request: Request) -> Result<(), String> {
        let capacity = self.ledger;
        let open = self
            .instances
            .get_mut(id)
            .and_then(|instance| match instance.phase {
                Phase::Open(live) => Some((live, instance)),
                _ => None,
            });
        let Some((live, instance)) = open else {
            return Err("the session's client is not connected".to_owned());
        };
        if instance.pending.len() >= capacity {
            return Err("too many requests are awaiting their answers".to_owned());
        }
        let Some(request_id) = instance.next else {
            return Err("the client's request IDs are exhausted".to_owned());
        };
        live.client.submit(request_id, &request)?;
        instance.next = request_id.checked_add(1);
        instance
            .pending
            .insert(request_id, Pending::Request(request));
        Ok(())
    }

    /// A request's answer goes to the session's terminal as its request, a
    /// Kill's to its Delete.
    fn answer(&mut self, id: &SessionId, request_id: u64, answer: Answer) {
        let pending = self
            .instances
            .get_mut(id)
            .map(|instance| &mut instance.pending);
        match pending.and_then(|pending| pending.remove(&request_id)) {
            Some(Pending::Request(request)) => {
                let answered = Presentation::Answered(request, answer.result());
                self.report(id, ClientEvent::Presentation(answered));
                self.settle(id);
            }
            Some(Pending::Kill) => self.killed(id, answer),
            None => {}
        }
    }

    fn internals(&mut self) {
        while let Ok(internal) = self.internals.try_recv() {
            self.internal(internal);
        }
    }

    fn internal(&mut self, internal: Internal) {
        match internal {
            Internal::Loaded(version, loaded) => {
                self.join_load(&version);
                self.loaded(version, loaded);
            }
            Internal::Published(id, client, refused) => self.published(&id, client, refused),
            Internal::Stopped(id, ran, workers) => self.stopped(&id, ran, workers),
            Internal::OwnerDone(id, destroyed) => self.owner_done(&id, destroyed),
        }
    }

    /// Loads releases and starts clients the lanes have room for and
    /// admission admits, once each retry's backoff has passed; then tells
    /// each waiting row why it waits, if it is resource-blocked or its
    /// release is loading slowly. A session waiting on its release holds
    /// no start slot.
    fn schedule(&mut self) {
        let now = Instant::now();
        self.due();
        for version in self.versions() {
            self.release(&version, now);
        }
        self.refusals();
        self.kill_only(now);
        self.starts(now);
        self.slow(now);
        self.conditions();
        let reserved = self.admitted.bytes();
        self.reserved.store(reserved, Ordering::Relaxed);
    }

    /// The releases Kill-only instances and waiting sessions need, newest
    /// session first.
    fn versions(&mut self) -> Vec<VersionName> {
        let removing = &self.removing;
        let killing = self.killing.keys().map(|id| &removing[id].metadata);
        let waiting = self.queue.entries().map(|entry| &entry.metadata);
        let mut versions = Vec::new();
        for metadata in killing.chain(waiting) {
            let version = &metadata.climon2_version;
            if !versions.contains(version) {
                versions.push(version.clone());
            }
        }
        versions
    }

    /// Loads `version` if it is not loaded yet, or reconsiders it once its
    /// admission interval has passed.
    fn release(&mut self, version: &VersionName, now: Instant) {
        match self.releases.get(version) {
            None => self.load(version, now),
            Some(Release::Blocked { until, library, .. }) if passed(*until, now) => {
                let library = *library;
                self.reconsider(version, library, now);
            }
            Some(_) => {}
        }
    }

    /// A resource-blocked release once its interval has passed: loaded if
    /// it never was, otherwise qualified again.
    fn reconsider(&mut self, version: &VersionName, library: Option<Loaded>, now: Instant) {
        match library {
            None => self.load(version, now),
            Some(library) => self.qualify(version, library, now),
        }
    }

    /// Every waiting session whose release is refused: Unavailable, and
    /// retried unless that is permanent.
    fn refusals(&mut self) {
        let releases = &self.releases;
        let refused = self.queue.take(|entry| refusal(releases, entry).is_some());
        for entry in refused {
            if let Some(refusal) = refusal(&self.releases, &entry).cloned() {
                self.unstarted(
                    entry.metadata,
                    refusal.message().to_owned(),
                    refusal.retry(),
                );
            }
        }
    }

    /// Starts the sessions the queue picks while the start lane has room,
    /// each once admission reserves its instance.
    fn starts(&mut self, now: Instant) {
        while room(self.starting(), self.context.lanes.starts) {
            let releases = &self.releases;
            let Some(entry) = self.queue.pick(|entry| startable(releases, entry, now)) else {
                return;
            };
            self.admit(entry, now);
        }
    }

    /// Reserves `entry`'s instance and starts it, or leaves it
    /// resource-blocked until the next admission interval.
    fn admit(&mut self, mut entry: Entry, now: Instant) {
        let version = &entry.metadata.climon2_version;
        let Some(&Release::Ready {
            functions,
            instance,
        }) = self.releases.get(version)
        else {
            return self.queue.put(entry);
        };
        let holder = Holder::Instance(entry.metadata.session_id.clone());
        match self
            .admitted
            .hold(&[(holder, instance.amount)], self.capacity())
        {
            Ok(()) => {
                if entry.told.is_some() {
                    self.tell(&entry.metadata.session_id, ClientEvent::Starting);
                }
                self.spawn_owner(
                    functions,
                    instance.owner_stack,
                    entry.metadata,
                    Role::Monitor,
                );
            }
            Err(why) => {
                let id = &entry.metadata.session_id;
                tracing::info!(session = %id, %why, "client resource-blocked");
                entry.blocked = Some((why, now + self.interval()));
                self.queue.put(entry);
            }
        }
    }

    /// `retry.admission_ms`: the most often anything blocked is
    /// reconsidered.
    fn interval(&self) -> Duration {
        Duration::from_millis(self.context.admission.admission_ms.into())
    }

    /// What reservations may hold now: `memory.total_bytes` less what the
    /// monitor holds outside admission, and the operating system's
    /// descriptor limit, read afresh, less `resources.descriptor_headroom`.
    fn capacity(&self) -> Capacity {
        let admission = &self.context.admission;
        Capacity {
            bytes: admission.total_bytes.saturating_sub(self.context.outside),
            descriptors: process::descriptor_limit()
                .saturating_sub(admission.descriptor_headroom.into()),
        }
    }

    /// Each load running past `deadlines.load_ms` is logged once as loading
    /// slowly, and its waiting rows are told so; it is never aborted.
    fn slow(&mut self, now: Instant) {
        let bound = Duration::from_millis(self.context.lanes.load_ms.into());
        for (version, release) in &mut self.releases {
            if let Release::Loading {
                since,
                slow: slow @ false,
            } = release
                && passed(*since + bound, now)
            {
                *slow = true;
                tracing::warn!(%version, "client library loading slowly");
            }
        }
    }

    /// Tells each waiting row why it waits whenever that changes:
    /// resource-blocked with why, its release loading slowly, or, once
    /// neither holds any more, Starting.
    fn conditions(&mut self) {
        let mut changed = Vec::new();
        for entry in self.queue.entries() {
            let release = self.releases.get(&entry.metadata.climon2_version);
            let condition = condition(release, entry);
            if condition != entry.told {
                entry.told.clone_from(&condition);
                changed.push((entry.metadata.session_id.clone(), condition));
            }
        }
        for (id, condition) in changed {
            self.tell(&id, waiting(condition));
        }
    }

    fn loading(&self) -> u32 {
        let releases = self.releases.values();
        releases
            .filter(|release| matches!(release, Release::Loading { .. }))
            .count() as u32
    }

    fn starting(&self) -> u32 {
        let starting = self.instances.values();
        starting
            .filter(|instance| instance.phase.holds_slot())
            .count() as u32
    }

    /// Loads `version` on a worker thread while the load lane has room,
    /// once admission reserves its configured reservation and the load
    /// thread's stack before the dynamic loader opens it.
    fn load(&mut self, version: &VersionName, now: Instant) {
        if !room(self.loading(), self.context.lanes.loads) {
            return;
        }
        let admission = &self.context.admission;
        let holdings = [
            (
                Holder::Release(version.clone()),
                admission::configured(admission),
            ),
            (Holder::Load(version.clone()), admission::loader(admission)),
        ];
        match self.admitted.hold(&holdings, self.capacity()) {
            Ok(()) => self.spawn_load(version.clone(), now),
            Err(why) => self.block(version, why, None, now),
        }
    }

    fn spawn_load(&mut self, version: VersionName, now: Instant) {
        let loading = Release::Loading {
            since: now,
            slow: false,
        };
        self.releases.insert(version.clone(), loading);
        let context = Arc::clone(&self.context);
        let internal = self.internal.clone();
        let loading = version.clone();
        let spawned = thread::Builder::new()
            .name("load".into())
            .stack_size(stack(self.context.admission.stack_bytes))
            .spawn(move || {
                let loaded = (context.load)(&loading);
                let _ = internal.send(Internal::Loaded(loading, loaded));
            });
        match spawned {
            Ok(thread) => self.loads.push((version, thread)),
            Err(error) => self.loaded(version, Err(Refusal::Thread(error.to_string()))),
        }
    }

    /// `version`'s load thread has said its last: join it (#118).
    fn join_load(&mut self, version: &VersionName) {
        let (done, loading) = std::mem::take(&mut self.loads)
            .into_iter()
            .partition(|(loaded, _)| loaded == version);
        self.loads = loading;
        let done: Vec<_> = done;
        for (_, thread) in done {
            let _ = thread.join();
        }
    }

    /// `version`'s release is resource-blocked with why, `library` once
    /// loaded; reconsidered after the admission interval.
    fn block(&mut self, version: &VersionName, why: String, library: Option<Loaded>, now: Instant) {
        tracing::info!(%version, %why, "client library resource-blocked");
        let until = now + self.interval();
        let blocked = Release::Blocked {
            why,
            until,
            library,
        };
        self.releases.insert(version.clone(), blocked);
    }

    /// A load returned: its thread's reservation is freed, and the release
    /// is qualified, or refused.
    fn loaded(&mut self, version: VersionName, loaded: Result<Loaded, Refusal>) {
        self.admitted.free(&Holder::Load(version.clone()));
        match loaded {
            Ok(library) => {
                tracing::info!(%version, "client library loaded");
                self.qualify(&version, library, Instant::now());
            }
            Err(refusal) => self.refuse(version, refusal),
        }
    }

    /// Refuses `version`'s release; a reservation whose handle the dynamic
    /// loader never opened is freed, the others stay.
    fn refuse(&mut self, version: VersionName, refusal: Refusal) {
        tracing::warn!(%version, ?refusal, "client library refused");
        if !refusal.opened() {
            self.admitted.free(&Holder::Release(version.clone()));
        }
        self.releases.insert(version, Release::Refused(refusal));
    }

    /// Relaxed qualification (#120): the loaded release's requirements are
    /// checked against the limits, and its reservation raised to the
    /// greater of configured and qualified before any create; a raise
    /// admission refuses leaves it resource-blocked, its handle loaded.
    fn qualify(&mut self, version: &VersionName, library: Loaded, now: Instant) {
        let admission = self.context.admission;
        let requirements = &library.requirements;
        let instance = match admission::instance(&admission, requirements, &self.context.limits) {
            Ok(instance) => instance,
            Err(why) => return self.refuse(version.clone(), Refusal::Unqualified(why)),
        };
        let raised = (
            Holder::Release(version.clone()),
            admission::qualified(&admission, requirements),
        );
        match self.admitted.hold(&[raised], self.capacity()) {
            Ok(()) => {
                let ready = Release::Ready {
                    functions: library.functions,
                    instance,
                };
                self.releases.insert(version.clone(), ready);
            }
            Err(why) => self.block(version, why, Some(library), now),
        }
    }

    /// Starts an instance in `role` for `metadata`'s session in its next
    /// connection generation, the first 1. Generations never wrap: once a
    /// session's are exhausted, it is never attempted again (LIM-05).
    /// Its owner thread gets the stack admission reserved for it.
    fn spawn_owner(
        &mut self,
        functions: &'static Functions,
        stack: u64,
        metadata: SessionMetadata,
        role: Role,
    ) {
        let id = metadata.session_id.clone();
        let latest = self.generations.get(&id);
        let Some(generation) = latest.map_or(Some(self.context.first.generation), |latest| {
            latest.checked_add(1)
        }) else {
            let why = "connection generations are exhausted".to_owned();
            return self.unspawned(metadata, why, Retry::Permanent, role);
        };
        tracing::info!(session = %id, version = %metadata.climon2_version, ?role, "client starting");
        self.generations.insert(id.clone(), generation);
        let internal = self.internal.clone();
        let owned = id.clone();
        let tell = move |signal| {
            let _ = internal.send(Internal::of(owned.clone(), signal));
        };
        let started = Startup::new(&self.context.environment(), &metadata, generation);
        let spawned = started.and_then(|startup| {
            owner::spawn(
                functions,
                stack,
                startup,
                id.clone(),
                role,
                client::read,
                tell,
            )
        });
        match spawned {
            Ok(owner) => {
                let instance = Instance {
                    metadata,
                    role,
                    phase: Phase::Creating,
                    retry: None,
                    finished: false,
                    pending: BTreeMap::new(),
                    next: Some(self.context.first.request),
                    stream: Stream::default(),
                    owner,
                    evidence: Evidence::default(),
                };
                self.instances.insert(id.clone(), instance);
                self.started(&id, role);
            }
            Err(error) => self.unspawned(metadata, error.to_string(), Retry::Passive, role),
        }
    }

    /// A Kill-only instance's Kill is its first and only request, pending
    /// from its start, and it is never retried.
    fn started(&mut self, id: &SessionId, role: Role) {
        let instance = self.instances.get_mut(id).expect("a started instance");
        if role == Role::KillOnly {
            instance.pending.insert(KILL_ID, Pending::Kill);
            instance.retry = Some(Retry::None);
            self.set_kill(id, Kill::Pending);
        }
    }

    /// No instance could start: a monitoring client is Unavailable and
    /// retried as `retry` says; a Kill-only instance's Kill was never sent.
    /// What admission reserved for it is freed.
    fn unspawned(&mut self, metadata: SessionMetadata, why: String, retry: Retry, role: Role) {
        self.admitted
            .free(&Holder::Instance(metadata.session_id.clone()));
        match role {
            Role::Monitor => self.unstarted(metadata, why, retry),
            Role::KillOnly => self.killed(&metadata.session_id, rejected(why)),
        }
    }

    /// A session whose client could not start: Unavailable, and retried
    /// unless that is permanent. Anything admitted for it is freed.
    fn unstarted(&mut self, metadata: SessionMetadata, why: String, retry: Retry) {
        let id = metadata.session_id.clone();
        self.admitted.free(&Holder::Instance(id.clone()));
        self.tell(&id, ClientEvent::Unavailable(why.clone()));
        self.recover(&id, why, retry);
        if retry == Retry::Passive {
            self.backoff(metadata);
        }
    }

    /// Tells the row whether its client is retried, if it is or never is.
    fn recover(&self, id: &SessionId, why: String, retry: Retry) {
        if let Some(recovery) = recovery(retry, why) {
            self.tell(id, ClientEvent::Recovery(recovery));
        }
    }

    /// Schedules the session's next attempt after its backoff, unless
    /// discovery proved it gone.
    fn backoff(&mut self, metadata: SessionMetadata) {
        let id = metadata.session_id.clone();
        if self.forgotten.contains(&id) {
            return;
        }
        let attempt = self.attempts.get(&id).copied().unwrap_or_default();
        let delay = backoff::delay(self.context.retry, attempt, backoff::random());
        self.attempts.insert(id.clone(), attempt.saturating_add(1));
        tracing::info!(session = %id, delay_ms = delay.as_millis(), "client retry scheduled");
        self.retrying.insert(id, (Instant::now() + delay, metadata));
    }

    /// Each retry whose backoff has passed starts again: a fresh client
    /// waits for its start, its release loaded again if it was refused
    /// (only a refusal a later load may not repeat is ever retried).
    fn due(&mut self) {
        let now = Instant::now();
        let due: Vec<_> = self
            .retrying
            .iter()
            .filter(|(_, (at, _))| now.checked_duration_since(*at).is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            let (_, metadata) = self.retrying.remove(&id).expect("a due retry");
            let version = &metadata.climon2_version;
            if let Some(Release::Refused(_)) = self.releases.get(version) {
                self.releases.remove(version);
            }
            self.restart(metadata);
        }
    }

    /// A fresh client waits in the retry lane for its start, its row
    /// Starting again.
    fn restart(&mut self, metadata: SessionMetadata) {
        self.tell(&metadata.session_id, ClientEvent::Starting);
        self.queue.push(metadata, Lane::Retry);
    }

    /// The owner published its handles: look up the descriptor, or fail the
    /// instance without one. A Kill-only instance whose Kill was refused
    /// answers it so; one whose Delete is over is asked to stop.
    fn published(&mut self, id: &SessionId, client: Client, refused: Option<String>) {
        let lookup = client.notification();
        let descriptor = lookup.as_ref().ok().copied();
        self.set(id, Phase::Opening(Live { client, descriptor }));
        self.evidence(id, |evidence| evidence.published = true);
        tracing::info!(session = %id, "client published");
        if let Err(error) = lookup {
            self.fail(id, format!("cannot poll the client: {error}"));
        }
        let killing = self.role(id) == Some(Role::KillOnly);
        if let Some(why) = refused.filter(|_| killing) {
            self.answer(id, KILL_ID, rejected(why));
        }
        self.advance(id);
    }

    /// Waits a tick on the descriptor of every live instance that is not
    /// congested, then drains each of those; a congested one is failed once
    /// its congestion outlasts `deadlines.delivery_stall_ms` (#119). During
    /// shutdown neither the wait nor the drains go past its `deadline`. The
    /// drains share one bound, a tick after the wait (#462), and start one
    /// instance further on each round: each instance is drained at least
    /// once (during shutdown, only while its deadline has not passed), and
    /// none further past the bound, so one with deliveries without end
    /// neither starves the others nor keeps the round from its commands,
    /// however many instances there are.
    fn poll(&mut self, deadline: Option<Instant>) {
        let mut table: Vec<_> = self
            .instances
            .iter()
            .filter(|(id, _)| !self.throttled(id))
            .filter_map(|(_, instance)| instance.phase.live()?.descriptor)
            // SAFETY: a live instance's library keeps its notification
            // descriptor open, and the table ends with this wait.
            .map(|descriptor| unsafe { notifier::watch_raised(descriptor) })
            .collect();
        wait(&mut table, deadline.map_or(TICK, within));
        let mut live: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.phase.live().is_some())
            .map(|(id, _)| id.clone())
            .collect();
        // Each round starts one instance further on, so every instance in
        // turn has the bound to itself, however busy those before it are.
        let start = self.turn % live.len().max(1);
        live.rotate_left(start);
        self.turn += 1;
        let now = Instant::now();
        let round = now + TICK;
        let bound = deadline.map_or(round, |deadline| deadline.min(round));
        for id in live {
            if self.throttled(&id) {
                self.congestion(&id, now);
            } else {
                self.congested.drained(&id);
                self.drain_within(&id, bound, deadline.is_none_or(before));
            }
        }
    }

    /// Whether `id`'s reporting client has filled its `queues.event_bytes`
    /// with reports the UI has not processed: until the UI catches up it is
    /// not drained, and its records wait in the library. Ownership of what
    /// was drained moved to the UI, and its accounting with it.
    fn throttled(&self, id: &SessionId) -> bool {
        let reporting = self.phase(id).is_some_and(Phase::reporting) && !self.stopping;
        reporting && self.backlog.held(id) as u64 >= self.context.limits.event_bytes
    }

    /// `id` is congested at `now`: failed explicitly once that has lasted
    /// `deadlines.delivery_stall_ms`.
    fn congestion(&mut self, id: &SessionId, now: Instant) {
        let stall = Duration::from_millis(self.context.limits.delivery_stall_ms.into());
        if self.congested.congested(id, now, stall) {
            let why = format!("delivery congestion lasted past {} ms", stall.as_millis());
            self.fail(id, why);
        }
    }

    /// Drains `id` until it has nothing more, its reports fill its
    /// allowance ([`State::throttled`]) or `bound` passes, reporting every
    /// delivery; at least once if `once`, even past `bound`.
    fn drain_within(&mut self, id: &SessionId, bound: Instant, once: bool) {
        let mut due = once;
        while !self.throttled(id) && (due || before(bound)) && self.drain_one(id) {
            due = false;
        }
    }

    /// Drains one delivery of `id`'s, if it is live: whether there may be
    /// more. It does not block, so a deadline is checked between drains
    /// only: the library's drain is bounded local work, with no daemon I/O
    /// and no waiting (protocol README, Execution and representation), and
    /// reports reach the UI thread on an unbounded channel.
    fn drain_one(&mut self, id: &SessionId) -> bool {
        let Some(live) = self.phase(id).and_then(Phase::live) else {
            return false;
        };
        match live.client.drain(id) {
            Drain::Events(events) => {
                events.into_iter().for_each(|event| self.event(id, event));
                true
            }
            Drain::Stop(stop) => {
                self.stop(id, stop);
                false
            }
        }
    }

    /// Drains `id` until it has nothing more, reporting every delivery.
    fn drain(&mut self, id: &SessionId) {
        while self.drain_one(id) {}
    }

    fn stop(&mut self, id: &SessionId, stop: Stop) {
        match stop {
            Stop::Empty => {}
            Stop::Terminal(message, retry, terminal) => {
                self.evidence(id, |evidence| evidence.terminal = Some(terminal));
                self.terminal(id, message, retry);
            }
            Stop::Failed(error) => self.fail(id, format!("cannot drain the client: {error}")),
        }
    }

    /// A report from `id`'s deliveries: Open's refusal closes the instance;
    /// anything else reaches the row while it follows this instance.
    fn event(&mut self, id: &SessionId, event: ClientEvent) {
        match event {
            ClientEvent::Unavailable(why) => {
                self.close(id, why);
            }
            ClientEvent::Answer(request_id, answer) => self.answer(id, request_id, answer),
            event => self.report(id, event),
        }
    }

    fn report(&mut self, id: &SessionId, event: ClientEvent) {
        let Some(phase) = self.phase(id) else {
            return;
        };
        self.set(id, phase.after(&event));
        self.settle_retry(id, &event);
        if phase.reporting() {
            self.tell(id, event);
        }
        self.advance(id);
    }

    /// Whether the row follows `id`'s instance: a monitoring client, never
    /// a Kill-only one.
    fn follows(&self, id: &SessionId) -> bool {
        self.role(id) == Some(Role::Monitor)
    }

    /// An open client starts its session's backoff afresh; a final outcome
    /// means nothing retries it.
    fn settle_retry(&mut self, id: &SessionId, event: &ClientEvent) {
        match event {
            ClientEvent::Opened { .. } => {
                self.attempts.remove(id);
            }
            ClientEvent::Report(Report::Final(_)) => self.finished(id),
            _ => {}
        }
    }

    /// The command's final outcome is known for `id`: nothing retries it,
    /// and a Delete of it is over.
    fn finished(&mut self, id: &SessionId) {
        if let Some(instance) = self.instances.get_mut(id) {
            instance.finished = true;
        }
        if let Some(removal) = self.removing.get_mut(id) {
            removal.ended = true;
        }
    }

    /// Terminal evidence: report what it means, then release the handles.
    fn terminal(&mut self, id: &SessionId, message: String, retry: Retry) {
        tracing::info!(session = %id, %message, ?retry, "client ended");
        let message = Some(message).filter(|message| !message.is_empty());
        let why = message.unwrap_or_else(|| "the client stopped before it opened".into());
        self.finish(id, why, retry);
    }

    /// A failed instance the monitor stops: report it Unavailable once,
    /// retried passively, ask it to stop and keep draining it until its
    /// stream is over.
    fn fail(&mut self, id: &SessionId, why: String) {
        if let Some(live) = self.close(id, why.clone()) {
            live.client.shutdown();
            self.decide(id, why, Retry::Passive);
        }
    }

    /// Reports a reporting instance Unavailable once and keeps draining it
    /// until its stream is over; its handles, if it was reporting. Open's
    /// refusal ends the stream by itself, and its terminal evidence says
    /// whether it is retried.
    fn close(&mut self, id: &SessionId, why: String) -> Option<Live> {
        let phase = self.phase(id)?;
        let live = phase.live().filter(|_| phase.reporting())?;
        tracing::warn!(session = %id, %why, "client failed");
        self.set(id, Phase::Closing(live));
        if self.follows(id) {
            self.tell(id, ClientEvent::Unavailable(why));
        }
        Some(live)
    }

    /// Decides once whether the instance that is over is retried, and
    /// tells its row: never once its final outcome is known.
    fn decide(&mut self, id: &SessionId, why: String, retry: Retry) {
        let Some(instance) = self.instances.get_mut(id) else {
            return;
        };
        if instance.retry.is_some() {
            return;
        }
        let retry = [retry, Retry::None][usize::from(instance.finished)];
        instance.retry = Some(retry);
        self.recover(id, why, retry);
    }

    /// Run returned, or create failed: whatever is left to drain is
    /// drained, and an instance still not over ends here, retried passively
    /// unless create's refusal was permanent.
    fn stopped(
        &mut self,
        id: &SessionId,
        ran: Result<(), (String, Retry)>,
        workers: Result<(), String>,
    ) {
        self.evidence(id, |evidence| evidence.workers = Some(workers));
        self.drain(id);
        let (why, retry) = ran.err().unwrap_or_else(|| {
            let why = "the client stopped without terminal evidence";
            (why.into(), Retry::Passive)
        });
        if !matches!(self.phase(id), Some(Phase::Done) | None) {
            tracing::warn!(session = %id, %why, "client stopped");
            self.finish(id, why, retry);
        }
    }

    /// Reports the end and whether it is retried, releases the handles and
    /// lets the owner destroy.
    fn finish(&mut self, id: &SessionId, why: String, retry: Retry) {
        let Some(phase) = self.phase(id) else {
            return;
        };
        let released = phase.live().map_or(Ok(()), |live| live.client.release(id));
        self.evidence(id, |evidence| evidence.released = released.is_ok());
        self.set(id, Phase::Done);
        if let Some(instance) = self.instances.get(id) {
            instance.owner.release();
        }
        if let Some(event) = phase.over(why.clone()).filter(|_| self.follows(id)) {
            self.tell(id, event);
        }
        self.decide(id, why, retry);
    }

    /// The owner destroyed its instance, or could not, and is joined: only
    /// then may a passive retry be scheduled. One whose worker cleanup or
    /// destroy cannot be confirmed is never replaced, and its evidence is
    /// kept for the exit's fail-stop (#118), as is every instance's that
    /// ends during shutdown.
    fn owner_done(&mut self, id: &SessionId, destroyed: Result<(), String>) {
        let Some(mut instance) = self.instances.remove(id) else {
            return;
        };
        instance.owner.join();
        self.admitted.free(&Holder::Instance(id.clone()));
        let cleaned = self.joined(id, &mut instance.evidence, destroyed);
        if let Some(removal) = self.removing.get_mut(id) {
            removal.resume |= instance.retry == Some(Retry::Passive);
            return self.cleaned(id, cleaned);
        }
        self.retry_cleaned(id, instance.retry, instance.metadata, cleaned);
    }

    /// A joined instance the monitor retries passively is retried after
    /// its backoff once its cleanup is confirmed, and never once it cannot
    /// be; any other is done.
    fn retry_cleaned(
        &mut self,
        id: &SessionId,
        retry: Option<Retry>,
        metadata: SessionMetadata,
        cleaned: Result<(), String>,
    ) {
        if retry != Some(Retry::Passive) {
            return;
        }
        match cleaned {
            Ok(()) => self.backoff(metadata),
            Err(error) => {
                let why = format!("cannot confirm the client's cleanup: {error}");
                self.recover(id, why, Retry::Permanent);
            }
        }
    }

    /// A Delete's instance is cleaned up and joined (a retry it was owed
    /// waits for the Delete to end): a Kill it left unanswered is
    /// unconfirmed. One whose cleanup cannot be confirmed
    /// ends the Delete with why, its row kept (the fail-stop is #118's).
    fn cleaned(&mut self, id: &SessionId, destroyed: Result<(), String>) {
        if let Err(error) = destroyed {
            self.removing.remove(id);
            let why = format!("cannot confirm the client's cleanup: {error}");
            return self.tell(id, ClientEvent::Termination(Termination::Unconfirmed(why)));
        }
        if self.removing[id].kill == Kill::Pending {
            let why = "the client ended before the Kill was answered".to_owned();
            let kill = self.unkilled(id, Termination::Unconfirmed(why), Kill::Unconfirmed);
            self.set_kill(id, kill);
        }
        self.advance(id);
    }

    /// Records that `id`'s owner, done with `destroyed`, is joined, and
    /// keeps its evidence if shutdown has begun or its cleanup is
    /// unconfirmed; whether its worker cleanup and destroy are confirmed.
    fn joined(
        &mut self,
        id: &SessionId,
        evidence: &mut Evidence,
        destroyed: Result<(), String>,
    ) -> Result<(), String> {
        evidence.joined = true;
        evidence.destroyed = Some(destroyed.clone());
        let cleaned = evidence.workers.clone().unwrap_or(Ok(())).and(destroyed);
        if self.stopping || cleaned.is_err() {
            self.ended.push((id.clone(), evidence.clone()));
        }
        cleaned
    }

    /// Records evidence of `id`'s instance.
    fn evidence(&mut self, id: &SessionId, record: impl FnOnce(&mut Evidence)) {
        if let Some(instance) = self.instances.get_mut(id) {
            record(&mut instance.evidence);
        }
    }

    fn phase(&self, id: &SessionId) -> Option<Phase> {
        self.instances.get(id).map(|instance| instance.phase)
    }

    fn set(&mut self, id: &SessionId, phase: Phase) {
        if let Some(instance) = self.instances.get_mut(id) {
            instance.phase = phase;
        }
    }

    /// Reports `event` to the UI, charged to `id` until it is processed.
    fn tell(&self, id: &SessionId, event: ClientEvent) {
        self.backlog.charge(id, footprint(id, &event));
        let _ = self.report.send((id.clone(), event));
    }

    /// Asks every instance to stop, concurrently and again each round so
    /// that late publishers hear it too, and drains until each has ended
    /// and its owner is joined and every load has finished, or `deadline`
    /// passes. Dropping the state then closes every unconfirmed owner's
    /// release channel, so it never destroys an instance whose handles may
    /// be unreleased.
    fn shut_down(mut self, deadline: Instant) -> shutdown::Report {
        self.ask_shutdown();
        while !(self.instances.is_empty() && self.loads.is_empty()) && before(deadline) {
            self.internals();
            // Asked before the poll too (#462), so an instance `internals`
            // made live is asked before a poll that may run to the
            // deadline, not only after it.
            self.ask_shutdown();
            self.poll(Some(deadline));
            self.tell_waiting();
            self.ask_shutdown();
        }
        if !self.instances.is_empty() {
            tracing::error!(
                clients = self.instances.len(),
                "client shutdown unconfirmed at its deadline"
            );
        }
        self.proven()
    }

    /// Asks every live instance to shut down: idempotent, and asked even
    /// when settling input took the whole deadline. It does not block: the
    /// library's shutdown is a nonblocking intent (monitor-client README).
    fn ask_shutdown(&self) {
        let live = self.instances.values();
        live.filter_map(|instance| instance.phase.live())
            .for_each(|live| live.client.shutdown());
    }

    /// Tells the UI thread which sessions shutdown still waits for, on an
    /// unbounded channel, so it does not block.
    fn tell_waiting(&self) {
        let _ = self.waits.send(self.instances.keys().cloned().collect());
    }

    /// Every ended and still unconfirmed instance's evidence, and the
    /// loads still under way.
    fn proven(self) -> shutdown::Report {
        let mut instances = self.ended;
        let unconfirmed = self.instances.into_iter();
        instances.extend(unconfirmed.map(|(id, instance)| (id, instance.evidence)));
        instances.sort_by(|(one, _), (other, _)| one.cmp(other));
        let loads = self.loads.iter();
        shutdown::Report {
            instances,
            unfinished: loads
                .map(|(version, _)| format!("release {version} load"))
                .collect(),
        }
    }
}

/// A request refused before it was sent, with why.
fn rejected(why: String) -> Answer {
    Answer {
        verdict: Verdict::Rejected(why),
        ended: None,
    }
}

/// Whether `deadline` has not passed yet.
fn before(deadline: Instant) -> bool {
    deadline.checked_duration_since(Instant::now()).is_some()
}

/// Whether a lane of `bound` has room beside the `used` slots it holds.
fn room(used: u32, bound: u32) -> bool {
    used < bound
}

/// Why `entry`'s release is refused, if it is.
fn refusal<'a>(releases: &'a BTreeMap<VersionName, Release>, entry: &Entry) -> Option<&'a Refusal> {
    match releases.get(&entry.metadata.climon2_version) {
        Some(Release::Refused(refusal)) => Some(refusal),
        _ => None,
    }
}

/// Whether `entry` may start now: its release is ready, and it is not
/// waiting out an admission interval.
fn startable(releases: &BTreeMap<VersionName, Release>, entry: &Entry, now: Instant) -> bool {
    let release = releases.get(&entry.metadata.climon2_version);
    let ready = matches!(release, Some(Release::Ready { .. }));
    ready
        && entry
            .blocked
            .as_ref()
            .is_none_or(|(_, until)| passed(*until, now))
}

/// Whether `now` has reached `until`.
fn passed(until: Instant, now: Instant) -> bool {
    now.checked_duration_since(until).is_some()
}

/// Why a waiting session waits, as its row shows it: resource-blocked,
/// itself or its release; its release loading slowly; or neither.
fn condition(release: Option<&Release>, entry: &Entry) -> Option<Condition> {
    match (release, &entry.blocked) {
        (Some(Release::Blocked { why, .. }), _) | (_, Some((why, _))) => {
            Some(Condition::AwaitingResources(why.clone()))
        }
        (Some(Release::Loading { slow: true, .. }), None) => Some(Condition::LoadingSlowly),
        _ => None,
    }
}

/// What a row is told when why it waits changes.
fn waiting(condition: Option<Condition>) -> ClientEvent {
    match condition {
        Some(Condition::AwaitingResources(why)) => ClientEvent::AwaitingResources(why),
        Some(Condition::LoadingSlowly) => ClientEvent::LoadingSlowly,
        None => ClientEvent::Starting,
    }
}

/// One wait of a round before `deadline`: a tick, or what is left of it if
/// that is less, so the last round ends at the deadline, as far as the
/// platform timer allows (Windows wakes a wait at its scheduler tick, about
/// 15.6 ms), not a tick past it (#455). It is whole milliseconds rounded up, as a wait is: rounded down,
/// the last round would end before the deadline and cut a bounded wait,
/// such as settling input for `deadlines.operation_ms`, short, or spin
/// through its last millisecond; rounded up, it ends under a millisecond
/// past the deadline, well inside the UI's handover grace.
fn within(deadline: Instant) -> Duration {
    let left = TICK.min(deadline.saturating_duration_since(Instant::now()));
    u64::try_from(left.as_nanos().div_ceil(1_000_000)).map_or(TICK, Duration::from_millis)
}

/// Waits `bound` for any of `table`'s descriptors, or simply `bound`. A
/// failed wait only ends it early.
fn wait(table: &mut [Watch<'_>], bound: Duration) {
    if table.is_empty() {
        thread::sleep(bound);
        return;
    }
    let _ = waiting::wait(table, bound);
}

#[cfg(test)]
mod tests {
    //! The supervisor against a fake revision-1 client library (#106, Fault
    //! cases): Rust functions behind the same function table the validated
    //! loader hands the monitor. Each fake instance follows its session's
    //! [`Script`], keeps the native contract's ownership rules (handles release
    //! only once its stream is over and no delivery is outstanding; destroy is
    //! busy while either remains), and records what an independent observer
    //! needs: each destroy's result and when its owner thread exits.
    use super::{client::OPEN_ID, *};
    use crate::registry::{Final, Outcome, Recovery, Report};
    use crate::shutdown::{Leave, Terminal as Departed};
    use crate::terminal::{Authority, Request as Asked, Revocation};
    use climon2_core::protocol::TerminalSize;
    use climon2_core::{
        native::{
            Bytes, Call, Communication, Control, Created, Delivery, Drained, Error, Instance,
            Metrics, Record, Request, RunReport, Terminal,
        },
        wire::Guid,
    };
    use std::{
        cell::RefCell,
        collections::{BTreeSet, VecDeque},
        path::Path,
        ptr,
        sync::{Condvar, Mutex, PoisonError},
    };

    /// The connection generation the fake's own records name.
    const GENERATION: u64 = 1;

    /// How one fake instance behaves.
    #[derive(Clone, Copy, Default)]
    struct Script {
        /// Create fails with this call code.
        create_fails: Option<u32>,
        /// The terminal evidence's retry class (none unless set).
        retry: u32,
        /// The notification lookup fails.
        notification_fails: bool,
        /// Run fails at once, publishing nothing and no terminal evidence.
        run_fails: bool,
        /// Every drain fails.
        drain_fails: bool,
        /// Run ignores shutdown until the test lets it go.
        stalls: bool,
        /// Create returns only once the test lets it go.
        creates_late: bool,
        /// Run returns only once its terminal evidence is drained (or the
        /// test lets it go), as a library whose run outlives its stream.
        waits_for_drain: bool,
        /// The first shutdown lets this instance's create go, then returns
        /// only once its run has begun, so its handles are published by then.
        lets_go: Option<[u8; 16]>,
        /// After Open, one delivery of control and display records.
        presents: bool,
        /// The first this many requests after Open are refused at submit,
        /// synchronously, as full.
        refused_submits: u32,
        /// Requests after Open are accepted and never answered.
        silent: bool,
        /// Run sends Open's success only once the test lets it go (or
        /// shutdown asks), so the instance holds its start slot until then.
        opens_late: bool,
        /// Run ends at once, before Open's response, with terminal evidence
        /// carrying this message.
        ends_unopened: Option<&'static [u8]>,
        /// Run ends with terminal evidence right after Open's success (and
        /// any presentation): the connection is lost.
        drops: bool,
        /// Destroy fails, as with worker cleanup unconfirmed.
        destroy_fails: bool,
        /// Open is refused, and the stream ends with terminal evidence.
        refuses_open: bool,
        /// How a Kill is answered.
        kills: Killing,
        /// The connection is lost once a Kill is answered.
        drops_after_kill: bool,
        /// Once a Kill is answered, the presentation's records follow,
        /// with the command's final results.
        presents_after_kill: bool,
        /// Destroy waits until the test lets it go.
        destroy_waits: bool,
        /// The terminal evidence's Leave code, when not confirmed.
        leave: Option<u32>,
        /// The client outcome is failed with this message.
        client_fails: Option<&'static [u8]>,
        /// Releasing the control handle stays busy.
        release_fails: bool,
        /// Once shutdown is asked, every drain is another delivery, without
        /// end (#455).
        floods: bool,
        /// Once its queue is empty, every drain is another delivery,
        /// without end, whether shutdown is asked or not (#462).
        floods_at_once: bool,
    }

    /// How the fake answers a Kill (#117).
    #[derive(Clone, Copy, Default)]
    enum Killing {
        /// Terminated by signal 9, cleanup complete.
        #[default]
        Confirms,
        /// Refused without effect.
        Refuses,
        /// Indeterminate: the answer was lost.
        Loses,
        /// Never answered.
        Ignores,
        /// Refused at submit, never sent.
        RefusedAtSubmit,
    }

    /// What the observer saw of one session's instance.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Seen {
        Created,
        Destroyed(u32),
        OwnerExited,
    }

    /// A request the fake accepted: tag, request ID and its arguments' words
    /// (generation; or generation, period, restoration; plus columns and rows,
    /// or each input byte).
    type Submitted = (u32, u64, Vec<u64>);

    #[derive(Default)]
    struct World {
        /// Each session's script for each of its instances in turn, the
        /// last for every later one.
        scripts: BTreeMap<[u8; 16], Vec<Script>>,
        /// Each instance's connection generation, as its startup named it.
        generations: Vec<([u8; 16], u64)>,
        /// When each of `seen` happened.
        times: Vec<([u8; 16], Seen, Instant)>,
        seen: Vec<([u8; 16], Seen)>,
        unstalled: BTreeSet<[u8; 16]>,
        submitted: Vec<([u8; 16], Submitted)>,
        /// Instances whose notification descriptor was looked up: the
        /// supervisor does so as it learns of their handles.
        looked_up: BTreeSet<[u8; 16]>,
        /// Each instance that was submitted monitor Open, by session.
        opens: Vec<[u8; 16]>,
        /// How many times each instance was drained.
        drains: BTreeMap<[u8; 16], usize>,
        /// Instances whose run has begun, after their handles were
        /// published.
        running: BTreeSet<[u8; 16]>,
        /// When each instance was first asked to shut down: `drains` then.
        first_asks: BTreeMap<[u8; 16], BTreeMap<[u8; 16], usize>>,
    }

    impl World {
        fn saw(&mut self, id: [u8; 16], seen: Seen) {
            self.seen.push((id, seen));
            self.times.push((id, seen, Instant::now()));
        }
    }

    static WORLD: Mutex<World> = Mutex::new(World {
        times: Vec::new(),
        scripts: BTreeMap::new(),
        generations: Vec::new(),
        seen: Vec::new(),
        unstalled: BTreeSet::new(),
        submitted: Vec::new(),
        looked_up: BTreeSet::new(),
        opens: Vec::new(),
        drains: BTreeMap::new(),
        running: BTreeSet::new(),
        first_asks: BTreeMap::new(),
    });
    static CHANGED: Condvar = Condvar::new();
    const WAIT: Duration = Duration::from_secs(20);

    fn world() -> std::sync::MutexGuard<'static, World> {
        WORLD.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn changed(world: &mut World, change: impl FnOnce(&mut World)) {
        change(world);
        CHANGED.notify_all();
    }

    /// Waits until `done` holds of the world, at most [`WAIT`].
    fn until(done: impl FnMut(&World) -> bool) {
        assert!(
            waited(done),
            "the fake library never reached the expected state"
        );
    }

    /// Waits until `done` holds of the world, at most [`WAIT`]: whether it
    /// does.
    fn waited(mut done: impl FnMut(&World) -> bool) -> bool {
        let deadline = Instant::now() + WAIT;
        let mut world = world();
        while !done(&world) {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else {
                return false;
            };
            world = CHANGED
                .wait_timeout(world, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        true
    }

    fn seen(id: &[u8; 16]) -> Vec<Seen> {
        world()
            .seen
            .iter()
            .filter(|(seen, _)| seen == id)
            .map(|(_, seen)| *seen)
            .collect()
    }

    /// Records `OwnerExited` when the owner thread that created the instance
    /// exits, after everything it did.
    struct Exit([u8; 16]);

    impl Drop for Exit {
        fn drop(&mut self) {
            changed(&mut world(), |world| {
                world.saw(self.0, Seen::OwnerExited);
            });
        }
    }

    thread_local! {
        static OWNER: RefCell<Option<Exit>> = const { RefCell::new(None) };
    }

    enum Item {
        Delivery(Vec<Record>),
        Terminal,
    }

    #[derive(Default)]
    struct State {
        queue: VecDeque<Item>,
        shutdown: bool,
        /// Run returned or terminal evidence was drained: the stream is over.
        over: bool,
        outstanding: usize,
        /// Requests after Open refused at submit so far.
        refused: u32,
        communication: bool,
        control: bool,
        /// Monitor Open was submitted: a Kill-only instance has none.
        opened: bool,
        /// The connection was lost after a Kill's answer.
        dropped: bool,
        /// Its terminal evidence was drained.
        drained: bool,
    }

    struct Fake {
        id: [u8; 16],
        script: Script,
        /// The notification descriptor: raised while items may wait.
        notifier: climon2_platform::notifier::Notifier,
        state: Mutex<State>,
        terminal: Terminal,
    }

    impl Fake {
        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn push(&self, item: Item) {
            self.state().queue.push_back(item);
            self.notifier.set(true);
        }
    }

    fn fake<'a, T>(handle: *mut T) -> &'a Fake {
        // SAFETY: every handle the fake hands out is its `Fake`.
        unsafe { &*handle.cast::<Fake>() }
    }

    const fn bytes_of(text: &'static [u8]) -> Bytes {
        Bytes {
            data: text.as_ptr(),
            length: text.len() as u64,
        }
    }

    const NONE: Bytes = Bytes {
        data: ptr::null(),
        length: 0,
    };

    fn success() -> Call {
        Call {
            code: native::SUCCESS,
            reserved: 0,
            error: no_error(),
        }
    }

    fn no_error() -> Error {
        Error {
            origin: native::ORIGIN_NONE,
            reserved: 0,
            code: NONE,
            message: NONE,
        }
    }

    fn failure(code: u32, name: &'static [u8], message: &'static [u8]) -> Call {
        Call {
            code,
            reserved: 0,
            error: Error {
                origin: native::ORIGIN_LIBRARY,
                reserved: 0,
                code: bytes_of(name),
                message: bytes_of(message),
            },
        }
    }

    fn busy() -> Call {
        failure(native::BUSY, b"busy", b"still in use")
    }

    extern "C" fn create(startup: *const native::Startup, out: *mut Created) -> Call {
        // SAFETY: the supervisor passes a complete startup.
        let (id, generation) = unsafe { ((*startup).session_id.bytes, (*startup).generation) };
        let script = {
            let mut world = world();
            let made = world
                .generations
                .iter()
                .filter(|(made, _)| *made == id)
                .count();
            world.generations.push((id, generation));
            let scripts = world.scripts.get(&id).cloned().unwrap_or_default();
            let last = scripts.len().saturating_sub(1);
            scripts.get(made.min(last)).copied().unwrap_or_default()
        };
        if let Some(code) = script.create_fails {
            return failure(code, b"refused", b"create refused");
        }
        let fake = Box::into_raw(Box::new(Fake {
            id,
            script,
            notifier: climon2_platform::notifier::Notifier::new().unwrap(),
            state: Mutex::default(),
            terminal: Terminal {
                generation: GENERATION,
                client_outcome: [native::CLIENT_STOPPED, native::CLIENT_FAILED]
                    [usize::from(script.client_fails.is_some())],
                retry: script.retry,
                leave: script.leave.unwrap_or(native::LEAVE_CONFIRMED),
                attachment_detached: 0,
                connection_closed: 1,
                reserved: [0; 2],
                error: Error {
                    code: bytes_of(script.client_fails.map_or(b"", |_| b"timeout")),
                    message: bytes_of(
                        script
                            .client_fails
                            .or(script.ends_unopened)
                            .unwrap_or_default(),
                    ),
                    ..no_error()
                },
            },
        }));
        OWNER.with(|owner| *owner.borrow_mut() = Some(Exit(id)));
        changed(&mut world(), |world| world.saw(id, Seen::Created));
        wait_while(|world| script.creates_late && !world.unstalled.contains(&id));
        // SAFETY: the supervisor passes an output struct.
        unsafe {
            let out = &mut *out;
            out.instance = fake.cast();
            out.communication = fake.cast();
            out.control = fake.cast();
        }
        success()
    }

    /// Monitor Open's successful response, selecting no optional family.
    fn opened() -> Vec<Record> {
        // SAFETY: an all-zero record is a valid plain value.
        let mut record: Record = unsafe { std::mem::zeroed() };
        record.tag = native::RESPONSE;
        record.generation = GENERATION;
        record.data.response = native::Response {
            request_id: OPEN_ID,
            outcome: native::OK,
            result_tag: native::OPENED,
            error: no_error(),
            result: native::ResponseResult {
                opened: native::Opened {
                    connection_id: native::Guid { bytes: [0; 16] },
                    selected: native::Tokens {
                        data: ptr::null(),
                        count: 0,
                    },
                },
            },
        };
        vec![record]
    }

    extern "C" fn run(instance: *mut Instance, out: *mut RunReport) -> Call {
        let fake = fake(instance);
        changed(&mut world(), |world| {
            world.running.insert(fake.id);
        });
        if fake.script.run_fails {
            fake.state().over = true;
            // SAFETY: the owner passes an output struct.
            unsafe { (*out).workers_cleanup = native::CLEANUP_COMPLETE };
            return failure(native::INTERNAL_ERROR, b"internal_error", b"run failed");
        }
        let opened = fake.state().opened;
        if fake.script.ends_unopened.is_none() && opened {
            serve(fake);
        }
        fake.push(Item::Terminal);
        fake.state().over = true;
        let let_go = |world: &World| world.unstalled.contains(&fake.id);
        wait_while(|world| fake.script.waits_for_drain && !fake.state().drained && !let_go(world));
        success()
    }

    /// Run's opened life: Open's success, as late as the script says, then
    /// shutdown, as late as the script says.
    fn serve(fake: &Fake) {
        let let_go = |world: &World| world.unstalled.contains(&fake.id);
        let shutdown = || fake.state().shutdown;
        wait_while(|world| fake.script.opens_late && !let_go(world) && !shutdown());
        if fake.script.refuses_open {
            return fake.push(Item::Delivery(vec![answer(OPEN_ID, true)]));
        }
        fake.push(Item::Delivery(opened()));
        if fake.script.presents {
            fake.push(Item::Delivery(presentation()));
        }
        if fake.script.drops {
            return;
        }
        let dropped = || fake.state().dropped;
        wait_while(|world| !(shutdown() && (!fake.script.stalls || let_go(world)) || dropped()));
    }

    /// Waits for each change of the world while `blocked` holds of it.
    fn wait_while(mut blocked: impl FnMut(&World) -> bool) {
        let mut guard = world();
        while blocked(&guard) {
            guard = CHANGED.wait(guard).unwrap_or_else(PoisonError::into_inner);
        }
    }

    extern "C" fn destroy(instance: *mut Instance) -> Call {
        let fake = fake(instance);
        wait_while(|world| fake.script.destroy_waits && !world.unstalled.contains(&fake.id));
        let state = fake.state();
        let released = state.communication && state.control && state.outstanding == 0;
        let free = released && !fake.script.destroy_fails;
        drop(state);
        let call = if free { success() } else { busy() };
        changed(&mut world(), |world| {
            world.saw(fake.id, Seen::Destroyed(call.code));
        });
        call
    }

    extern "C" fn submit(communication: *mut Communication, request: *const Request) -> Call {
        // SAFETY: the supervisor passes a complete request.
        let request = unsafe { &*request };
        let fake = fake(communication);
        if request.tag == native::OPEN {
            assert_eq!(request.request_id, OPEN_ID);
            fake.state().opened = true;
            changed(&mut world(), |world| world.opens.push(fake.id));
            return success();
        }
        if request.tag == native::KILL {
            return kill(fake, request.request_id);
        }
        {
            let mut state = fake.state();
            if state.refused < fake.script.refused_submits {
                state.refused += 1;
                return failure(native::FULL, b"full", b"the request queue is full");
            }
        }
        // SAFETY: the tag names the active member.
        let words = unsafe {
            match request.tag {
                native::ACQUIRE => vec![request.args.generation],
                native::READY => authority_words(request.args.authority),
                native::INPUT => input_words(request.args.input),
                _ => {
                    let resize = request.args.resize;
                    let mut words = authority_words(resize.authority);
                    words.extend([
                        u64::from(resize.geometry.cols),
                        u64::from(resize.geometry.rows),
                    ]);
                    words
                }
            }
        };
        // Input holding an F does not fit.
        if request.tag == native::INPUT && words[3..].contains(&u64::from(b'F')) {
            return failure(native::FULL, b"full", b"no room");
        }
        changed(&mut world(), |world| {
            world
                .submitted
                .push((fake.id, (request.tag, request.request_id, words)));
        });
        if fake.script.silent {
            return success();
        }
        // Ready is refused; anything else accepted, after an answer to a
        // request never made.
        let refused = request.tag == native::READY;
        fake.push(Item::Delivery(vec![
            answer(99, false),
            answer(request.request_id, refused),
        ]));
        success()
    }

    /// Records a Kill and answers it as the script says, then drops the
    /// connection if the script says so.
    fn kill(fake: &Fake, request_id: u64) -> Call {
        if let Killing::RefusedAtSubmit = fake.script.kills {
            return failure(native::FULL, b"full", b"no room for a kill");
        }
        changed(&mut world(), |world| {
            world
                .submitted
                .push((fake.id, (native::KILL, request_id, Vec::new())));
        });
        let mut answer = record(native::RESPONSE);
        // SAFETY: an all-zero response is a valid plain value.
        let mut response: native::Response = unsafe { std::mem::zeroed() };
        response.request_id = request_id;
        response.error = no_error();
        match fake.script.kills {
            Killing::Confirms => {
                response.outcome = native::OK;
                response.result_tag = native::TERMINATION;
                // SAFETY: an all-zero termination is a valid plain value.
                let mut termination: native::Termination = unsafe { std::mem::zeroed() };
                termination.command_result.present = 1;
                termination.command_result.value.kind = native::SIGNAL;
                termination.command_result.value.value = 9;
                termination.cleanup = native::CLEANUP_COMPLETE;
                response.result.termination = termination;
            }
            Killing::Refuses => {
                response.outcome = native::REJECTED;
                response.error.message = bytes_of(b"refused");
            }
            Killing::Loses => {
                response.outcome = native::INDETERMINATE;
                response.error.message = bytes_of(b"timeout");
            }
            Killing::Ignores | Killing::RefusedAtSubmit => return success(),
        }
        answer.data.response = response;
        fake.push(Item::Delivery(vec![answer]));
        if fake.script.presents_after_kill {
            fake.push(Item::Delivery(presentation()));
        }
        if fake.script.drops_after_kill {
            changed(&mut world(), |_| fake.state().dropped = true);
        }
        success()
    }

    /// The input's authority, then each of its bytes.
    ///
    /// # Safety
    /// The input's bytes are valid for this call.
    unsafe fn input_words(input: native::Input) -> Vec<u64> {
        let mut words = authority_words(input.authority);
        // SAFETY: as the caller promises.
        let bytes =
            unsafe { std::slice::from_raw_parts(input.bytes.data, input.bytes.length as usize) };
        words.extend(bytes.iter().map(|&byte| u64::from(byte)));
        words
    }

    fn authority_words(authority: native::Authority) -> Vec<u64> {
        vec![
            authority.generation,
            authority.period,
            authority.restoration_id,
        ]
    }

    fn record(tag: u32) -> Record {
        // SAFETY: an all-zero record is a valid plain value.
        let mut record: Record = unsafe { std::mem::zeroed() };
        record.tag = tag;
        record.generation = GENERATION;
        record
    }

    /// A command's answer: accepted, or rejected as stale.
    fn answer(request_id: u64, refused: bool) -> Record {
        let mut answer = record(native::RESPONSE);
        // SAFETY: an all-zero response is a valid plain value.
        let mut response: native::Response = unsafe { std::mem::zeroed() };
        response.request_id = request_id;
        response.outcome = if refused {
            native::REJECTED
        } else {
            native::OK
        };
        response.error = no_error();
        if refused {
            response.error.message = bytes_of(b"stale");
        }
        answer.data.response = response;
        answer
    }

    static DISPLAYED: &[u8] = b"\0hi";

    /// Initial metadata that knows the command's result, then a grant (in
    /// generation 7), a restoration with its bytes, two Geometry records (one
    /// too wide for any session, one with no columns), a revocation, a Command
    /// final by signal 15 and a status record with no known state, which
    /// reports nothing.
    fn presentation() -> Vec<Record> {
        let geometry = |cols, rows| native::Geometry { cols, rows };
        let mut metadata = record(native::INITIAL_METADATA);
        // SAFETY: an all-zero metadata record is a valid plain value.
        let mut initial: native::Metadata = unsafe { std::mem::zeroed() };
        initial.title.state = native::VALUE;
        initial.title.value = bytes_of(b"T");
        initial.command_result.present = 1;
        initial.command_result.value.kind = native::EXIT;
        metadata.data.metadata = initial;
        let mut granted = record(native::GRANTED);
        granted.generation = 7;
        granted.data.granted = native::Granted {
            period: 2,
            restoration_id: 3,
            geometry: geometry(80, 24),
            reason: native::TAKE_CONTROL,
            reserved: 0,
        };
        let mut begun = record(native::RESTORATION_BEGIN);
        begun.data.restoration = native::Restoration {
            period: 2,
            restoration_id: 4,
            geometry: geometry(100, 30),
        };
        let barrier = native::Barrier {
            period: 2,
            restoration_id: 4,
        };
        let mut display = record(native::DISPLAY);
        display.data.display = native::Display {
            authority: barrier,
            bytes: bytes_of(DISPLAYED),
        };
        let mut complete = record(native::RESTORATION_COMPLETE);
        complete.data.barrier = barrier;
        let reshaped = |cols| {
            let mut changed = record(native::GEOMETRY);
            changed.data.geometry = native::GeometryEvent {
                period: 2,
                geometry: geometry(cols, 5),
            };
            changed
        };
        let mut ended = record(native::FINAL);
        // SAFETY: an all-zero Command final is a valid plain value.
        let mut known: native::Final = unsafe { std::mem::zeroed() };
        known.command_result.kind = native::SIGNAL;
        known.command_result.value = 15;
        ended.data.r#final = known;
        let revoked = |reason| {
            let mut revoked = record(native::REVOKED);
            revoked.data.revoked = native::Revoked {
                period: 2,
                restoration_id: 4,
                reason,
                reserved: 0,
            };
            revoked
        };
        vec![
            metadata,
            granted,
            begun,
            display,
            complete,
            reshaped(70_000),
            reshaped(0),
            revoked(native::TAKEOVER),
            revoked(native::SESSION_ENDING),
            revoked(99),
            ended,
            record(native::STATUS),
        ]
    }

    extern "C" fn drain(communication: *mut Communication, out: *mut Drained) -> Call {
        let fake = fake(communication);
        changed(&mut world(), |world| {
            *world.drains.entry(fake.id).or_default() += 1;
        });
        if fake.script.drain_fails {
            return failure(native::INTERNAL_ERROR, b"internal_error", b"drain failed");
        }
        let mut state = fake.state();
        // SAFETY: the supervisor passes an output struct.
        let out = unsafe { &mut *out };
        let flood = fake.script.floods && state.shutdown
            || fake.script.floods_at_once && state.queue.is_empty();
        let next = if flood {
            // There is always more, so the descriptor stays raised.
            fake.notifier.set(true);
            Some(Item::Delivery(Vec::new()))
        } else {
            state.queue.pop_front()
        };
        let terminal = matches!(next, Some(Item::Terminal));
        match next {
            Some(Item::Delivery(records)) => {
                state.outstanding += 1;
                let records = Box::leak(records.into_boxed_slice());
                out.kind = native::DELIVERY;
                out.records = records.as_ptr();
                out.count = records.len() as u64;
                out.delivery = Box::into_raw(Box::new((communication, records))).cast();
            }
            Some(Item::Terminal) => {
                state.over = true;
                state.drained = true;
                out.kind = native::TERMINAL;
                out.terminal = &fake.terminal;
            }
            None => {
                fake.notifier.set(false);
                out.kind = native::EMPTY;
            }
        }
        drop(state);
        if terminal {
            changed(&mut world(), |_| {});
        }
        success()
    }

    extern "C" fn release_delivery(delivery: *mut Delivery) -> Call {
        // SAFETY: made by `drain` and released once.
        let (communication, records) =
            *unsafe { Box::from_raw(delivery.cast::<(*mut Communication, &mut [Record])>()) };
        // SAFETY: leaked by `drain`.
        drop(unsafe { Box::from_raw(records as *mut [Record]) });
        fake(communication).state().outstanding -= 1;
        success()
    }

    extern "C" fn shutdown(control: *mut Control) -> Call {
        let fake = fake(control);
        let mut first = false;
        changed(&mut world(), |world| {
            fake.state().shutdown = true;
            if !world.first_asks.contains_key(&fake.id) {
                first = true;
                let drains = world.drains.clone();
                world.first_asks.insert(fake.id, drains);
            }
        });
        if let Some(late) = fake.script.lets_go.filter(|_| first) {
            changed(&mut world(), |world| {
                world.unstalled.insert(late);
            });
            // An owner that exits without running ends the wait too, and it
            // lasts at most WAIT, so the test fails rather than hangs (a
            // panic here would cross the C boundary and abort).
            let exited = (late, Seen::OwnerExited);
            let _ = waited(|world| world.running.contains(&late) || world.seen.contains(&exited));
        }
        success()
    }

    extern "C" fn release_communication(communication: *mut Communication) -> Call {
        let mut state = fake(communication).state();
        if !state.over || state.outstanding > 0 {
            return busy();
        }
        state.communication = true;
        success()
    }

    extern "C" fn release_control(control: *mut Control) -> Call {
        let fake = fake(control);
        let mut state = fake.state();
        if !state.over || fake.script.release_fails {
            return busy();
        }
        state.control = true;
        success()
    }

    extern "C" fn notification(communication: *mut Communication, out: *mut i32) -> Call {
        let fake = fake(communication);
        let (descriptor, call) = if fake.script.notification_fails {
            let call = failure(
                native::INTERNAL_ERROR,
                b"internal_error",
                b"no notification",
            );
            (-1, call)
        } else {
            (fake.notifier.descriptor(), success())
        };
        // SAFETY: the supervisor passes an output integer.
        unsafe { *out = descriptor };
        changed(&mut world(), |world| {
            world.looked_up.insert(fake.id);
        });
        call
    }

    extern "C" fn snapshot(_communication: *mut Communication, _out: *mut Metrics) -> Call {
        failure(native::UNSUPPORTED_OPERATION, b"unsupported_operation", b"")
    }

    /// The fake library as its load hands it over, with the shipped
    /// library's requirements.
    const FAKE_LIBRARY: Loaded = Loaded {
        functions: &FAKE,
        requirements: native::Requirements {
            release_bytes: 64 << 20,
            instance_bytes: 1 << 20,
            owner_stack_bytes: 2 << 20,
            worker_stack_bytes: 2 << 20,
            workers: 2,
            instance_descriptors: 8,
            load_descriptors: 4,
            reserved: 0,
        },
    };

    static FAKE: Functions = Functions {
        create,
        run,
        destroy,
        submit,
        drain,
        release_delivery,
        shutdown,
        release_communication,
        release_control,
        notification,
        snapshot,
    };

    /// A session ID unique to one test: `test` names it, `index` the session.
    fn session(test: u8, index: u8) -> SessionId {
        format!("0d5c{test:02x}00-0000-4000-8000-0000000000{index:02x}")
            .parse()
            .unwrap()
    }

    fn guid(id: &SessionId) -> [u8; 16] {
        id.as_str().parse::<Guid>().unwrap().bytes()
    }

    fn script(id: &SessionId, script: Script) {
        scripts(id, &[script]);
    }

    /// `id`'s instances follow `scripts` in turn, the last for every later
    /// one.
    fn scripts(id: &SessionId, scripts: &[Script]) {
        world().scripts.insert(guid(id), scripts.to_vec());
    }

    fn metadata(id: &SessionId, created: &str) -> SessionMetadata {
        versioned(id, created, "0.1.0")
    }

    fn versioned(id: &SessionId, created: &str, version: &str) -> SessionMetadata {
        serde_json::from_value(serde_json::json!({
            "format_version": 1, "session_id": id.as_str(), "socket_path": "/tmp/s",
            "daemon_pid": 1, "daemon_start_time": 0, "command": "fake", "args": [],
            "cwd": "/", "depth": 1, "created_at": created, "climon2_version": version
        }))
        .unwrap()
    }

    /// A supervisor in its own climon2 home, and the home.
    struct Supervised {
        directory: tempfile::TempDir,
        supervisor: Supervisor,
        log: std::path::PathBuf,
    }

    impl Supervised {
        /// What the supervisor wrote to the monitor's general log.
        fn log(&self) -> String {
            std::fs::read_to_string(&self.log).unwrap()
        }

        /// Whether `id`'s private log directory exists: the supervisor makes
        /// it when, and only when, it starts that session's client.
        fn started(&self, id: &SessionId) -> bool {
            self.directory
                .path()
                .join("logs")
                .join(id.as_str())
                .is_dir()
        }
    }

    /// A backoff so long that no retry starts within a test.
    const NEVER: Backoff = Backoff {
        initial_ms: 3_600_000,
        cap_ms: 3_600_000,
        jitter_percent: 0,
    };

    /// A supervisor over `load` in its own climon2 home, logging to that
    /// home's monitor log; it schedules retries but never starts one.
    fn supervisor(starts: u32, shutdown_ms: u64, load: Load) -> Supervised {
        let limits = climon2_core::config::Limits::default().library();
        limited(starts, shutdown_ms, load, limits)
    }

    /// [`supervisor`] with the instance limits `limits`.
    fn limited(starts: u32, shutdown_ms: u64, load: Load, limits: native::Limits) -> Supervised {
        retrying(starts, shutdown_ms, load, (limits, NEVER))
    }

    /// [`supervisor`] with the instance limits and retry backoff given.
    fn retrying(
        starts: u32,
        shutdown_ms: u64,
        load: Load,
        (limits, retry): (native::Limits, Backoff),
    ) -> Supervised {
        starting_at(
            starts,
            shutdown_ms,
            load,
            (limits, retry),
            Identifiers::default(),
        )
    }

    /// [`retrying`] with the monitor's identifiers starting at `first`.
    fn starting_at(
        starts: u32,
        shutdown_ms: u64,
        load: Load,
        (limits, retry): (native::Limits, Backoff),
        first: Identifiers,
    ) -> Supervised {
        configured(starts, shutdown_ms, load, (limits, retry), first, |_| {})
    }

    /// A supervisor over `load` with `starts` start slots and default
    /// limits, its context changed by `adjust`.
    fn adjusted(starts: u32, load: Load, adjust: impl FnOnce(&mut Context)) -> Supervised {
        let limits = climon2_core::config::Limits::default().library();
        let first = Identifiers::default();
        configured(starts, 20_000, load, (limits, NEVER), first, adjust)
    }

    /// [`starting_at`], its context changed by `adjust`.
    fn configured(
        starts: u32,
        shutdown_ms: u64,
        load: Load,
        (limits, retry): (native::Limits, Backoff),
        first: Identifiers,
        adjust: impl FnOnce(&mut Context),
    ) -> Supervised {
        let directory = tempfile::tempdir().unwrap();
        let home = HomeLayout::resolve(Some(directory.path().as_os_str()), Path::new("/")).unwrap();
        let log = climon2_platform::logging::LogFile::monitor(directory.path()).unwrap();
        let context = Context {
            load,
            home,
            runtime: directory.path().join("run"),
            escape: ControlKey::from_byte(0x1d).unwrap(),
            take_control: None,
            limits,
            filter: "info".into(),
            lanes: Lanes {
                loads: 1,
                starts,
                newer_start_burst: 8,
                load_ms: 10_000,
                shutdown_ms: 0,
                final_host_ms: 0,
            },
            shutdown: Duration::from_millis(shutdown_ms),
            retry,
            first,
            admission: climon2_core::config::Limits::default().admission(),
            outside: 0,
        };
        let mut context = context;
        adjust(&mut context);
        let dispatch = log.dispatch(Some("info")).unwrap();
        let supervisor = Supervisor::start(context, dispatch).unwrap();
        Supervised {
            directory,
            supervisor,
            log: log.path().to_owned(),
        }
    }

    impl Supervisor {
        /// Shuts down and waits, at most [`WAIT`], for what that proved.
        fn stop(&mut self) -> shutdown::Report {
            self.shut_down();
            let deadline = Instant::now() + WAIT;
            loop {
                if let Some(report) = self.report() {
                    return report;
                }
                assert!(Instant::now() < deadline, "the supervisor never finished");
                thread::sleep(Duration::from_millis(1));
            }
        }
    }

    fn fake_library() -> Load {
        Box::new(|_| Ok(FAKE_LIBRARY))
    }

    /// The next report, waiting at most [`WAIT`].
    fn next(supervisor: &Supervisor) -> (SessionId, ClientEvent) {
        supervisor
            .reports
            .recv_timeout(WAIT)
            .expect("the supervisor reported nothing")
    }

    fn unavailable(event: &ClientEvent) -> &str {
        match event {
            ClientEvent::Unavailable(why) => why,
            other => panic!("not Unavailable: {other:?}"),
        }
    }

    /// With one start slot, `failing` (created first) fails as `script` says,
    /// then `healthy` starts and opens. The failed instance is destroyed
    /// successfully before exit, so its deliveries and both handles were
    /// released, and the healthy one ends at exit, confirmed in time.
    fn a_failure_ends_cleanly_and_frees_its_start_slot(test: u8, failing: Script) -> String {
        let (doomed, healthy) = (session(test, 1), session(test, 2));
        script(&doomed, failing);
        let mut supervised = supervisor(1, 20_000, fake_library());
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&doomed, "2026-11-13T10:00:02.000Z"));
        supervisor.start_client(metadata(&healthy, "2026-11-13T10:00:01.000Z"));
        let (id, event) = next(supervisor);
        assert_eq!(id, doomed);
        let why = unavailable(&event).to_owned();
        let retrying = ClientEvent::Recovery(Recovery::Retrying(why.clone()));
        assert_eq!(next(supervisor), (doomed.clone(), retrying));
        assert_eq!(
            next(supervisor),
            (healthy.clone(), ClientEvent::Opened { status: false })
        );
        let destroyed = |id: &SessionId| {
            let id = guid(id);
            move |world: &World| world.seen.contains(&(id, Seen::OwnerExited))
        };
        until(destroyed(&doomed));
        assert_eq!(
            seen(&guid(&doomed)),
            [
                Seen::Created,
                Seen::Destroyed(native::SUCCESS),
                Seen::OwnerExited
            ]
        );
        let report = supervisor.stop();
        let (_, evidence) = report
            .instances
            .iter()
            .find(|(id, _)| *id == healthy)
            .unwrap();
        assert!(evidence.current && evidence.joined, "{evidence:?}");
        assert_eq!(report.verdict(), shutdown::Verdict::Clean);
        assert_eq!(
            seen(&guid(&healthy)),
            [
                Seen::Created,
                Seen::Destroyed(native::SUCCESS),
                Seen::OwnerExited
            ]
        );
        let rest: Vec<_> = supervisor.reports.try_iter().collect();
        assert_eq!(rest, [(healthy, ClientEvent::Ended)]);
        assert!(
            !supervised.log().contains(UNCONFIRMED),
            "{}",
            supervised.log()
        );
        why
    }

    /// Client supervisor (Fault, #106; REC-01 (Monitor, Fault): retried
    /// passively): a failed notification lookup ends that
    /// instance cleanly: its row is Unavailable with the library's error, its
    /// pending deliveries and handles are released and it is destroyed, and its
    /// start slot goes to the next session.
    #[test]
    fn a_failed_notification_lookup_ends_its_client_cleanly_and_frees_its_start_slot() {
        let why = a_failure_ends_cleanly_and_frees_its_start_slot(
            1,
            Script {
                notification_fails: true,
                ..Script::default()
            },
        );
        assert!(why.contains("no notification"), "{why}");
    }

    /// Client supervisor (Fault, #106; REC-01: retried passively): a run that
    /// fails without terminal
    /// evidence ends that instance cleanly: its row is Unavailable with run's
    /// error, its handles are released and it is destroyed, and its start slot
    /// goes to the next session.
    #[test]
    fn a_failed_run_without_terminal_evidence_ends_its_client_cleanly_and_frees_its_start_slot() {
        let why = a_failure_ends_cleanly_and_frees_its_start_slot(
            2,
            Script {
                run_fails: true,
                ..Script::default()
            },
        );
        assert!(why.contains("run failed"), "{why}");
    }

    /// Client supervisor (Fault, #106; LIB-05 (Monitor, Fault): the other
    /// instance progresses; REC-01: retried passively): a failed drain ends
    /// that instance cleanly: its
    /// row is Unavailable with drain's error, its handles are released and it
    /// is destroyed, and its start slot goes to the next session.
    #[test]
    fn a_failed_drain_ends_its_client_cleanly_and_frees_its_start_slot() {
        let why = a_failure_ends_cleanly_and_frees_its_start_slot(
            11,
            Script {
                drain_fails: true,
                ..Script::default()
            },
        );
        assert_eq!(why, "cannot drain the client: internal_error: drain failed");
    }

    /// Client supervisor (Fault, #106; shutdown bound), EXT-02 (Fault, #118):
    /// an instance that ignores shutdown holds exit until
    /// `deadlines.shutdown_ms` and never past it, while shutdown says it
    /// waits for that session; it is never destroyed while its handles are
    /// unreleased: its owner leaves it to the fail-stop, its evidence
    /// claims no terminal evidence, release, destroy or join, and the
    /// monitor log says so.
    #[test]
    fn exit_is_bounded_and_never_destroys_an_instance_with_unreleased_handles() {
        let stalled = session(3, 1);
        script(
            &stalled,
            Script {
                stalls: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(1, 200, fake_library());
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&stalled, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (stalled.clone(), ClientEvent::Opened { status: false })
        );
        let started = Instant::now();
        let report = supervisor.stop();
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(200), "{took:?}");
        assert!(took < Duration::from_secs(10), "{took:?}");
        assert_eq!(supervisor.waiting(), Some(vec![stalled.clone()]));
        let unconfirmed = Evidence {
            current: true,
            published: true,
            ..Evidence::default()
        };
        assert_eq!(report.instances, [(stalled.clone(), unconfirmed)]);
        assert!(report.unfinished.is_empty(), "{report:?}");
        let log = supervised.log();
        assert!(log.contains(&format!("{UNCONFIRMED} clients=1")), "{log}");
        let id = guid(&stalled);
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        assert_eq!(seen(&id), [Seen::Created, Seen::OwnerExited]);
    }

    /// EXT-02 (Fault, #118; #455): a round's wait ends at the shutdown
    /// deadline, never a tick past it, so the supervisor's report is ready
    /// while the UI thread still waits for it: a tick while one is left,
    /// what is left of it near the deadline, in whole milliseconds, and none
    /// once it has passed.
    #[test]
    fn a_round_waits_no_further_than_the_shutdown_deadline() {
        let now = Instant::now();
        assert_eq!(within(now + Duration::from_secs(60)), TICK);
        assert!(within(now + Duration::from_millis(5)) <= Duration::from_millis(5));
        assert!(within(now + Duration::from_micros(1500)) <= Duration::from_millis(2));
        let near = within(now + Duration::from_micros(12_700));
        assert_eq!(near.subsec_nanos() % 1_000_000, 0, "{near:?}");
        assert_eq!(within(now), Duration::ZERO);
    }

    /// EXT-02 (Fault, #118; #455): an instance that has deliveries without
    /// end once shutdown is asked is drained only until the shutdown
    /// deadline: then the supervisor reports, leaving it unconfirmed, so its
    /// report reaches the UI thread within the handover grace rather than
    /// never. When the report arrives is not asserted against the grace,
    /// since a loaded host can delay any thread.
    #[test]
    fn draining_stops_at_the_shutdown_deadline() {
        let flooding = session(0x45, 1);
        script(
            &flooding,
            Script {
                stalls: true,
                floods: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(1, 200, fake_library());
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&flooding, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (flooding.clone(), ClientEvent::Opened { status: false })
        );
        let report = supervisor.stop();
        let unconfirmed = Evidence {
            current: true,
            published: true,
            ..Evidence::default()
        };
        assert_eq!(report.instances, [(flooding.clone(), unconfirmed)]);
        let id = guid(&flooding);
        assert!(world().drains[&id] > 1);
        let rounds = supervisor.waiting.try_iter().count();
        assert!(
            rounds < 100,
            "{rounds} rounds: each should drain for a tick"
        );
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
    }

    /// EXT-02 (Fault, #118; #462): an instance with deliveries without end
    /// from the start does not keep the supervisor draining: each round
    /// returns to its commands, so it reads the shutdown and reports by the
    /// deadline, leaving the instance unconfirmed, rather than never.
    #[test]
    fn a_flood_does_not_starve_the_shutdown_command() {
        let flooding = session(0x46, 1);
        script(
            &flooding,
            Script {
                stalls: true,
                floods_at_once: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(1, 200, fake_library());
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&flooding, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (flooding.clone(), ClientEvent::Opened { status: false })
        );
        let report = supervisor.stop();
        let unconfirmed = Evidence {
            current: true,
            published: true,
            ..Evidence::default()
        };
        assert_eq!(report.instances, [(flooding.clone(), unconfirmed)]);
        let id = guid(&flooding);
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
    }

    /// Lets each instance's stalled create or run go when dropped, so a
    /// failed assertion leaves no owner thread blocked.
    struct LetGo(Vec<[u8; 16]>);

    impl Drop for LetGo {
        fn drop(&mut self) {
            changed(&mut world(), |world| world.unstalled.extend(&self.0));
        }
    }

    /// The Opened report of each of `ids`, in any order.
    fn all_opened(supervisor: &Supervisor, ids: &[SessionId]) {
        let mut opened: Vec<_> = ids.iter().map(|_| next(supervisor)).collect();
        opened.sort_by(|one, other| one.0.cmp(&other.0));
        let expected: Vec<_> = ids
            .iter()
            .map(|id| (id.clone(), ClientEvent::Opened { status: false }))
            .collect();
        assert_eq!(opened, expected);
    }

    /// EXT-01 (Fault, #118; #462): in shutdown, an instance with deliveries
    /// without end shares each round's drain bound with the others, so a
    /// healthy instance drained after it still ends confirmed.
    #[test]
    fn a_flood_in_shutdown_does_not_starve_another_instance() {
        let [flooding, healthy] = [1, 2].map(|index| session(0x48, index));
        script(
            &flooding,
            Script {
                stalls: true,
                floods: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(2, 1_000, fake_library());
        let _let_go = LetGo(vec![guid(&flooding)]);
        let supervisor = &mut supervised.supervisor;
        for id in [&flooding, &healthy] {
            supervisor.start_client(metadata(id, "2026-11-13T10:00:00.000Z"));
        }
        all_opened(supervisor, &[flooding.clone(), healthy.clone()]);
        let report = supervisor.stop();
        let [(first, flooded), (second, ended)] = &report.instances[..] else {
            panic!("{report:?}");
        };
        assert_eq!((first, second), (&flooding, &healthy));
        assert!(flooded.terminal.is_none(), "{flooded:?}");
        assert!(ended.joined, "{ended:?}");
    }

    /// EXT-02 (Fault, #118; #462): however many instances have deliveries
    /// without end, a round's drains end a tick after its wait, so a round
    /// lasts about a tick, not a tick for each (an idle instance, drained
    /// once a round, counts them), and the supervisor reads the exit
    /// within about a tick. In shutdown each instance is still drained once
    /// a round, so the idle one, whose run lasts until its terminal
    /// evidence is drained, ends confirmed among them.
    #[test]
    fn many_floods_still_let_the_exit_be_read_within_about_a_tick() {
        let flooding: Vec<_> = (1..=48).map(|index| session(0x49, index)).collect();
        for id in &flooding {
            script(
                id,
                Script {
                    stalls: true,
                    floods_at_once: true,
                    ..Script::default()
                },
            );
        }
        let idle = session(0x49, 0x7f);
        script(
            &idle,
            Script {
                waits_for_drain: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(49, 200, fake_library());
        let all: Vec<_> = flooding.iter().chain([&idle]).cloned().collect();
        let _let_go = LetGo(all.iter().map(guid).collect());
        let supervisor = &mut supervised.supervisor;
        for id in &all {
            supervisor.start_client(metadata(id, "2026-11-13T10:00:00.000Z"));
        }
        all_opened(supervisor, &all);
        let idle_id = guid(&idle);
        let earlier = world().drains[&idle_id];
        // About 20 rounds; a tick for each flood would be under one.
        thread::sleep(20 * TICK);
        let rounds = world().drains[&idle_id] - earlier;
        assert!(rounds >= 2, "{rounds} rounds in 20 ticks");
        let asked = Instant::now();
        supervisor.shut_down();
        while supervisor.waiting().is_none() {
            assert!(asked.elapsed() < WAIT, "the exit was never read");
            thread::sleep(Duration::from_millis(1));
        }
        let took = asked.elapsed();
        assert!(took < 50 * TICK, "{took:?}");
        let report = supervisor.stop();
        assert_eq!(report.instances.len(), all.len(), "{report:?}");
        let (_, ended) = report.instances.iter().find(|(id, _)| id == &idle).unwrap();
        assert!(ended.joined, "{ended:?}");
    }

    /// #462: rounds take turns to start their drains, so a busy instance
    /// after one with deliveries without end still has whole rounds to
    /// itself, not one delivery a round (an idle instance counts them).
    #[test]
    fn a_flood_does_not_starve_a_busy_instance_after_it() {
        let [flooding, busy, idle] = [1, 2, 3].map(|index| session(0x4b, index));
        for id in [&flooding, &busy] {
            script(
                id,
                Script {
                    stalls: true,
                    floods_at_once: true,
                    ..Script::default()
                },
            );
        }
        let mut supervised = supervisor(3, 200, fake_library());
        let _let_go = LetGo(vec![guid(&flooding), guid(&busy)]);
        let supervisor = &mut supervised.supervisor;
        let all = [flooding.clone(), busy.clone(), idle.clone()];
        for id in &all {
            supervisor.start_client(metadata(id, "2026-11-13T10:00:00.000Z"));
        }
        all_opened(supervisor, &all);
        let (busy_id, idle_id) = (guid(&busy), guid(&idle));
        let counts = || {
            let world = world();
            (world.drains[&busy_id], world.drains[&idle_id])
        };
        let (busy_before, idle_before) = counts();
        until(|world| world.drains[&idle_id] >= idle_before + 6);
        let (busy_after, idle_after) = counts();
        let (drained, rounds) = (busy_after - busy_before, idle_after - idle_before);
        assert!(drained > 3 * rounds, "{drained} drains in {rounds} rounds");
        supervisor.stop();
    }

    /// #462: an instance with nothing to deliver is drained once a round,
    /// not again and again until the round's bound.
    #[test]
    fn an_idle_instance_is_drained_once_a_round() {
        let idle = session(0x4a, 1);
        let mut supervised = supervisor(1, 200, fake_library());
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&idle, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (idle.clone(), ClientEvent::Opened { status: false })
        );
        let id = guid(&idle);
        let earlier = world().drains[&id];
        thread::sleep(10 * TICK);
        let drained = world().drains[&id] - earlier;
        assert!(drained < 100, "{drained} drains in about 10 rounds");
        supervisor.stop();
    }

    /// EXT-01 (Fault, #118; #462): an instance whose owner publishes it only
    /// once shutdown has begun is asked to shut down in the round that
    /// learns of it, before that round's poll, which may run to the
    /// deadline: before its stalled sibling is drained again, and it ends
    /// confirmed.
    #[test]
    fn an_instance_published_during_shutdown_is_asked_before_the_poll() {
        let [sibling, late] = [1, 2].map(|index| session(0x47, index));
        let (sibling_id, late_id) = (guid(&sibling), guid(&late));
        script(
            &sibling,
            Script {
                stalls: true,
                lets_go: Some(late_id),
                ..Script::default()
            },
        );
        script(
            &late,
            Script {
                creates_late: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(2, 1_000, fake_library());
        let _let_go = LetGo(vec![sibling_id, late_id]);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&sibling, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (sibling.clone(), ClientEvent::Opened { status: false })
        );
        supervisor.start_client(metadata(&late, "2026-11-13T10:00:00.000Z"));
        until(|world| world.seen.contains(&(late_id, Seen::Created)));
        let report = supervisor.stop();
        let asks = world().first_asks.clone();
        assert_eq!(
            asks[&late_id].get(&sibling_id),
            asks[&sibling_id].get(&sibling_id),
            "the sibling was drained before the late instance was asked"
        );
        let [(_, stalled), (asked, ended)] = &report.instances[..] else {
            panic!("{report:?}");
        };
        assert!(stalled.terminal.is_none(), "{stalled:?}");
        assert_eq!(asked, &late);
        assert!(ended.joined, "{ended:?}");
    }

    /// EXT-01 (Monitor, Fault; #118): shutdown asks every instance at once,
    /// passive, partially started (Open not yet answered), failing and
    /// stalled alike, so one stalled instance holds up no other, and
    /// proves each kind of evidence of each on its own: the terminal
    /// evidence's Leave, client outcome and closure, worker cleanup, handle
    /// release, destroy and the owner thread's join. A lost Leave fails
    /// the exit; a failed destroy and the stalled instance make it a
    /// fail-stop, named with exactly what is unconfirmed.
    #[test]
    fn shutdown_asks_every_instance_at_once_and_proves_each_kind_of_evidence_alone() {
        let [passive, partial, lost, unclean, stalled, unreleased] =
            [1, 2, 3, 4, 5, 6].map(|index| session(48, index));
        script(
            &unreleased,
            Script {
                release_fails: true,
                ..Script::default()
            },
        );
        script(
            &partial,
            Script {
                opens_late: true,
                ..Script::default()
            },
        );
        script(
            &lost,
            Script {
                leave: Some(native::LEAVE_INDETERMINATE),
                client_fails: Some(b"leave lost"),
                ..Script::default()
            },
        );
        script(
            &unclean,
            Script {
                destroy_fails: true,
                ..Script::default()
            },
        );
        script(
            &stalled,
            Script {
                stalls: true,
                ..Script::default()
            },
        );
        let mut supervised = supervisor(6, 1_000, fake_library());
        let supervisor = &mut supervised.supervisor;
        let all = [&passive, &partial, &lost, &unclean, &stalled, &unreleased];
        for (index, id) in all.iter().enumerate() {
            supervisor.start_client(metadata(id, &format!("2026-11-13T10:00:0{index}.000Z")));
        }
        let opened = reports_until(supervisor, |reports| {
            let opened = ClientEvent::Opened { status: false };
            [&passive, &lost, &unclean, &stalled, &unreleased]
                .iter()
                .all(|id| {
                    reports
                        .get(*id)
                        .is_some_and(|events| events.contains(&opened))
                })
        });
        assert!(!opened.contains_key(&partial), "{opened:?}");
        until(|world| world.looked_up.contains(&guid(&partial)));
        let report = supervisor.stop();
        let confirmed = Departed {
            failed: None,
            leave: Leave::Confirmed,
            detached: false,
            closed: true,
        };
        let clean = Evidence {
            current: true,
            published: true,
            terminal: Some(confirmed.clone()),
            workers: Some(Ok(())),
            released: true,
            destroyed: Some(Ok(())),
            joined: true,
        };
        let expected = [
            (passive.clone(), clean.clone()),
            (partial.clone(), clean.clone()),
            (
                lost.clone(),
                Evidence {
                    terminal: Some(Departed {
                        failed: Some("timeout: leave lost".into()),
                        leave: Leave::Indeterminate,
                        ..confirmed
                    }),
                    ..clean.clone()
                },
            ),
            (
                unclean.clone(),
                Evidence {
                    destroyed: Some(Err("busy: still in use".into())),
                    ..clean.clone()
                },
            ),
            (
                stalled.clone(),
                Evidence {
                    current: true,
                    published: true,
                    ..Evidence::default()
                },
            ),
            (
                unreleased.clone(),
                Evidence {
                    released: false,
                    destroyed: Some(Err("busy: still in use".into())),
                    ..clean.clone()
                },
            ),
        ];
        assert_eq!(report.instances, expected);
        assert_eq!(supervisor.waiting(), Some(vec![stalled.clone()]));
        assert!(report.unfinished.is_empty(), "{report:?}");
        assert_eq!(
            report.verdict(),
            shutdown::Verdict::FailStop(format!(
                "monitor shutdown unconfirmed: \
                 session {lost}: Leave unconfirmed, client failed: timeout: leave lost; \
                 session {unclean}: not destroyed: busy: still in use; \
                 session {stalled}: no terminal evidence, handles not released, \
                 run not returned, not destroyed, thread not joined; \
                 session {unreleased}: handles not released, not destroyed: busy: still in use"
            ))
        );
        let id = guid(&stalled);
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
    }

    /// EXT-02 (Fault, #118): a release load still under way at the
    /// shutdown deadline is reported unfinished, never joined or claimed,
    /// which makes the exit a fail-stop; it may finish later.
    #[test]
    fn a_load_unfinished_at_the_deadline_is_reported_and_never_joined() {
        let (finish, finished) = mpsc::channel::<()>();
        let (start, started) = mpsc::channel::<()>();
        let held = std::sync::Mutex::new((start, finished));
        let stalls: Load = Box::new(move |_| {
            let held = held.lock().unwrap();
            let _ = held.0.send(());
            let _ = held.1.recv();
            Ok(FAKE_LIBRARY)
        });
        let id = session(49, 1);
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 200, stalls);
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        started.recv_timeout(WAIT).unwrap();
        let report = supervisor.stop();
        assert!(report.instances.is_empty(), "{report:?}");
        assert_eq!(report.unfinished, ["release 0.1.0 load"]);
        assert_eq!(
            report.verdict(),
            shutdown::Verdict::FailStop(
                "monitor shutdown unconfirmed: release 0.1.0 load unfinished".into()
            )
        );
        finish.send(()).unwrap();
    }

    /// Client supervisor (Fault, #106; one shared load attempt per release;
    /// REC-01 (Monitor, Fault): a missing library is retried): a refused
    /// release is loaded once however many sessions use it, and each of
    /// them is Unavailable with the loader's diagnosis, retrying.
    #[test]
    fn a_refused_release_is_loaded_once_and_its_sessions_are_unavailable() {
        let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&loads);
        let refuse: Load = Box::new(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Refusal::Library(Failure::MissingLibrary, "missing".into()))
        });
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(4, 20_000, refuse);
        let sessions = [session(4, 1), session(4, 2), session(4, 3)];
        for (second, id) in sessions.iter().enumerate() {
            supervisor.start_client(metadata(id, &format!("2026-11-13T10:00:0{second}.000Z")));
        }
        let mut reported: Vec<_> = (0..6).map(|_| next(&supervisor)).collect();
        reported.sort_by(|a, b| a.0.cmp(&b.0));
        let retrying = ClientEvent::Recovery(Recovery::Retrying("missing".into()));
        let expected: Vec<_> = sessions
            .iter()
            .flat_map(|id| {
                [
                    (id.clone(), ClientEvent::Unavailable("missing".into())),
                    (id.clone(), retrying.clone()),
                ]
            })
            .collect();
        assert_eq!(reported, expected);
        supervisor.stop();
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    fn geometry(cols: u16, rows: u16) -> climon2_vt::Geometry {
        climon2_vt::Geometry::for_host(cols, rows).unwrap()
    }

    fn presented(presentation: Presentation) -> ClientEvent {
        ClientEvent::Presentation(presentation)
    }

    /// A final outcome without a terminal status time (no `status-v1`).
    fn ended(outcome: Outcome) -> ClientEvent {
        ClientEvent::Report(Report::Final(Final {
            outcome,
            changed: None,
        }))
    }

    /// Client supervisor (#108): control and display records reach the
    /// session's terminal as the library delivered them: each grant and
    /// beginning with its record's generation, period, restoration and
    /// effective geometry (one too wide for any session clamped to 500
    /// columns, one with no columns dropped), Display's exact bytes, a
    /// revocation, and the command's final result from the Initial metadata
    /// and from Command final; unread records add nothing.
    #[test]
    fn control_and_display_records_reach_the_terminal_as_delivered() {
        let id = session(5, 1);
        script(
            &id,
            Script {
                presents: true,
                ..Script::default()
            },
        );
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let authority = |generation, restoration| Authority {
            generation,
            period: 2,
            restoration,
        };
        let expected = [
            ClientEvent::Opened { status: false },
            ClientEvent::Report(Report::Title(Some("T".into()))),
            ended(Outcome::Exit(0)),
            presented(Presentation::Granted(authority(7, 3), geometry(80, 24))),
            presented(Presentation::Begin(
                authority(GENERATION, 4),
                geometry(100, 30),
            )),
            presented(Presentation::Display(DISPLAYED.to_vec())),
            presented(Presentation::Complete),
            presented(Presentation::Geometry(geometry(500, 5))),
            presented(Presentation::Revoked(Revocation::Takeover)),
            presented(Presentation::Revoked(Revocation::Ending)),
            presented(Presentation::Revoked(Revocation::Unknown)),
            ended(Outcome::Signal(15)),
        ];
        for event in expected {
            assert_eq!(next(&supervisor), (id.clone(), event));
        }
        supervisor.stop();
        assert_eq!(
            supervisor.reports.try_iter().collect::<Vec<_>>(),
            [(id, ClientEvent::Ended)]
        );
    }

    /// #502: a request for a local session reaches the supervisor, which
    /// refuses it at once while no client is open; one for a remote session
    /// never does.
    #[test]
    fn only_a_local_sessions_requests_are_routed_to_its_client() {
        let id = session(6, 1);
        let Supervised {
            supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        let relay = climon2_link::PublicKey::from_bytes([1; 32]);
        supervisor.route(&SessionAt::remote(relay, id.clone()), Asked::Acquire);
        supervisor.route(&SessionAt::local(id.clone()), Asked::Acquire);
        let refused = presented(Presentation::Answered(
            Asked::Acquire,
            Err("the session's client is not connected".into()),
        ));
        assert_eq!(next(&supervisor), (id, refused));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(supervisor.reports.try_iter().count(), 0);
    }

    /// Client supervisor (#108): Acquire, Ready and Resize are submitted with
    /// increasing request IDs after Open's and their native arguments, and each
    /// answer reaches the terminal as its own request, accepted or refused with
    /// why; an answer to no request adds nothing, and a request for a session
    /// without an open client is refused at once.
    #[test]
    fn requests_are_submitted_in_order_and_answered_as_themselves() {
        let id = session(6, 1);
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        supervisor.request(id.clone(), Asked::Acquire);
        let refused = presented(Presentation::Answered(
            Asked::Acquire,
            Err("the session's client is not connected".into()),
        ));
        assert_eq!(next(&supervisor), (id.clone(), refused));
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(&supervisor),
            (id.clone(), ClientEvent::Opened { status: false })
        );
        let held = Authority {
            generation: 1,
            period: 5,
            restoration: 6,
        };
        let size = TerminalSize {
            cols: 120,
            rows: 29,
        };
        let requests = [
            Asked::Acquire,
            Asked::Ready(held),
            Asked::Resize(held, size),
        ];
        for request in requests {
            supervisor.request(id.clone(), request.clone());
            let answer = if matches!(request, Asked::Ready(_)) {
                Err("stale".to_owned())
            } else {
                Ok(())
            };
            let answered = presented(Presentation::Answered(request, answer));
            assert_eq!(next(&supervisor), (id.clone(), answered));
        }
        supervisor.stop();
        assert_eq!(
            submitted(&id),
            [
                (native::ACQUIRE, 2, vec![GENERATION]),
                (native::READY, 3, vec![1, 5, 6]),
                (native::RESIZE, 4, vec![1, 5, 6, 120, 29]),
            ]
        );
    }

    /// Client supervisor (#108): a request for a session whose client has
    /// published its handles but not yet opened is refused at once as not
    /// connected, submits nothing and consumes no request ID; once Open
    /// succeeds, the same request is submitted as the first after Open's.
    /// The request is sent once the supervisor has looked up the client's
    /// descriptor, which it does as the client becomes Opening.
    #[test]
    fn a_request_before_the_client_opens_is_refused_as_not_connected() {
        let id = session(14, 1);
        script(
            &id,
            Script {
                opens_late: true,
                ..Script::default()
            },
        );
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let published = guid(&id);
        until(|world| world.looked_up.contains(&published));
        supervisor.request(id.clone(), Asked::Acquire);
        let refused = presented(Presentation::Answered(
            Asked::Acquire,
            Err("the session's client is not connected".into()),
        ));
        assert_eq!(next(&supervisor), (id.clone(), refused));
        let_go(&id);
        assert_eq!(
            next(&supervisor),
            (id.clone(), ClientEvent::Opened { status: false })
        );
        supervisor.request(id.clone(), Asked::Acquire);
        let accepted = presented(Presentation::Answered(Asked::Acquire, Ok(())));
        assert_eq!(next(&supervisor), (id.clone(), accepted));
        supervisor.stop();
        assert_eq!(
            submitted(&id),
            [(native::ACQUIRE, OPEN_ID + 1, vec![GENERATION])]
        );
    }

    /// What the fake accepted from `id`'s instance, in order.
    fn submitted(id: &SessionId) -> Vec<Submitted> {
        let id = guid(id);
        let world = world();
        let accepted = world.submitted.iter().filter(|(seen, _)| *seen == id);
        accepted.map(|(_, submitted)| submitted.clone()).collect()
    }

    /// An open client for `id` under `script`, whose pending ledger holds at
    /// most two requests: one queued record and one operation.
    fn opened_with(id: &SessionId, script_of: Script) -> Supervised {
        script(id, script_of);
        let mut limits = climon2_core::config::Limits::default().library();
        limits.request_records = 1;
        limits.operations = 1;
        let supervised = limited(1, 20_000, fake_library(), limits);
        let supervisor = &supervised.supervisor;
        supervisor.start_client(metadata(id, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (id.clone(), ClientEvent::Opened { status: false })
        );
        supervised
    }

    /// Client supervisor (#108; core-v1 "Submit", operational limits "Queues and
    /// transfers"): a request the library refuses at submit is answered at once
    /// with why, leaves nothing in the pending ledger and consumes no request
    /// ID. More refusals than the ledger holds still leave room, and the next
    /// accepted request takes the next unused ID.
    #[test]
    fn a_request_refused_at_submit_is_not_pending_and_consumes_no_id() {
        let id = session(7, 1);
        let script = Script {
            refused_submits: 3,
            ..Script::default()
        };
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = opened_with(&id, script);
        let full = presented(Presentation::Answered(
            Asked::Acquire,
            Err("full: the request queue is full".into()),
        ));
        for _ in 0..3 {
            supervisor.request(id.clone(), Asked::Acquire);
            assert_eq!(next(&supervisor), (id.clone(), full.clone()));
        }
        supervisor.request(id.clone(), Asked::Acquire);
        let accepted = presented(Presentation::Answered(Asked::Acquire, Ok(())));
        assert_eq!(next(&supervisor), (id.clone(), accepted));
        supervisor.stop();
        assert_eq!(
            submitted(&id),
            [(native::ACQUIRE, OPEN_ID + 1, vec![GENERATION])]
        );
    }

    /// Client supervisor (#108; core-v1 "bounded pending ledger"): the ledger
    /// holds at most the requests the instance may have accepted (its queued
    /// request records plus its operations); one more is refused at once, never
    /// submitted, and consumes no request ID.
    #[test]
    fn the_pending_ledger_is_bounded_by_the_requests_the_library_may_hold() {
        let id = session(8, 1);
        let script = Script {
            silent: true,
            ..Script::default()
        };
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = opened_with(&id, script);
        for _ in 0..3 {
            supervisor.request(id.clone(), Asked::Acquire);
        }
        let refused = presented(Presentation::Answered(
            Asked::Acquire,
            Err("too many requests are awaiting their answers".into()),
        ));
        assert_eq!(next(&supervisor), (id.clone(), refused));
        supervisor.stop();
        let acquire = |request_id| (native::ACQUIRE, request_id, vec![GENERATION]);
        assert_eq!(submitted(&id), [acquire(OPEN_ID + 1), acquire(OPEN_ID + 2)]);
    }

    /// COM-03 queue-full and COM-04 typed cases (#109): input is submitted under
    /// its authority in chunks of at most `chunk_bytes`, at the limit and one
    /// past, in order, and each accepted chunk is answered as itself. The first
    /// chunk the library refuses is answered at once with why, carrying it and
    /// every later byte; nothing after it is submitted, and it consumes no
    /// request ID.
    #[test]
    fn input_is_submitted_in_chunks_and_a_refusal_carries_every_unsent_byte() {
        let id = session(9, 1);
        let mut limits = climon2_core::config::Limits::default().library();
        limits.chunk_bytes = 3;
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = limited(1, 20_000, fake_library(), limits);
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(&supervisor),
            (id.clone(), ClientEvent::Opened { status: false })
        );
        let held = Authority {
            generation: 1,
            period: 5,
            restoration: 6,
        };
        let input = |bytes: &[u8]| Asked::Input(held, bytes.to_vec());
        let answered = |bytes: &[u8], answer| {
            (
                id.clone(),
                presented(Presentation::Answered(input(bytes), answer)),
            )
        };
        supervisor.request(id.clone(), input(b"abc"));
        supervisor.request(id.clone(), input(b"\0\xffAz"));
        for chunk in [&b"abc"[..], b"\0\xffA", b"z"] {
            assert_eq!(next(&supervisor), answered(chunk, Ok(())));
        }
        supervisor.request(id.clone(), input(b"xyzFghij"));
        let mut reports = [next(&supervisor), next(&supervisor)];
        reports.sort_by_key(|(_, event)| format!("{event:?}"));
        let mut expected = [
            answered(b"xyz", Ok(())),
            answered(b"Fghij", Err("full: no room".to_owned())),
        ];
        expected.sort_by_key(|(_, event)| format!("{event:?}"));
        assert_eq!(reports, expected);
        supervisor.request(id.clone(), input(b"k"));
        assert_eq!(next(&supervisor), answered(b"k", Ok(())));
        supervisor.stop();
        let words = |bytes: &[u8]| {
            let mut words = vec![1, 5, 6];
            words.extend(bytes.iter().map(|&byte| u64::from(byte)));
            words
        };
        assert_eq!(
            submitted(&id)
                .into_iter()
                .filter(|(tag, ..)| *tag == native::INPUT)
                .collect::<Vec<_>>(),
            [
                (native::INPUT, 2, words(b"abc")),
                (native::INPUT, 3, words(b"\0\xffA")),
                (native::INPUT, 4, words(b"z")),
                (native::INPUT, 5, words(b"xyz")),
                (native::INPUT, 6, words(b"k")),
            ]
        );
    }

    /// Spec #73 exit (#109, owner decision): stopping first waits until every
    /// accepted input chunk is answered, at most `deadlines.operation_ms`, so a
    /// stuck session's unanswered input holds shutdown for the bound and no
    /// longer; answered input holds it not at all. EXT-02 (owner decision on
    /// #118): that wait counts inside `deadlines.shutdown_ms`, so a shorter
    /// shutdown deadline ends it, and the instance is still asked to shut
    /// down, but is left unconfirmed.
    #[test]
    fn stopping_waits_for_accepted_input_at_most_the_operation_deadline() {
        let held = Authority {
            generation: 1,
            period: 9,
            restoration: 9,
        };
        let cases = [
            (1, true, 400, 1_000),
            (2, false, 30_000, 1_000),
            (3, true, 30_000, 400),
        ];
        for (index, silent, operation_ms, shutdown_ms) in cases {
            let id = session(10, index);
            script(
                &id,
                Script {
                    silent,
                    ..Script::default()
                },
            );
            let mut limits = climon2_core::config::Limits::default().library();
            limits.operation_ms = operation_ms;
            let Supervised {
                mut supervisor,
                directory: _home,
                ..
            } = limited(1, shutdown_ms, fake_library(), limits);
            supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
            assert_eq!(
                next(&supervisor),
                (id.clone(), ClientEvent::Opened { status: false })
            );
            supervisor.request(id.clone(), Asked::Input(held, b"z".to_vec()));
            let started = Instant::now();
            let report = supervisor.stop();
            let took = started.elapsed();
            if silent {
                assert!(took >= Duration::from_millis(400), "{took:?}");
            }
            assert!(took < Duration::from_secs(15), "{took:?}");
            let joined = report.instances.iter().all(|(_, evidence)| evidence.joined);
            assert_eq!(joined, index != 3, "{report:?}");
            if index == 3 {
                // The deadline passed while settling input: shutdown said
                // once whom it waits for.
                assert_eq!(supervisor.waiting(), Some(vec![id.clone()]));
            }
            let id = guid(&id);
            until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        }
    }

    const UNCONFIRMED: &str = "client shutdown unconfirmed at its deadline";

    /// A loader that refuses release 0.0.1 and loads the fake library for
    /// any other.
    fn refusing_old() -> Load {
        Box::new(|version| match version.as_str() {
            "0.0.1" => Err(Refusal::Library(Failure::WrongRelease, "old".into())),
            _ => Ok(FAKE_LIBRARY),
        })
    }

    /// Lets `id`'s scripted wait end.
    fn let_go(id: &SessionId) {
        let id = guid(id);
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
    }

    /// Client supervisor (#106; `scheduling.starts` at its bound): with two
    /// start slots, two sessions whose Open is unanswered both start, and a
    /// third waits, holding no client and no log directory, until one of
    /// them opens; then it starts and opens after it. A session of a refused
    /// release, newer than none of the three, is reported once the waiting
    /// session has been scheduled, so its report proves the third was held.
    #[test]
    fn client_starts_never_exceed_the_start_lane() {
        let [first, second, third] = [session(12, 1), session(12, 2), session(12, 3)];
        let [refused, marker] = [session(12, 4), session(12, 5)];
        for id in [&first, &second] {
            let opens_late = Script {
                opens_late: true,
                ..Script::default()
            };
            script(id, opens_late);
        }
        let mut supervised = supervisor(2, 20_000, refusing_old());
        let supervisor = &mut supervised.supervisor;
        let old = |id: &SessionId, created: &str| versioned(id, created, "0.0.1");
        supervisor.start_client(old(&refused, "2026-11-13T10:00:01.000Z"));
        let why = ClientEvent::Unavailable("old".into());
        let rejected = ClientEvent::Recovery(Recovery::Rejected("old".into()));
        assert_eq!(next(supervisor), (refused.clone(), why.clone()));
        assert_eq!(next(supervisor), (refused, rejected.clone()));
        supervisor.start_client(metadata(&first, "2026-11-13T10:00:09.000Z"));
        supervisor.start_client(metadata(&second, "2026-11-13T10:00:08.000Z"));
        let [one, two] = [&first, &second].map(guid);
        until(|world| {
            [one, two]
                .iter()
                .all(|id| world.seen.contains(&(*id, Seen::Created)))
        });
        supervisor.start_client(metadata(&third, "2026-11-13T10:00:05.000Z"));
        supervisor.start_client(old(&marker, "2026-11-13T10:00:04.000Z"));
        assert_eq!(next(supervisor), (marker.clone(), why));
        assert_eq!(next(supervisor), (marker, rejected));
        assert!(!supervised.started(&third));
        assert!(seen(&guid(&third)).is_empty());
        let supervisor = &mut supervised.supervisor;
        let_go(&first);
        let opened = ClientEvent::Opened { status: false };
        assert_eq!(next(supervisor), (first.clone(), opened.clone()));
        assert_eq!(next(supervisor), (third.clone(), opened.clone()));
        let_go(&second);
        assert_eq!(next(supervisor), (second.clone(), opened));
        supervisor.stop();
        let mut rest: Vec<_> = supervisor.reports.try_iter().collect();
        rest.sort_by(|a, b| a.0.cmp(&b.0));
        let ended = [first, second, third].map(|id| (id, ClientEvent::Ended));
        assert_eq!(rest, ended);
    }

    /// Client supervisor (#106): terminal evidence before Open's response
    /// makes the row Unavailable with the evidence's message, or, when the
    /// message is empty, with why it is unavailable. Each instance is then
    /// released and destroyed.
    #[test]
    fn terminal_evidence_before_open_says_why_the_session_is_unavailable() {
        let [named, silent] = [session(13, 1), session(13, 2)];
        for (id, message) in [(&named, &b"daemon refused"[..]), (&silent, b"")] {
            let ends = Script {
                ends_unopened: Some(message),
                ..Script::default()
            };
            script(id, ends);
        }
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(2, 20_000, fake_library());
        supervisor.start_client(metadata(&named, "2026-11-13T10:00:02.000Z"));
        supervisor.start_client(metadata(&silent, "2026-11-13T10:00:01.000Z"));
        let mut reported = vec![next(&supervisor), next(&supervisor)];
        reported.sort_by(|a, b| a.0.cmp(&b.0));
        let unavailable = |why: &str| ClientEvent::Unavailable(why.into());
        assert_eq!(
            reported,
            [
                (named.clone(), unavailable("daemon refused")),
                (
                    silent.clone(),
                    unavailable("the client stopped before it opened")
                ),
            ]
        );
        for id in [&named, &silent].map(guid) {
            until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        }
        supervisor.stop();
        for id in [&named, &silent].map(guid) {
            let destroyed = [
                Seen::Created,
                Seen::Destroyed(native::SUCCESS),
                Seen::OwnerExited,
            ];
            assert_eq!(seen(&id), destroyed);
        }
    }

    /// What `supervisor` reports of each session until `done` holds of all
    /// of it, waiting at most [`WAIT`] for each report; presentation is
    /// left out.
    fn reports_until(
        supervisor: &Supervisor,
        mut done: impl FnMut(&BTreeMap<SessionId, Vec<ClientEvent>>) -> bool,
    ) -> BTreeMap<SessionId, Vec<ClientEvent>> {
        let mut reports = BTreeMap::<_, Vec<_>>::new();
        while !done(&reports) {
            let (id, event) = next(supervisor);
            if !matches!(event, ClientEvent::Presentation(_)) {
                reports.entry(id).or_default().push(event);
            }
        }
        reports
    }

    /// When `id`'s instances reached `seen`, in order.
    fn times(id: &SessionId, seen: Seen) -> Vec<Instant> {
        let id = guid(id);
        let world = world();
        let times = world.times.iter();
        times
            .filter(|(at, what, _)| *at == id && *what == seen)
            .map(|(_, _, when)| *when)
            .collect()
    }

    /// The delays `log` says the supervisor scheduled for `id`'s retries.
    fn scheduled(log: &str, id: &SessionId) -> Vec<u64> {
        let session = format!("session={id}");
        log.lines()
            .filter(|line| line.contains("client retry scheduled") && line.contains(&session))
            .map(|line| {
                let delay = line.split("delay_ms=").nth(1).unwrap();
                delay.split_whitespace().next().unwrap().parse().unwrap()
            })
            .collect()
    }

    /// The connection generation each of `id`'s instances was created in.
    fn generations(id: &SessionId) -> Vec<u64> {
        let id = guid(id);
        let world = world();
        let generations = world.generations.iter();
        generations
            .filter(|(made, _)| *made == id)
            .map(|(_, generation)| *generation)
            .collect()
    }

    /// REC-01 (Monitor, Fault: startup failure and transport loss; passive
    /// retry), COM-05 (Monitor, Fault: a fresh attempt never overlaps the
    /// old one): a client failing before it opens is retried, each attempt
    /// a fresh instance in the next connection generation, created only
    /// once the one before was destroyed and its owner joined, after a
    /// backoff from `retry.initial_ms` doubling to `retry.cap_ms` (100, 200,
    /// then 300 ms at the cap rather than 400). The row hears each failure
    /// with its diagnostic and a retry, then Starting for each attempt,
    /// until one opens; a later loss is retried again from the initial
    /// delay.
    #[test]
    fn a_failed_client_is_retried_after_its_cleanup_in_a_fresh_generation() {
        let id = session(20, 1);
        let failing = Script {
            ends_unopened: Some(b"transport io_error: refused"),
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        let lost = Script {
            drops: true,
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        scripts(&id, &[failing, failing, failing, lost, Script::default()]);
        let backoff = Backoff {
            initial_ms: 100,
            cap_ms: 300,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(1, 20_000, fake_library(), (limits, backoff));
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        let reports = reports_until(supervisor, |reports| {
            reports
                .get(&id)
                .is_some_and(|events| events.iter().filter(|event| **event == opened).count() == 2)
        });
        let why = "transport io_error: refused";
        let failed = [
            ClientEvent::Unavailable(why.into()),
            ClientEvent::Recovery(Recovery::Retrying(why.into())),
            ClientEvent::Starting,
        ];
        let mut expected: Vec<_> = failed.iter().cycle().take(9).cloned().collect();
        expected.extend([
            opened.clone(),
            ClientEvent::Ended,
            ClientEvent::Recovery(Recovery::Retrying(
                "the client stopped before it opened".into(),
            )),
            ClientEvent::Starting,
            opened,
        ]);
        assert_eq!(reports[&id], expected);
        assert_eq!(generations(&id), [1, 2, 3, 4, 5]);
        let attempt = [
            Seen::Created,
            Seen::Destroyed(native::SUCCESS),
            Seen::OwnerExited,
        ];
        let mut lives: Vec<_> = attempt.iter().cycle().take(12).copied().collect();
        lives.push(Seen::Created);
        assert_eq!(seen(&guid(&id)), lives);
        let (created, exited) = (times(&id, Seen::Created), times(&id, Seen::OwnerExited));
        let delays = [100, 200, 300, 100];
        for (index, least) in delays.into_iter().enumerate() {
            let waited = created[index + 1] - exited[index];
            assert!(
                waited >= Duration::from_millis(least),
                "{index}: {waited:?}"
            );
            assert!(waited < Duration::from_secs(10), "{index}: {waited:?}");
        }
        supervisor.stop();
        assert_eq!(scheduled(&supervised.log(), &id), delays);
    }

    /// REC-01 (Monitor, Fault: permanent rejection and invalid
    /// configuration; a known final outcome), LIB-05 (cleanup that cannot
    /// be confirmed): a client the library rejects permanently, a create
    /// refusing its limits, a client whose final outcome is known before it
    /// is lost and one whose destroy fails are never retried, nor one whose
    /// Open the daemon refused and whose evidence says so for good, which
    /// is Unavailable with the refusal; a create refused for want of
    /// resources and a client the monitor stopped itself (its notification
    /// lookup failed) are retried and open.
    #[test]
    fn rejections_known_outcomes_and_unconfirmed_cleanup_retry_nothing() {
        let [
            rejected,
            limited_,
            finished,
            unclean,
            scarce,
            unpolled,
            refusing,
        ] = [1, 2, 3, 4, 5, 6, 7].map(|index| session(21, index));
        let ends = |retry| Script {
            ends_unopened: Some(b"daemon unsupported_capability"),
            retry,
            ..Script::default()
        };
        script(&rejected, ends(native::RETRY_PERMANENT));
        let refused = |code| Script {
            create_fails: Some(code),
            ..Script::default()
        };
        script(&limited_, refused(native::UNSUPPORTED_LIMIT));
        let presents = Script {
            presents: true,
            drops: true,
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        script(&finished, presents);
        let unconfirmed = Script {
            destroy_fails: true,
            ..ends(native::RETRY_PASSIVE)
        };
        script(&unclean, unconfirmed);
        scripts(
            &scarce,
            &[refused(native::RESOURCE_LIMIT), Script::default()],
        );
        let lookup_fails = Script {
            notification_fails: true,
            ..Script::default()
        };
        scripts(&unpolled, &[lookup_fails, Script::default()]);
        let refuses = Script {
            refuses_open: true,
            retry: native::RETRY_PERMANENT,
            ..Script::default()
        };
        script(&refusing, refuses);
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(7, 20_000, fake_library(), (limits, quick));
        let supervisor = &mut supervised.supervisor;
        let all = [
            &rejected, &limited_, &finished, &unclean, &scarce, &unpolled, &refusing,
        ];
        for (index, id) in all.iter().enumerate() {
            supervisor.start_client(metadata(id, &format!("2026-11-13T10:00:0{index}.000Z")));
        }
        let unconfirmed_why = "cannot confirm the client's cleanup: busy: still in use";
        let reports = reports_until(supervisor, |reports| {
            let has = |id: &SessionId, event: &ClientEvent| {
                reports.get(id).is_some_and(|events| events.contains(event))
            };
            let rejection = ClientEvent::Recovery(Recovery::Rejected(unconfirmed_why.into()));
            let opened = ClientEvent::Opened { status: false };
            has(&scarce, &opened)
                && has(&unpolled, &opened)
                && has(&finished, &ClientEvent::Ended)
                && has(&unclean, &rejection)
                && [&rejected, &limited_, &refusing]
                    .iter()
                    .all(|id| reports.get(*id).is_some_and(|events| events.len() == 2))
        });
        let unavailable = |why: &str| ClientEvent::Unavailable(why.into());
        let rejection = |why: &str| ClientEvent::Recovery(Recovery::Rejected(why.into()));
        let retried = |why: &str| ClientEvent::Recovery(Recovery::Retrying(why.into()));
        let capability = "daemon unsupported_capability";
        assert_eq!(
            reports[&rejected],
            [unavailable(capability), rejection(capability)]
        );
        let limit = "refused: create refused";
        assert_eq!(reports[&limited_], [unavailable(limit), rejection(limit)]);
        assert_eq!(
            reports[&finished],
            [
                ClientEvent::Opened { status: false },
                ClientEvent::Report(Report::Title(Some("T".into()))),
                ended(Outcome::Exit(0)),
                ended(Outcome::Signal(15)),
                ClientEvent::Ended,
            ]
        );
        assert_eq!(
            reports[&unclean],
            [
                unavailable(capability),
                retried(capability),
                rejection(unconfirmed_why)
            ]
        );
        assert_eq!(
            reports[&scarce],
            [
                unavailable(limit),
                retried(limit),
                ClientEvent::Starting,
                ClientEvent::Opened { status: false },
            ]
        );
        let polled = "cannot poll the client: internal_error: no notification";
        assert_eq!(
            reports[&unpolled],
            [
                unavailable(polled),
                retried(polled),
                ClientEvent::Starting,
                ClientEvent::Opened { status: false },
            ]
        );
        assert_eq!(
            reports[&refusing],
            [
                unavailable("stale"),
                rejection("the client stopped before it opened")
            ]
        );
        for id in [&finished, &unclean, &refusing] {
            let id = guid(id);
            until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        }
        let report = supervisor.stop();
        let (_, earlier) = report
            .instances
            .iter()
            .find(|(id, _)| *id == unclean)
            .unwrap();
        assert!(!earlier.current && earlier.joined, "{earlier:?}");
        assert_eq!(earlier.destroyed, Some(Err("busy: still in use".into())));
        assert_eq!(
            report.verdict(),
            shutdown::Verdict::FailStop(format!(
                "monitor shutdown unconfirmed: session {unclean}: not destroyed: busy: still in use"
            ))
        );
        for id in [&rejected, &limited_, &finished, &unclean, &refusing] {
            assert_eq!(generations(id), [1], "{id}");
        }
        for id in [&scarce, &unpolled] {
            assert_eq!(generations(id), [1, 2], "{id}");
        }
    }

    /// REC-01 (Monitor, Fault: missing library, then a repaired
    /// installation; a wrong release): a release refused for a reason a
    /// later load may not have is loaded again once its session's retry is
    /// due, and its session then opens; a release refused for good is
    /// loaded once and its session never retried.
    #[test]
    fn a_missing_library_is_loaded_again_when_its_retry_is_due() {
        let [id, wrong] = [session(22, 1), session(22, 2)];
        let loads = std::sync::Arc::new(Mutex::new(Vec::new()));
        let counted = std::sync::Arc::clone(&loads);
        let repaired: Load = Box::new(move |version| {
            let mut loads = counted.lock().unwrap();
            loads.push(version.to_string());
            let earlier = loads
                .iter()
                .filter(|loaded| *loaded == version.as_str())
                .count();
            match (version.as_str(), earlier) {
                ("0.0.2", _) => Err(Refusal::Library(Failure::WrongRelease, "wrong".into())),
                (_, 1) => Err(Refusal::Library(Failure::MissingLibrary, "missing".into())),
                _ => Ok(FAKE_LIBRARY),
            }
        });
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(1, 20_000, repaired, (limits, quick));
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(versioned(&wrong, "2026-11-13T10:00:01.000Z", "0.0.2"));
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        let reports = reports_until(supervisor, |reports| {
            let opened = reports
                .get(&id)
                .is_some_and(|events| events.contains(&opened));
            opened && reports.get(&wrong).is_some_and(|events| events.len() == 2)
        });
        assert_eq!(
            reports[&id],
            [
                ClientEvent::Unavailable("missing".into()),
                ClientEvent::Recovery(Recovery::Retrying("missing".into())),
                ClientEvent::Starting,
                opened,
            ]
        );
        assert_eq!(
            reports[&wrong],
            [
                ClientEvent::Unavailable("wrong".into()),
                ClientEvent::Recovery(Recovery::Rejected("wrong".into())),
            ]
        );
        supervisor.stop();
        let mut loads = loads.lock().unwrap().clone();
        loads.sort();
        assert_eq!(loads, ["0.0.2", "0.1.0", "0.1.0"]);
    }

    /// Owner decision on #116 (2026-11-16): discovery proving a session
    /// gone drops the retry already scheduled for it.
    #[test]
    fn a_scheduled_retry_is_dropped_once_its_session_is_gone() {
        let id = session(24, 1);
        let failing = Script {
            ends_unopened: Some(b"transport io_error: refused"),
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        script(&id, failing);
        let backoff = Backoff {
            initial_ms: 100,
            cap_ms: 100,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(1, 20_000, fake_library(), (limits, backoff));
        supervised
            .supervisor
            .start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let deadline = Instant::now() + WAIT;
        while scheduled(&supervised.log(), &id).is_empty() {
            assert!(Instant::now() < deadline, "no retry was scheduled");
            thread::sleep(Duration::from_millis(5));
        }
        let supervisor = &mut supervised.supervisor;
        supervisor.forget(id.clone());
        // Three times the backoff: a retry still scheduled would start.
        thread::sleep(Duration::from_millis(300));
        supervisor.stop();
        assert_eq!(generations(&id), [1]);
        let reports: Vec<_> = supervisor
            .reports
            .try_iter()
            .map(|(_, event)| event)
            .collect();
        assert!(!reports.contains(&ClientEvent::Starting), "{reports:?}");
    }

    /// LIM-03 (Monitor, Fault: rejected updates; owner decision on #119,
    /// 2026-11-16): a session whose model refused an update has its client
    /// dropped, reported Unavailable with the model's error, and retried
    /// passively in a fresh generation once the old instance is destroyed;
    /// dropping it again once it is no longer open changes nothing.
    #[test]
    fn a_client_dropped_at_its_memory_limit_is_retried_passively() {
        let id = session(25, 1);
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(1, 20_000, fake_library(), (limits, quick));
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        assert_eq!(next(supervisor), (id.clone(), opened.clone()));
        let why = "terminal model over its limit: 9 bytes needed, 8 allowed";
        supervisor.drop_client(id.clone(), why.into());
        supervisor.drop_client(id.clone(), "again".into());
        let reports = reports_until(supervisor, |reports| {
            reports
                .get(&id)
                .is_some_and(|events| events.contains(&opened))
        });
        assert_eq!(
            reports[&id],
            [
                ClientEvent::Unavailable(why.into()),
                ClientEvent::Recovery(Recovery::Retrying(why.into())),
                ClientEvent::Starting,
                opened,
            ]
        );
        let created = guid(&id);
        assert_eq!(
            seen(&created)[..3],
            [
                Seen::Created,
                Seen::Destroyed(native::SUCCESS),
                Seen::OwnerExited
            ]
        );
        supervisor.stop();
        assert_eq!(generations(&id), [1, 2]);
    }

    /// LIM-05 (Monitor, Fault: generation exhaustion): connection
    /// generations strictly increase per session across attempts and never
    /// wrap. Starting near exhaustion, a session lost twice is created in
    /// generations u64::MAX - 1 and u64::MAX; its next attempt is never
    /// started: the row hears it Unavailable and rejected for good because
    /// its generations are exhausted.
    #[test]
    fn a_session_whose_generations_are_exhausted_is_never_attempted_again() {
        let id = session(26, 1);
        let lost = Script {
            drops: true,
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        script(&id, lost);
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let near = Identifiers {
            generation: u64::MAX - 1,
            ..Identifiers::default()
        };
        let mut supervised = starting_at(1, 20_000, fake_library(), (limits, quick), near);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let exhausted = "connection generations are exhausted";
        let rejected = ClientEvent::Recovery(Recovery::Rejected(exhausted.into()));
        let reports = reports_until(supervisor, |reports| {
            reports
                .get(&id)
                .is_some_and(|events| events.contains(&rejected))
        });
        let lost = [
            ClientEvent::Opened { status: false },
            ClientEvent::Ended,
            ClientEvent::Recovery(Recovery::Retrying(
                "the client stopped before it opened".into(),
            )),
            ClientEvent::Starting,
        ];
        let mut expected: Vec<_> = lost.iter().cycle().take(8).cloned().collect();
        expected.extend([ClientEvent::Unavailable(exhausted.into()), rejected]);
        assert_eq!(reports[&id], expected);
        supervisor.stop();
        assert_eq!(generations(&id), [u64::MAX - 1, u64::MAX]);
    }

    /// LIM-05 (Monitor, Fault: request ID exhaustion): an instance's request
    /// IDs strictly increase and never wrap. Starting near exhaustion,
    /// requests take IDs u64::MAX - 1 and u64::MAX; the next is refused at
    /// once because the IDs are exhausted, and nothing more is submitted.
    #[test]
    fn requests_after_the_last_request_id_are_refused() {
        let id = session(27, 1);
        let limits = climon2_core::config::Limits::default().library();
        let near = Identifiers {
            request: u64::MAX - 1,
            ..Identifiers::default()
        };
        let mut supervised = starting_at(1, 20_000, fake_library(), (limits, NEVER), near);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(supervisor),
            (id.clone(), ClientEvent::Opened { status: false })
        );
        let accepted = presented(Presentation::Answered(Asked::Acquire, Ok(())));
        for _ in 0..2 {
            supervisor.request(id.clone(), Asked::Acquire);
            assert_eq!(next(supervisor), (id.clone(), accepted.clone()));
        }
        supervisor.request(id.clone(), Asked::Acquire);
        let exhausted = presented(Presentation::Answered(
            Asked::Acquire,
            Err("the client's request IDs are exhausted".into()),
        ));
        assert_eq!(next(supervisor), (id.clone(), exhausted));
        supervisor.stop();
        assert_eq!(
            submitted(&id),
            [
                (native::ACQUIRE, u64::MAX - 1, vec![GENERATION]),
                (native::ACQUIRE, u64::MAX, vec![GENERATION]),
            ]
        );
    }

    /// The bytes one report for `id` with nothing carried holds.
    fn one_report(id: &SessionId) -> u64 {
        backlog::footprint(id, &ClientEvent::Ended) as u64
    }

    /// LIM-02 (Monitor, Fault: delivery congestion), with `event_bytes` the
    /// footprint of one report: once Open's success is reported and the UI
    /// has not processed it, the client's reports fill its allowance and it
    /// is not drained again, so its presentation stays in the library;
    /// after `deadlines.delivery_stall_ms` (200 ms) of that congestion, it
    /// is failed explicitly and retried passively.
    #[test]
    fn a_client_whose_reports_stay_unprocessed_is_failed_after_its_delivery_stall() {
        let id = session(28, 1);
        let presents = Script {
            presents: true,
            ..Script::default()
        };
        script(&id, presents);
        let mut limits = climon2_core::config::Limits::default().library();
        limits.event_bytes = one_report(&id);
        limits.delivery_stall_ms = 200;
        let mut supervised = limited(1, 20_000, fake_library(), limits);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        assert_eq!(next(supervisor), (id.clone(), opened));
        let since = Instant::now();
        let why = "delivery congestion lasted past 200 ms";
        assert_eq!(
            next(supervisor),
            (id.clone(), ClientEvent::Unavailable(why.into()))
        );
        assert!(
            since.elapsed() >= Duration::from_millis(150),
            "{:?}",
            since.elapsed()
        );
        assert_eq!(
            next(supervisor),
            (
                id.clone(),
                ClientEvent::Recovery(Recovery::Retrying(why.into()))
            )
        );
        supervisor.stop();
    }

    /// LIM-02 (Monitor: capacity accounting at its boundary): with
    /// `event_bytes` one byte more than a report, and with the UI
    /// processing each report as it arrives, the client keeps being
    /// drained: its whole presentation arrives and it is never failed.
    /// Exit drains a congested client regardless, so it ends well within
    /// the shutdown deadline.
    #[test]
    fn reports_the_ui_processes_leave_room_to_drain_the_client() {
        for (test, processes, room) in [(29, false, 1), (30, true, 0)] {
            let id = session(test, 1);
            let presents = Script {
                presents: true,
                ..Script::default()
            };
            script(&id, presents);
            let mut limits = climon2_core::config::Limits::default().library();
            limits.event_bytes = one_report(&id) + room;
            limits.delivery_stall_ms = 600_000;
            let mut supervised = limited(1, 20_000, fake_library(), limits);
            let supervisor = &mut supervised.supervisor;
            supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
            let mut seen = Vec::new();
            while !seen.contains(&ClientEvent::Presentation(Presentation::Complete)) {
                let (_, event) = next(supervisor);
                if processes {
                    supervisor.processed(&id, backlog::footprint(&id, &event));
                }
                seen.push(event);
            }
            assert!(
                !seen
                    .iter()
                    .any(|event| matches!(event, ClientEvent::Unavailable(_))),
                "{seen:?}"
            );
            let stopping = Instant::now();
            supervisor.stop();
            assert!(stopping.elapsed() < Duration::from_secs(10));
        }
    }

    /// LIM-02 (Monitor: capacity accounting, pure supplement): a client
    /// whose reports fill its allowance is not waited on, so its pending
    /// notification does not wake the supervisor: another, idle client is
    /// still drained about once a tick (20 ms), not continually.
    #[test]
    fn a_congested_client_does_not_wake_the_supervisor() {
        let [congested, idle] = [session(60, 1), session(60, 2)];
        let presents = Script {
            presents: true,
            ..Script::default()
        };
        script(&congested, presents);
        script(&idle, Script::default());
        let mut limits = climon2_core::config::Limits::default().library();
        limits.event_bytes = one_report(&congested);
        limits.delivery_stall_ms = 600_000;
        let mut supervised = limited(2, 20_000, fake_library(), limits);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&congested, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        assert_eq!(next(supervisor), (congested.clone(), opened.clone()));
        supervisor.start_client(metadata(&idle, "2026-11-13T10:00:01.000Z"));
        let (id, event) = next(supervisor);
        assert_eq!((&id, &event), (&idle, &opened));
        supervisor.processed(&idle, backlog::footprint(&idle, &event));
        let drains = || world().drains.get(&guid(&idle)).copied().unwrap_or(0);
        let before = drains();
        thread::sleep(Duration::from_millis(500));
        let drained = drains() - before;
        assert!(drained < 100, "{drained} drains in 500 ms");
        supervisor.stop();
    }

    /// LIM-02 (Monitor, Fault: delivery congestion; pure supplement over
    /// the fake library, #119): throttling holds back only a client whose
    /// row still follows its reports. Once its delivery stall fails it, the
    /// closing client is drained again although its unprocessed reports
    /// still fill its allowance, even while its run has not yet returned,
    /// and it is then released and destroyed.
    #[test]
    fn a_client_failed_for_congestion_is_drained_while_it_closes() {
        let id = session(61, 1);
        let stalls = Script {
            presents: true,
            stalls: true,
            ..Script::default()
        };
        script(&id, stalls);
        let mut limits = climon2_core::config::Limits::default().library();
        limits.event_bytes = one_report(&id);
        limits.delivery_stall_ms = 200;
        let mut supervised = limited(1, 20_000, fake_library(), limits);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        assert_eq!(next(supervisor), (id.clone(), opened));
        let why = "delivery congestion lasted past 200 ms";
        assert_eq!(
            next(supervisor),
            (id.clone(), ClientEvent::Unavailable(why.into()))
        );
        let instance = guid(&id);
        let drains = |world: &World| world.drains.get(&instance).copied().unwrap_or(0);
        let closed = drains(&world());
        until(|world| drains(world) > closed);
        let_go(&id);
        until(|world| world.seen.contains(&(instance, Seen::OwnerExited)));
        let destroyed = [
            Seen::Created,
            Seen::Destroyed(native::SUCCESS),
            Seen::OwnerExited,
        ];
        assert_eq!(seen(&instance), destroyed);
        supervisor.stop();
    }

    /// Owner decision on #116 (2026-11-16): once discovery proves a
    /// session gone, nothing retries it, even a client lost after that,
    /// while another session still starts and opens.
    #[test]
    fn a_session_proven_gone_is_never_retried() {
        let [gone, other] = [session(23, 1), session(23, 2)];
        let lost = Script {
            opens_late: true,
            drops: true,
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        script(&gone, lost);
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(2, 20_000, fake_library(), (limits, quick));
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&gone, "2026-11-13T10:00:00.000Z"));
        let id = guid(&gone);
        until(|world| world.seen.contains(&(id, Seen::Created)));
        supervisor.forget(gone.clone());
        let_go(&gone);
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        supervisor.start_client(metadata(&other, "2026-11-13T10:00:01.000Z"));
        let opened = ClientEvent::Opened { status: false };
        let reports = reports_until(supervisor, |reports| {
            reports
                .get(&other)
                .is_some_and(|events| events.contains(&opened))
        });
        assert_eq!(
            reports[&gone],
            [
                opened.clone(),
                ClientEvent::Ended,
                ClientEvent::Recovery(Recovery::Retrying(
                    "the client stopped before it opened".into()
                )),
            ]
        );
        supervisor.stop();
        assert_eq!(generations(&gone), [1]);
    }

    /// REC-01 (pure supplement; #106's loader notes): a library the
    /// dynamic loader could not open, a contained bootstrap failure and a
    /// load thread that could not start may load later; every other refusal
    /// is permanent.
    #[test]
    fn only_a_refusal_a_later_load_may_not_repeat_is_retried() {
        let retry = |failure| Refusal::Library(failure, String::new()).retry();
        let passive = [
            Failure::MissingLibrary,
            Failure::MissingDependency,
            Failure::IncompatibleArchitecture,
            Failure::BootstrapFailure,
        ];
        let permanent = [
            Failure::SubstitutedImage,
            Failure::MissingSymbol,
            Failure::UnsupportedRevision,
            Failure::WrongRelease,
            Failure::MalformedDescription,
        ];
        assert!(
            passive
                .into_iter()
                .all(|failure| retry(failure) == Retry::Passive)
        );
        assert!(
            permanent
                .into_iter()
                .all(|failure| retry(failure) == Retry::Permanent)
        );
        assert_eq!(Refusal::Thread("no thread".into()).retry(), Retry::Passive);
        let unqualified = Refusal::Unqualified("too many workers".into());
        assert_eq!(unqualified.retry(), Retry::Permanent);
        assert_eq!(unqualified.message(), "too many workers");
    }

    /// LIM-04 (pure supplement; #120): a refused release whose handle the
    /// dynamic loader never opened frees its reservation, as does a load
    /// thread that never started; any other handle stays loaded until exit,
    /// and its release's configured reservation with it.
    #[test]
    fn only_a_handle_the_loader_opened_keeps_its_reservation() {
        let opened = |failure| Refusal::Library(failure, String::new()).opened();
        let closed = [
            Failure::MissingLibrary,
            Failure::MissingDependency,
            Failure::IncompatibleArchitecture,
        ];
        let open = [
            Failure::BootstrapFailure,
            Failure::SubstitutedImage,
            Failure::MissingSymbol,
            Failure::UnsupportedRevision,
            Failure::WrongRelease,
            Failure::MalformedDescription,
        ];
        assert!(closed.into_iter().all(|failure| !opened(failure)));
        assert!(open.into_iter().all(opened));
        assert!(!Refusal::Thread(String::new()).opened());
        assert!(Refusal::Unqualified(String::new()).opened());
        for (test, failure, kept) in [
            (1, Failure::MissingLibrary, 0),
            (2, Failure::WrongRelease, RELEASE),
        ] {
            let id = session(45, test);
            let load: Load = Box::new(move |_| Err(Refusal::Library(failure, "no".into())));
            let mut supervised = adjusted(8, load, |_| {});
            let supervisor = &mut supervised.supervisor;
            supervisor.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
            let unavailable = ClientEvent::Unavailable("no".into());
            assert_eq!(next(supervisor), (id, unavailable));
            supervisor.stop();
            assert_eq!(supervisor.reserved(), kept);
        }
    }

    /// LIM-04 (pure supplement; #120): a supervisor that has admitted
    /// nothing reserves nothing of `memory.total_bytes`, even before its
    /// thread first schedules, so the UI's history budget starts whole.
    #[test]
    fn a_fresh_supervisor_reserves_nothing() {
        for _ in 0..4 {
            let mut supervised = supervisor(1, 20_000, fake_library());
            let supervisor = &mut supervised.supervisor;
            assert_eq!(supervisor.reserved(), 0);
            supervisor.stop();
            assert_eq!(supervisor.reserved(), 0);
        }
    }

    /// LIM-04 and the admission rate limit (#120): a resource-blocked
    /// session or release is reconsidered at most once per
    /// `retry.admission_ms`, never in a loop: with an interval of an hour,
    /// capacity freed meanwhile admits nothing, and each was blocked once.
    #[test]
    fn blocked_work_is_reconsidered_at_most_once_per_admission_interval() {
        let [first, second] = [session(46, 1), session(46, 2)];
        let drops = Script {
            drops: true,
            ..Script::default()
        };
        script(&first, drops);
        let mut supervised = adjusted(8, fake_library(), |context| {
            context.admission.total_bytes = RELEASE + INSTANCE + 1000;
            context.outside = 1000;
            context.admission.admission_ms = 3_600_000;
        });
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&first, "2026-11-16T10:00:02.000Z"));
        supervisor.start_client(metadata(&second, "2026-11-16T10:00:01.000Z"));
        until(|world| world.seen.contains(&(guid(&first), Seen::OwnerExited)));
        // Ten ticks: an earlier reconsideration would have started it.
        thread::sleep(Duration::from_millis(200));
        supervisor.stop();
        assert!(!supervised.started(&second));
        let log = supervised.log();
        assert_eq!(log.matches("client resource-blocked").count(), 1, "{log}");
        let id = session(46, 3);
        let mut blocked = adjusted(8, fake_library(), |context| {
            context.admission.release_bytes = 1 << 20;
            context.admission.total_bytes = RELEASE - 1;
            context.admission.admission_ms = 3_600_000;
        });
        blocked
            .supervisor
            .start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let (_, event) = next(&blocked.supervisor);
        assert!(matches!(event, ClientEvent::AwaitingResources(_)));
        thread::sleep(Duration::from_millis(200));
        blocked.supervisor.stop();
        let log = blocked.log();
        let once = "client library resource-blocked";
        assert_eq!(log.matches(once).count(), 1, "{log}");
    }

    /// What one instance of the fake (the shipped library's requirements)
    /// reserves by default: the library's share of `memory.client_bytes`,
    /// its owner's and two workers' 2 MiB stacks.
    const INSTANCE: u64 = 3_407_872 + 3 * (2 << 20);
    /// What its release reserves once loaded: 64 MiB.
    const RELEASE: u64 = 64 << 20;

    /// `id`'s reports, in order, until `last` (included), each within
    /// [`WAIT`]; other sessions' reports go to `others`.
    fn reports_of(
        supervisor: &Supervisor,
        id: &SessionId,
        last: &ClientEvent,
        others: &mut Vec<(SessionId, ClientEvent)>,
    ) -> Vec<ClientEvent> {
        let mut reports = Vec::new();
        while reports.last() != Some(last) {
            let (from, event) = next(supervisor);
            if from == *id {
                reports.push(event);
            } else {
                others.push((from, event));
            }
        }
        reports
    }

    /// LIM-04 (Monitor, Fault; #120): admission reserves each instance
    /// before its create. With room for the release and exactly one
    /// instance, the newer session starts and the older one is
    /// resource-blocked, saying what is short; once the first instance is
    /// cleaned up its reservation is freed, and the blocked session is
    /// admitted at its next admission interval. Every reservation the
    /// ledger holds is visible to the UI's pressure policy.
    #[test]
    fn a_session_admission_cannot_reserve_is_resource_blocked_until_capacity_is_freed() {
        let [first, second] = [session(40, 1), session(40, 2)];
        let drops = Script {
            drops: true,
            ..Script::default()
        };
        script(&first, drops);
        let mut supervised = adjusted(8, fake_library(), |context| {
            context.admission.total_bytes = RELEASE + INSTANCE;
            context.admission.admission_ms = 20;
        });
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&first, "2026-11-16T10:00:02.000Z"));
        supervisor.start_client(metadata(&second, "2026-11-16T10:00:01.000Z"));
        let opened = ClientEvent::Opened { status: false };
        let mut others = Vec::new();
        let reports = reports_of(supervisor, &second, &opened, &mut others);
        let short = "not enough memory: 9699328 bytes needed, 0 bytes available";
        assert_eq!(
            reports,
            [
                ClientEvent::AwaitingResources(short.into()),
                ClientEvent::Starting,
                opened.clone(),
            ]
        );
        assert_eq!(
            others[..2],
            [(first.clone(), opened), (first, ClientEvent::Ended)]
        );
        assert_eq!(supervisor.reserved(), RELEASE + INSTANCE);
        supervisor.stop();
    }

    /// Relaxed pre-load qualification (#120, confirmed scenario; LIM-04):
    /// a release whose configured reservation and load thread do not fit
    /// is never loaded, its sessions resource-blocked; one whose
    /// configured reservation fits is loaded, then raised to its qualified
    /// 64 MiB before any create. A raise one byte short leaves its session
    /// resource-blocked, its library loaded once however often it is
    /// reconsidered; with the raise admitted, an instance one byte short is
    /// resource-blocked, and at exactly the raised reservation and one
    /// instance it starts.
    #[test]
    fn a_release_is_raised_to_its_qualified_reservation_before_any_create() {
        let loads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counting = |loads: &Arc<std::sync::atomic::AtomicUsize>| -> Load {
            let loads = Arc::clone(loads);
            Box::new(move |_| {
                loads.fetch_add(1, Ordering::SeqCst);
                Ok(FAKE_LIBRARY)
            })
        };
        let configured_mib = |total: u64| {
            move |context: &mut Context| {
                context.admission.release_bytes = 1 << 20;
                context.admission.total_bytes = total;
                context.admission.admission_ms = 1;
            }
        };
        let id = session(41, 1);
        let mut unloaded = adjusted(8, counting(&loads), configured_mib((3 << 20) - 1));
        let unloaded = &mut unloaded.supervisor;
        unloaded.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let short = "not enough memory: 3145728 bytes needed, 3145727 bytes available";
        let blocked = ClientEvent::AwaitingResources(short.into());
        assert_eq!(next(unloaded), (id.clone(), blocked));
        unloaded.stop();
        assert_eq!(loads.load(Ordering::SeqCst), 0);
        let id = session(41, 2);
        let mut short = adjusted(8, counting(&loads), configured_mib(RELEASE - 1));
        let unraised = &mut short.supervisor;
        unraised.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let why = "not enough memory: 67108864 bytes needed, 67108863 bytes available";
        let blocked = ClientEvent::AwaitingResources(why.into());
        assert_eq!(next(unraised), (id.clone(), blocked));
        // Reconsidered every millisecond meanwhile.
        thread::sleep(Duration::from_millis(50));
        unraised.stop();
        assert!(unraised.reports.try_iter().next().is_none());
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert!(seen(&guid(&id)).is_empty());
        let id = session(41, 3);
        let mut no_instance = adjusted(8, counting(&loads), configured_mib(RELEASE + INSTANCE - 1));
        let raised = &mut no_instance.supervisor;
        raised.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let why = "not enough memory: 9699328 bytes needed, 9699327 bytes available";
        let blocked = ClientEvent::AwaitingResources(why.into());
        assert_eq!(next(raised), (id.clone(), blocked));
        raised.stop();
        assert_eq!(loads.load(Ordering::SeqCst), 2);
        let id = session(41, 4);
        let mut exact = adjusted(8, counting(&loads), configured_mib(RELEASE + INSTANCE));
        let raised = &mut exact.supervisor;
        raised.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        assert_eq!(next(raised), (id, opened));
        assert_eq!(raised.reserved(), RELEASE + INSTANCE);
        raised.stop();
        assert_eq!(loads.load(Ordering::SeqCst), 3);
    }

    /// Relaxed qualification and LIM-01 (Monitor; #120): a release that
    /// needs more workers for each client than
    /// `resources.max_workers_per_client` allows is rejected explicitly
    /// and for good, before any create; the limit is never raised.
    #[test]
    fn too_many_qualified_workers_is_an_explicit_rejection() {
        let id = session(42, 1);
        let mut supervised = adjusted(8, fake_library(), |context| {
            context.admission.max_workers_per_client = 1;
        });
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let why = "the release needs 2 worker threads for each client; \
                   monitor.limits.resources.max_workers_per_client allows 1";
        assert_eq!(
            next(supervisor),
            (id.clone(), ClientEvent::Unavailable(why.into()))
        );
        let rejected = ClientEvent::Recovery(Recovery::Rejected(why.into()));
        assert_eq!(next(supervisor), (id.clone(), rejected));
        supervisor.stop();
        assert!(seen(&guid(&id)).is_empty());
    }

    /// LIM-04 (Monitor, operating-system limits; #120): descriptors are
    /// admitted only within the process's descriptor limit less
    /// `resources.descriptor_headroom`, so with a headroom past any limit
    /// nothing is loaded and the session is resource-blocked.
    #[test]
    fn the_descriptor_headroom_below_the_os_limit_bounds_admission() {
        let id = session(43, 1);
        let mut supervised = adjusted(8, fake_library(), |context| {
            context.admission.descriptor_headroom = u32::MAX;
        });
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let why = "not enough file descriptors: 4 needed, 0 available";
        let blocked = ClientEvent::AwaitingResources(why.into());
        assert_eq!(next(supervisor), (id, blocked));
        supervisor.stop();
    }

    /// DSC-02 (Monitor, Fault: one release stalled) and the owner decision
    /// on `deadlines.load_ms` (#120): a load past it is logged once and its
    /// waiting session told `LoadingSlowly`, but never aborted; meanwhile
    /// sessions of a release already loaded still start, and once the load
    /// returns its session starts too.
    #[test]
    fn a_slow_load_is_reported_and_never_blocks_a_loaded_release() {
        let [loaded, stalled, older] = [session(44, 1), session(44, 2), session(44, 3)];
        let (release, held) = mpsc::channel::<()>();
        let held = std::sync::Mutex::new(held);
        let load: Load = Box::new(move |version| {
            if version.as_str() == "0.0.9" {
                let _ = held.lock().unwrap().recv();
            }
            Ok(FAKE_LIBRARY)
        });
        let mut supervised = adjusted(8, load, |context| {
            context.lanes.loads = 2;
            context.lanes.load_ms = 30;
        });
        let supervisor = &mut supervised.supervisor;
        let opened = ClientEvent::Opened { status: false };
        supervisor.start_client(metadata(&loaded, "2026-11-16T10:00:02.000Z"));
        assert_eq!(next(supervisor), (loaded.clone(), opened.clone()));
        supervisor.start_client(versioned(&stalled, "2026-11-16T10:00:03.000Z", "0.0.9"));
        assert_eq!(
            next(supervisor),
            (stalled.clone(), ClientEvent::LoadingSlowly)
        );
        supervisor.start_client(metadata(&older, "2026-11-16T10:00:01.000Z"));
        assert_eq!(next(supervisor), (older.clone(), opened.clone()));
        release.send(()).unwrap();
        assert_eq!(next(supervisor), (stalled.clone(), ClientEvent::Starting));
        assert_eq!(next(supervisor), (stalled, opened));
        supervisor.stop();
        let log = supervised.log();
        assert_eq!(
            log.matches("client library loading slowly").count(),
            1,
            "{log}"
        );
    }

    /// The Kills `id`'s instances were submitted, by request ID.
    fn kills(id: &SessionId) -> Vec<u64> {
        let kills = submitted(id).into_iter();
        kills
            .filter(|(tag, ..)| *tag == native::KILL)
            .map(|(_, request_id, _)| request_id)
            .collect()
    }

    /// How many of `id`'s instances were submitted monitor Open.
    fn opens(id: &SessionId) -> usize {
        let id = guid(id);
        world().opens.iter().filter(|opened| **opened == id).count()
    }

    /// Whether every instance of `id` made so far was destroyed and its
    /// owner exited: a report this holds at was made after its cleanup and
    /// join.
    fn cleaned_up(id: &SessionId) -> bool {
        let seen = seen(&guid(id));
        let count = |what: Seen| seen.iter().filter(|seen| **seen == what).count();
        let made = count(Seen::Created);
        made == count(Seen::Destroyed(native::SUCCESS)) && made == count(Seen::OwnerExited)
    }

    /// What `supervisor` reports of `id`, presentation left out, until
    /// `last`, and whether `id`'s instances were all cleaned up by then.
    fn reported_until(
        supervisor: &Supervisor,
        id: &SessionId,
        last: &ClientEvent,
    ) -> (Vec<ClientEvent>, bool) {
        let reports = reports_until(supervisor, |reports| {
            reports
                .get(id)
                .is_some_and(|events| events.last() == Some(last))
        });
        (reports[id].clone(), cleaned_up(id))
    }

    fn killed_by(signal: u32) -> ClientEvent {
        ClientEvent::Report(Report::Final(Final {
            outcome: Outcome::Signal(signal),
            changed: None,
        }))
    }

    /// DEL-01 and DEL-02 (pure supplement over the fake library, #117): a
    /// confirmed Delete of a session with an open client submits one Kill
    /// through it; its termination result is the row's final outcome, the
    /// client is asked to stop, and the session is reported removed only
    /// once its instance is destroyed and its owner joined. A session
    /// without an open client, one whose Open was refused for good or one
    /// never started whose release must still load, is killed through a
    /// Kill-only instance: no Open, its one Kill as request 2, nothing
    /// reported of it but the Kill's outcome, removed after its cleanup.
    #[test]
    fn a_confirmed_kill_removes_its_session_only_after_cleanup() {
        let [open, refused, unstarted] = [session(30, 1), session(30, 2), session(30, 3)];
        let refusing = Script {
            refuses_open: true,
            retry: native::RETRY_PERMANENT,
            ..Script::default()
        };
        scripts(&refused, &[refusing, Script::default()]);
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        supervisor.start_client(metadata(&open, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        let (reports, _) = reported_until(&supervisor, &open, &opened);
        assert_eq!(reports, std::slice::from_ref(&opened));
        supervisor.remove(metadata(&open, "2026-11-13T10:00:00.000Z"), true);
        let (reports, cleaned) = reported_until(&supervisor, &open, &ClientEvent::Removed);
        assert_eq!(
            reports,
            [killed_by(9), ClientEvent::Ended, ClientEvent::Removed]
        );
        assert!(cleaned);
        assert_eq!(kills(&open), [2]);
        assert_eq!(submitted(&open).len(), 1);

        supervisor.start_client(metadata(&refused, "2026-11-13T10:00:01.000Z"));
        let rejected = ClientEvent::Recovery(Recovery::Rejected(
            "the client stopped before it opened".into(),
        ));
        let (reports, _) = reported_until(&supervisor, &refused, &rejected);
        assert_eq!(
            reports,
            [ClientEvent::Unavailable("stale".into()), rejected]
        );
        supervisor.remove(metadata(&refused, "2026-11-13T10:00:01.000Z"), true);
        let (reports, cleaned) = reported_until(&supervisor, &refused, &ClientEvent::Removed);
        assert_eq!(reports, [killed_by(9), ClientEvent::Removed]);
        assert!(cleaned);
        assert_eq!((generations(&refused), opens(&refused)), (vec![1, 2], 1));
        assert_eq!(kills(&refused), [2]);

        let fresh = versioned(&unstarted, "2026-11-13T10:00:02.000Z", "0.2.0");
        supervisor.remove(fresh, true);
        let (reports, cleaned) = reported_until(&supervisor, &unstarted, &ClientEvent::Removed);
        assert_eq!(reports, [killed_by(9), ClientEvent::Removed]);
        assert!(cleaned);
        assert_eq!((generations(&unstarted), opens(&unstarted)), (vec![1], 0));
        assert_eq!(kills(&unstarted), [2]);
        supervisor.stop();
    }

    /// DEL-01 and LIM-04 (pure supplement over the fake library; #117 with
    /// #120's admission): a Kill-only instance is reserved like any
    /// instance. With room for the release and exactly one instance, the
    /// Kill-only instance of a session resource-blocked behind an open
    /// client is refused admission too and waits; once a Delete of the open
    /// client's session removes it, freeing its reservation, the Kill-only
    /// instance is admitted, its one Kill sent as request 2, and its
    /// session removed.
    #[test]
    fn a_kill_only_instance_waits_for_admission_like_any_instance() {
        let [open, blocked] = [session(47, 1), session(47, 2)];
        let mut supervised = adjusted(8, fake_library(), |context| {
            context.admission.total_bytes = RELEASE + INSTANCE;
            context.admission.admission_ms = 20;
        });
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&open, "2026-11-16T10:00:02.000Z"));
        supervisor.start_client(metadata(&blocked, "2026-11-16T10:00:01.000Z"));
        let short = "not enough memory: 9699328 bytes needed, 0 bytes available";
        let waiting = ClientEvent::AwaitingResources(short.into());
        let mut others = Vec::new();
        let reports = reports_of(supervisor, &blocked, &waiting, &mut others);
        assert_eq!(reports, std::slice::from_ref(&waiting));
        supervisor.remove(metadata(&blocked, "2026-11-16T10:00:01.000Z"), true);
        supervisor.remove(metadata(&open, "2026-11-16T10:00:02.000Z"), true);
        let mut sequence = Vec::new();
        let reports = reports_of(supervisor, &blocked, &ClientEvent::Removed, &mut sequence);
        assert_eq!(reports, [killed_by(9), ClientEvent::Removed]);
        assert_eq!(
            sequence.last(),
            Some(&(open.clone(), ClientEvent::Removed)),
            "the Kill-only instance waited for the open client's removal"
        );
        assert_eq!((generations(&blocked), opens(&blocked)), (vec![1], 0));
        assert_eq!(kills(&blocked), [2]);
        supervisor.stop();
        let log = supervised.log();
        assert!(log.contains("Kill-only instance resource-blocked"), "{log}");
    }

    /// LIM-04 and the admission rate limit (pure supplement over the fake
    /// library; #120 with #117): a Kill-only instance admission refuses is
    /// reconsidered only once `retry.admission_ms` has passed, never in a
    /// loop: with an interval of an hour, the capacity the open client's
    /// removal frees admits nothing meanwhile, and it was blocked once.
    #[test]
    fn a_blocked_kill_only_instance_waits_out_its_admission_interval() {
        let [open, blocked] = [session(50, 1), session(50, 2)];
        let mut supervised = adjusted(8, fake_library(), |context| {
            context.admission.total_bytes = RELEASE + INSTANCE;
            context.admission.admission_ms = 3_600_000;
        });
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&open, "2026-11-16T10:00:02.000Z"));
        supervisor.start_client(metadata(&blocked, "2026-11-16T10:00:01.000Z"));
        let short = "not enough memory: 9699328 bytes needed, 0 bytes available";
        let waiting = ClientEvent::AwaitingResources(short.into());
        reports_of(supervisor, &blocked, &waiting, &mut Vec::new());
        supervisor.remove(metadata(&blocked, "2026-11-16T10:00:01.000Z"), true);
        supervisor.remove(metadata(&open, "2026-11-16T10:00:02.000Z"), true);
        let (_, cleaned) = reported_until(supervisor, &open, &ClientEvent::Removed);
        assert!(cleaned);
        // Ten ticks: an earlier reconsideration would have started it.
        thread::sleep(Duration::from_millis(200));
        supervisor.stop();
        assert!(kills(&blocked).is_empty());
        assert!(generations(&blocked).is_empty());
        let log = supervised.log();
        let once = "Kill-only instance resource-blocked";
        assert_eq!(log.matches(once).count(), 1, "{log}");
    }

    /// DEL-01 (pure supplement over the fake library; #117 with #120's
    /// start lane): a Kill-only instance takes a start slot like any start.
    /// While the only slot is held by a client still opening, a Delete of a
    /// session waiting for its start sends nothing; once that client opens
    /// and frees the slot, the Kill-only instance starts, sends its Kill and
    /// its session is removed.
    #[test]
    fn a_kill_only_instance_waits_for_a_start_slot() {
        let [opening, waiting] = [session(51, 1), session(51, 2)];
        script(
            &opening,
            Script {
                opens_late: true,
                ..Script::default()
            },
        );
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        supervisor.start_client(metadata(&opening, "2026-11-16T10:00:02.000Z"));
        let published = guid(&opening);
        until(|world| world.looked_up.contains(&published));
        supervisor.start_client(metadata(&waiting, "2026-11-16T10:00:01.000Z"));
        supervisor.remove(metadata(&waiting, "2026-11-16T10:00:01.000Z"), true);
        // Ten ticks: a Kill-only instance started meanwhile would have sent
        // its Kill.
        thread::sleep(Duration::from_millis(200));
        assert!(kills(&waiting).is_empty());
        let_go(&opening);
        let (reports, cleaned) = reported_until(&supervisor, &waiting, &ClientEvent::Removed);
        assert_eq!(reports, [killed_by(9), ClientEvent::Removed]);
        assert!(cleaned);
        assert_eq!(kills(&waiting), [2]);
        supervisor.stop();
    }

    /// LIM-05 and DEL-02 (pure supplement over the fake library; #119 with
    /// #117): a Kill takes its client's next request ID, which never wraps.
    /// With the last ID free the Kill is sent as that ID; with every ID used
    /// it is refused unsent, the row learns why, and the Delete ends.
    #[test]
    fn a_kill_after_the_last_request_id_is_refused_unsent() {
        let [last, spent] = [session(52, 1), session(52, 2)];
        let limits = climon2_core::config::Limits::default().library();
        let near = Identifiers {
            request: u64::MAX,
            ..Identifiers::default()
        };
        let mut supervised = starting_at(2, 20_000, fake_library(), (limits, NEVER), near);
        let supervisor = &mut supervised.supervisor;
        let opened = ClientEvent::Opened { status: false };
        supervisor.start_client(metadata(&last, "2026-11-16T10:00:00.000Z"));
        assert_eq!(next(supervisor), (last.clone(), opened.clone()));
        supervisor.remove(metadata(&last, "2026-11-16T10:00:00.000Z"), true);
        let (reports, _) = reported_until(supervisor, &last, &ClientEvent::Removed);
        assert_eq!(
            reports,
            [killed_by(9), ClientEvent::Ended, ClientEvent::Removed]
        );
        assert_eq!(kills(&last), [u64::MAX]);
        supervisor.start_client(metadata(&spent, "2026-11-16T10:00:01.000Z"));
        assert_eq!(next(supervisor), (spent.clone(), opened));
        supervisor.request(spent.clone(), Asked::Acquire);
        let accepted = presented(Presentation::Answered(Asked::Acquire, Ok(())));
        assert_eq!(next(supervisor), (spent.clone(), accepted));
        supervisor.remove(metadata(&spent, "2026-11-16T10:00:01.000Z"), true);
        let why = "the client's request IDs are exhausted";
        let refused = ClientEvent::Termination(Termination::Refused(why.into()));
        assert_eq!(next(supervisor), (spent.clone(), refused));
        supervisor.stop();
        assert!(kills(&spent).is_empty());
    }

    /// LIM-05 and DEL-02 (pure supplement over the fake library; #119 with
    /// #117): a Kill-only instance needs a fresh connection generation, so
    /// once a session's are exhausted its Kill is refused unsent, the
    /// instance reservation admission made for it is freed, and the Delete
    /// ends.
    #[test]
    fn a_kill_only_instance_without_a_generation_refuses_its_kill() {
        let id = session(53, 1);
        let lost = Script {
            drops: true,
            retry: native::RETRY_PERMANENT,
            ..Script::default()
        };
        script(&id, lost);
        let limits = climon2_core::config::Limits::default().library();
        let near = Identifiers {
            generation: u64::MAX,
            ..Identifiers::default()
        };
        let mut supervised = starting_at(1, 20_000, fake_library(), (limits, NEVER), near);
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-16T10:00:00.000Z"));
        let rejected = ClientEvent::Recovery(Recovery::Rejected(
            "the client stopped before it opened".into(),
        ));
        let (reports, _) = reported_until(supervisor, &id, &rejected);
        assert_eq!(
            reports,
            [
                ClientEvent::Opened { status: false },
                ClientEvent::Ended,
                rejected
            ]
        );
        supervisor.remove(metadata(&id, "2026-11-16T10:00:00.000Z"), true);
        let exhausted = "connection generations are exhausted";
        let refused = ClientEvent::Termination(Termination::Refused(exhausted.into()));
        assert_eq!(next(supervisor), (id.clone(), refused));
        supervisor.stop();
        assert_eq!(generations(&id), [u64::MAX]);
        assert!(kills(&id).is_empty());
        assert_eq!(supervisor.reserved(), RELEASE);
    }

    /// DEL-02 (pure supplement over the fake library, #117): a refused Kill
    /// ends its Delete at once, the client serving on; a Kill-only
    /// instance's refusal, or its release's, ends it too, and a retry the
    /// Delete held is started again. A lost Kill keeps the Delete: the
    /// connection then lost, only a passive client in a fresh generation
    /// learns the outcome, never another Kill, and the session is removed
    /// once that client reports its final result and is cleaned up.
    #[test]
    fn a_refused_kill_ends_its_delete_and_a_lost_one_is_never_sent_again() {
        let [refused, lost, old] = [session(31, 1), session(31, 2), session(31, 4)];
        let refusing = Script {
            kills: Killing::Refuses,
            ..Script::default()
        };
        script(&refused, refusing);
        let losing = Script {
            kills: Killing::Loses,
            drops_after_kill: true,
            ..Script::default()
        };
        let ending = Script {
            presents: true,
            ..Script::default()
        };
        scripts(&lost, &[losing, ending]);
        let soon = Backoff {
            initial_ms: 50,
            cap_ms: 50,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = retrying(1, 20_000, refusing_old(), (limits, soon));
        let opened = ClientEvent::Opened { status: false };
        supervisor.start_client(metadata(&refused, "2026-11-13T10:00:00.000Z"));
        reported_until(&supervisor, &refused, &opened);
        supervisor.remove(metadata(&refused, "2026-11-13T10:00:00.000Z"), true);
        let refusal = ClientEvent::Termination(Termination::Refused("refused".into()));
        assert_eq!(next(&supervisor), (refused.clone(), refusal.clone()));
        supervisor.request(refused.clone(), Asked::Acquire);
        let answered = Presentation::Answered(Asked::Acquire, Ok(()));
        assert_eq!(
            next(&supervisor),
            (refused.clone(), ClientEvent::Presentation(answered))
        );

        supervisor.start_client(metadata(&lost, "2026-11-13T10:00:01.000Z"));
        reported_until(&supervisor, &lost, &opened);
        supervisor.remove(metadata(&lost, "2026-11-13T10:00:01.000Z"), true);
        let (reports, cleaned) = reported_until(&supervisor, &lost, &ClientEvent::Removed);
        let unconfirmed = ClientEvent::Termination(Termination::Unconfirmed("timeout".into()));
        assert_eq!(reports[0], unconfirmed);
        assert!(reports.contains(&ClientEvent::Starting), "{reports:?}");
        let finals = reports
            .iter()
            .filter(|event| matches!(event, ClientEvent::Report(Report::Final(_))));
        assert_eq!(finals.count(), 2, "{reports:?}");
        assert!(cleaned);
        assert_eq!((generations(&lost), kills(&lost)), (vec![1, 2], vec![2]));

        supervisor.start_client(versioned(&old, "2026-11-13T10:00:03.000Z", "0.0.1"));
        let (reports, _) = reported_until(
            &supervisor,
            &old,
            &ClientEvent::Recovery(Recovery::Rejected("old".into())),
        );
        assert_eq!(reports[0], ClientEvent::Unavailable("old".into()));
        supervisor.remove(versioned(&old, "2026-11-13T10:00:03.000Z", "0.0.1"), true);
        let refused_old = ClientEvent::Termination(Termination::Refused("old".into()));
        assert_eq!(next(&supervisor), (old.clone(), refused_old));
        // Nothing held a start or retry of it, so nothing starts again.
        supervisor.request(old.clone(), Asked::Acquire);
        let unconnected = Presentation::Answered(
            Asked::Acquire,
            Err("the session's client is not connected".into()),
        );
        assert_eq!(
            next(&supervisor),
            (old.clone(), ClientEvent::Presentation(unconnected))
        );
        supervisor.stop();
        assert!(generations(&old).is_empty());
    }

    /// DEL-01 and DEL-02 (pure supplement over the fake library, #117): a
    /// final or gone session's Delete sends no Kill: an open client is
    /// asked to stop and the session reported removed after its cleanup,
    /// one with no instance at once. A Delete whose instance's cleanup
    /// cannot be confirmed is never reported removed but unconfirmed, with
    /// why. A Kill-only instance that ends without answering leaves the
    /// Kill unconfirmed, learned passively by a scheduled retry.
    #[test]
    fn a_final_or_gone_session_is_removed_without_a_kill_after_its_cleanup() {
        let [open, unstarted, stuck, ignored, held, scheduled_retry] = [
            session(32, 1),
            session(32, 2),
            session(32, 3),
            session(32, 4),
            session(32, 5),
            session(32, 6),
        ];
        let dropping = Script {
            drops: true,
            retry: native::RETRY_PASSIVE,
            ..Script::default()
        };
        let refusing_kill = Script {
            kills: Killing::Refuses,
            ..Script::default()
        };
        let lingering = Script {
            destroy_waits: true,
            ..dropping
        };
        scripts(&held, &[lingering, refusing_kill, Script::default()]);
        scripts(
            &scheduled_retry,
            &[dropping, refusing_kill, Script::default()],
        );
        let stuck_script = Script {
            destroy_fails: true,
            ..Script::default()
        };
        script(&stuck, stuck_script);
        let refusing = Script {
            refuses_open: true,
            retry: native::RETRY_PERMANENT,
            ..Script::default()
        };
        let ignoring = Script {
            kills: Killing::Ignores,
            ..Script::default()
        };
        scripts(&ignored, &[refusing, ignoring]);
        let supervised = supervisor(1, 20_000, fake_library());
        let Supervised { supervisor, .. } = &supervised;
        let opened = ClientEvent::Opened { status: false };
        supervisor.start_client(metadata(&open, "2026-11-13T10:00:00.000Z"));
        reported_until(supervisor, &open, &opened);
        supervisor.remove(metadata(&open, "2026-11-13T10:00:00.000Z"), false);
        let (reports, cleaned) = reported_until(supervisor, &open, &ClientEvent::Removed);
        assert_eq!(reports, [ClientEvent::Ended, ClientEvent::Removed]);
        assert!(cleaned);
        assert!(submitted(&open).is_empty());

        supervisor.remove(metadata(&unstarted, "2026-11-13T10:00:01.000Z"), false);
        assert_eq!(next(supervisor), (unstarted.clone(), ClientEvent::Removed));
        assert!(generations(&unstarted).is_empty());

        supervisor.start_client(metadata(&stuck, "2026-11-13T10:00:02.000Z"));
        reported_until(supervisor, &stuck, &opened);
        supervisor.remove(metadata(&stuck, "2026-11-13T10:00:02.000Z"), false);
        let cleanup = ClientEvent::Termination(Termination::Unconfirmed(
            "cannot confirm the client's cleanup: busy: still in use".into(),
        ));
        let (reports, _) = reported_until(supervisor, &stuck, &cleanup);
        assert_eq!(reports, [ClientEvent::Ended, cleanup]);

        supervisor.start_client(metadata(&ignored, "2026-11-13T10:00:03.000Z"));
        let rejected = ClientEvent::Recovery(Recovery::Rejected(
            "the client stopped before it opened".into(),
        ));
        reported_until(supervisor, &ignored, &rejected);
        supervisor.remove(metadata(&ignored, "2026-11-13T10:00:03.000Z"), true);
        let unanswered = ClientEvent::Termination(Termination::Unconfirmed(
            "the client ended before the Kill was answered".into(),
        ));
        let (reports, cleaned) = reported_until(supervisor, &ignored, &unanswered);
        assert_eq!(reports, [unanswered]);
        assert!(cleaned);
        // Held's instance is over and its retry owed, but its owner is not
        // yet done when the Delete begins; Scheduled's retry is already
        // scheduled. Either is started again once the Kill is refused.
        supervisor.start_client(metadata(&held, "2026-11-13T10:00:04.000Z"));
        supervisor.start_client(metadata(&scheduled_retry, "2026-11-13T10:00:05.000Z"));
        let retrying = ClientEvent::Recovery(Recovery::Retrying(
            "the client stopped before it opened".into(),
        ));
        let reports = reports_until(supervisor, |reports| {
            [&held, &scheduled_retry].iter().all(|id| {
                reports
                    .get(*id)
                    .is_some_and(|events| events.last() == Some(&retrying))
            })
        });
        assert_eq!(
            reports[&held],
            [opened.clone(), ClientEvent::Ended, retrying.clone()]
        );
        let deadline = Instant::now() + WAIT;
        while scheduled(&supervised.log(), &scheduled_retry).is_empty() {
            assert!(Instant::now() < deadline, "no retry was scheduled");
            thread::sleep(Duration::from_millis(10));
        }
        supervisor.remove(metadata(&held, "2026-11-13T10:00:04.000Z"), true);
        supervisor.remove(metadata(&scheduled_retry, "2026-11-13T10:00:05.000Z"), true);
        let_go(&held);
        let refused = ClientEvent::Termination(Termination::Refused("refused".into()));
        let reports = reports_until(supervisor, |reports| {
            [&held, &scheduled_retry].iter().all(|id| {
                reports
                    .get(*id)
                    .is_some_and(|events| events.last() == Some(&opened))
            })
        });
        for id in [&held, &scheduled_retry] {
            assert_eq!(
                reports[id],
                [refused.clone(), ClientEvent::Starting, opened.clone()]
            );
            assert_eq!(generations(id), [1, 2, 3]);
            assert_eq!(kills(id), [2]);
        }

        let mut supervised = supervised;
        supervised.supervisor.stop();
        assert_eq!(scheduled(&supervised.log(), &ignored).len(), 1);
        assert_eq!(kills(&ignored), [2]);
    }

    /// DEL-01 and DEL-02 (pure supplement over the fake library, #117): a
    /// Kill refused at submit, through an open client or as a Kill-only
    /// instance's only request, is refused, never sent; a pending Kill
    /// keeps its own request ID, so a later request takes the next. A
    /// Delete holds a waiting first start, never others: removed with no
    /// instance, it is never started, while the session waiting beside it
    /// starts once a start slot is free. A Delete whose Kill was refused
    /// is over even once the session's own final results arrive: shutdown
    /// reports no removal.
    #[test]
    fn refused_kills_end_their_delete_and_a_delete_holds_only_its_own_start() {
        let [
            refused_open,
            refused_only,
            pending,
            late,
            waiting,
            held,
            ended,
        ] = [
            session(33, 1),
            session(33, 2),
            session(33, 3),
            session(33, 4),
            session(33, 5),
            session(33, 6),
            session(33, 7),
        ];
        let at_submit = Script {
            kills: Killing::RefusedAtSubmit,
            ..Script::default()
        };
        script(&refused_open, at_submit);
        let refusing = Script {
            refuses_open: true,
            retry: native::RETRY_PERMANENT,
            ..Script::default()
        };
        scripts(&refused_only, &[refusing, at_submit]);
        let ignoring = Script {
            kills: Killing::Ignores,
            ..Script::default()
        };
        script(&pending, ignoring);
        let opening_late = Script {
            opens_late: true,
            ..Script::default()
        };
        script(&late, opening_late);
        let refusing_then_ending = Script {
            kills: Killing::Refuses,
            presents_after_kill: true,
            ..Script::default()
        };
        script(&ended, refusing_then_ending);
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        let opened = ClientEvent::Opened { status: false };
        let full =
            ClientEvent::Termination(Termination::Refused("full: no room for a kill".into()));
        supervisor.start_client(metadata(&refused_open, "2026-11-13T10:00:00.000Z"));
        reported_until(&supervisor, &refused_open, &opened);
        supervisor.remove(metadata(&refused_open, "2026-11-13T10:00:00.000Z"), true);
        assert_eq!(next(&supervisor), (refused_open.clone(), full.clone()));

        supervisor.start_client(metadata(&refused_only, "2026-11-13T10:00:01.000Z"));
        let rejected = ClientEvent::Recovery(Recovery::Rejected(
            "the client stopped before it opened".into(),
        ));
        reported_until(&supervisor, &refused_only, &rejected);
        supervisor.remove(metadata(&refused_only, "2026-11-13T10:00:01.000Z"), true);
        assert_eq!(next(&supervisor), (refused_only.clone(), full));
        assert!(kills(&refused_open).is_empty() && kills(&refused_only).is_empty());

        supervisor.start_client(metadata(&pending, "2026-11-13T10:00:02.000Z"));
        reported_until(&supervisor, &pending, &opened);
        supervisor.remove(metadata(&pending, "2026-11-13T10:00:02.000Z"), true);
        supervisor.request(pending.clone(), Asked::Acquire);
        let answered = Presentation::Answered(Asked::Acquire, Ok(()));
        assert_eq!(
            next(&supervisor),
            (pending.clone(), ClientEvent::Presentation(answered))
        );
        assert_eq!(
            submitted(&pending),
            [(native::KILL, 2, vec![]), (native::ACQUIRE, 3, vec![1])]
        );

        supervisor.start_client(metadata(&late, "2026-11-13T10:00:03.000Z"));
        let late_id = guid(&late);
        until(|world| world.seen.contains(&(late_id, Seen::Created)));
        supervisor.start_client(metadata(&waiting, "2026-11-13T10:00:04.000Z"));
        supervisor.start_client(metadata(&held, "2026-11-13T10:00:05.000Z"));
        supervisor.remove(metadata(&held, "2026-11-13T10:00:05.000Z"), false);
        assert_eq!(next(&supervisor), (held.clone(), ClientEvent::Removed));
        let_go(&late);
        reported_until(&supervisor, &waiting, &opened);
        assert!(generations(&held).is_empty());

        supervisor.start_client(metadata(&ended, "2026-11-13T10:00:06.000Z"));
        reported_until(&supervisor, &ended, &opened);
        supervisor.remove(metadata(&ended, "2026-11-13T10:00:06.000Z"), true);
        let killed = ClientEvent::Report(Report::Final(Final {
            outcome: Outcome::Signal(15),
            changed: None,
        }));
        let (reports, _) = reported_until(&supervisor, &ended, &killed);
        assert_eq!(
            reports[0],
            ClientEvent::Termination(Termination::Refused("refused".into()))
        );
        supervisor.stop();
        let after: Vec<_> = supervisor.reports.try_iter().collect();
        assert!(
            !after.contains(&(ended.clone(), ClientEvent::Removed)),
            "{after:?}"
        );
    }

    /// DEL-02 (pure supplement over the fake library, #117): only a retry
    /// owed when a Delete's instance ends waits again once its Kill is
    /// refused. A session rejected for good, deleted, whose Kill-only
    /// instance (never retried) ends with its Kill refused, is never
    /// started again.
    #[test]
    fn a_refused_kill_restarts_nothing_its_instances_were_not_owed() {
        let rejected_for_good = session(34, 1);
        let refusing = Script {
            refuses_open: true,
            retry: native::RETRY_PERMANENT,
            ..Script::default()
        };
        let at_submit = Script {
            kills: Killing::RefusedAtSubmit,
            ..Script::default()
        };
        scripts(&rejected_for_good, &[refusing, at_submit]);
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(1, 20_000, fake_library());
        let created = "2026-11-13T10:00:00.000Z";
        supervisor.start_client(metadata(&rejected_for_good, created));
        let rejected = ClientEvent::Recovery(Recovery::Rejected(
            "the client stopped before it opened".into(),
        ));
        reported_until(&supervisor, &rejected_for_good, &rejected);
        supervisor.remove(metadata(&rejected_for_good, created), true);
        let full =
            ClientEvent::Termination(Termination::Refused("full: no room for a kill".into()));
        assert_eq!(next(&supervisor), (rejected_for_good.clone(), full));
        // Stopping joins every owner, so the Kill-only instance's end, and
        // any restart it caused, is reported by then.
        supervisor.stop();
        let after: Vec<_> = supervisor.reports.try_iter().collect();
        assert!(after.is_empty(), "{after:?}");
        assert_eq!(generations(&rejected_for_good), [1, 2]);
    }
}
