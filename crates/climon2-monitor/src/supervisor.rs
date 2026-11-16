//! The basic client supervisor (spec #73, "Client supervisor"; #106): one
//! passive monitoring client per live session, matched by full session ID.
//!
//! Its own thread loads each release on a worker thread through the shared
//! validated loader, one attempt per release and at most `scheduling.loads`
//! at once, and starts clients newest created first, then ascending session
//! ID, at most `scheduling.starts` at once. Each instance has a dedicated
//! owner thread (`owner.rs`) that creates it, submits monitor Open,
//! publishes its handles and runs it. The supervisor polls the borrowed
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
//! shutdown retry nothing. Resource admission and fairness arrive with #120.
mod backoff;
mod client;
mod owner;
mod stream;

use crate::registry::Report;
use crate::{
    registry::ClientEvent,
    terminal::{Presentation, Request},
};
use client::{Client, Drain, Retry, Stop};
use climon2_core::{
    SessionId, Timestamp, VersionName,
    config::{Backoff, Lanes},
    keys::ControlKey,
    layout::HomeLayout,
    metadata::SessionMetadata,
    native,
};
use climon2_platform::library::{Failure, Functions};
use owner::StartupData;
use std::{
    cmp::Reverse,
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use stream::Stream;

/// How long one wait on the notification descriptors lasts before commands
/// are read again.
const TICK_MILLISECONDS: i32 = 20;

/// Loads a release's validated client library: the one shared load of
/// `climon2_platform::library::Loader` in the monitor.
pub(crate) type Load =
    Box<dyn Fn(&VersionName) -> Result<&'static Functions, Refusal> + Send + Sync>;

/// Why a release cannot be used. The loader's [`Failure`] says whether a
/// later load could succeed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Refusal {
    /// The validated loader refused the library.
    Library(Failure, String),
    /// No worker thread could be started to load it.
    Thread(String),
}

impl Refusal {
    fn message(&self) -> &str {
        match self {
            Self::Library(_, message) | Self::Thread(message) => message,
        }
    }

    /// A library the dynamic loader could not open (missing, missing a
    /// dependency or of another architecture), a contained bootstrap
    /// failure and a load that could not start may load later, once the
    /// installation is repaired; any other refusal is permanent.
    fn retry(&self) -> Retry {
        match self {
            Self::Library(
                Failure::MissingLibrary
                | Failure::MissingDependency
                | Failure::IncompatibleArchitecture
                | Failure::BootstrapFailure,
                _,
            )
            | Self::Thread(_) => Retry::Passive,
            Self::Library(..) => Retry::Permanent,
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
}

enum Command {
    Start(Box<SessionMetadata>),
    Request(SessionId, Request),
    Forget(SessionId),
    Shutdown,
}

/// The running supervisor, as the UI thread holds it.
pub(crate) struct Supervisor {
    commands: Sender<Command>,
    pub(crate) reports: Receiver<(SessionId, ClientEvent)>,
    thread: Option<JoinHandle<()>>,
}

impl Supervisor {
    pub(crate) fn start(context: Context, dispatch: tracing::Dispatch) -> std::io::Result<Self> {
        let (commands, received) = mpsc::channel();
        let (report, reports) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("supervisor".into())
            .spawn(move || {
                tracing::dispatcher::with_default(&dispatch, || {
                    State::new(context, report).serve(&received);
                });
            })?;
        Ok(Self {
            commands,
            reports,
            thread: Some(thread),
        })
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

    /// Discovery proved `id`'s daemon gone: nothing retries it any more
    /// (owner decision on #116, 2026-11-16).
    pub(crate) fn forget(&self, id: SessionId) {
        let _ = self.commands.send(Command::Forget(id));
    }

    /// Waits for accepted input to be answered (at most
    /// `deadlines.operation_ms`), then shuts every client down, waiting at
    /// most `deadlines.shutdown_ms` (and
    /// one tick) for their terminal evidence, released handles and joined
    /// owners. Instances unconfirmed by then are logged and left, never
    /// destroyed, to process exit; the fail-stop arrives with #118.
    pub(crate) fn stop(&mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// What owner and load threads tell the supervisor.
enum Internal {
    Loaded(VersionName, Release),
    Published(SessionId, Client),
    /// Run returned, or create failed, with why if it failed and whether
    /// that is retried.
    Stopped(SessionId, Result<(), (String, Retry)>),
    /// The owner is done, with whether it destroyed the instance.
    OwnerDone(SessionId, Result<(), String>),
}

enum Release {
    Loading,
    Loaded(&'static Functions),
    Refused(Refusal),
}

impl Release {
    fn log(&self, version: &VersionName) {
        match self {
            Self::Loading => {}
            Self::Loaded(_) => tracing::info!(%version, "client library loaded"),
            Self::Refused(refusal) => {
                tracing::warn!(%version, ?refusal, "client library refused");
            }
        }
    }
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

struct Instance {
    metadata: SessionMetadata,
    phase: Phase,
    /// Whether it is retried, decided once it is over.
    retry: Option<Retry>,
    /// The command's final outcome came through it: nothing retries it.
    finished: bool,
    /// Accepted requests awaiting their answers, by request ID: at most
    /// [`State::ledger`].
    pending: BTreeMap<u64, Request>,
    /// The next request ID; Open is 1.
    next: u64,
    /// Its application input: chunks, pastes and what a truncated paste
    /// owes.
    stream: Stream,
    /// Tells the owner its handles are released, so it may destroy.
    released: Sender<()>,
    owner: JoinHandle<()>,
}

struct State {
    context: Arc<Context>,
    report: Sender<(SessionId, ClientEvent)>,
    internal: Sender<Internal>,
    internals: Receiver<Internal>,
    waiting: BTreeMap<(Reverse<Timestamp>, SessionId), SessionMetadata>,
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
    /// The most requests one instance's pending ledger holds: every request
    /// its library may have accepted, queued or under way.
    ledger: usize,
}

/// What scheduling does for one waiting session.
enum Step {
    Load(VersionName),
    Refuse(String, Retry),
    Start(&'static Functions),
}

impl State {
    fn new(context: Context, report: Sender<(SessionId, ClientEvent)>) -> Self {
        let (internal, internals) = mpsc::channel();
        let limits = context.limits;
        let accepted = limits.request_records as usize + limits.operations as usize;
        Self {
            ledger: accepted,
            context: Arc::new(context),
            report,
            internal,
            internals,
            waiting: BTreeMap::new(),
            releases: BTreeMap::new(),
            instances: BTreeMap::new(),
            retrying: BTreeMap::new(),
            attempts: BTreeMap::new(),
            generations: BTreeMap::new(),
            forgotten: BTreeSet::new(),
        }
    }

    /// Rounds of commands, internal events, scheduling, then a bounded poll
    /// and drains, until shutdown or the UI thread is gone.
    fn serve(mut self, commands: &Receiver<Command>) {
        while self.commands(commands) {
            self.internals();
            self.schedule();
            self.poll();
        }
        self.settle_input();
        self.shut_down();
    }

    /// Before shutting down, waits until every input chunk an open client
    /// accepted is answered, at most `deadlines.operation_ms` (owner decision
    /// on #109): typed keys read with prefix+x still reach their session, and
    /// a stuck one holds exit no longer than one operation may take.
    fn settle_input(&mut self) {
        let bound = Duration::from_millis(self.context.limits.operation_ms.into());
        let deadline = Instant::now() + bound;
        while self.input_pending() && Instant::now() < deadline {
            self.internals();
            self.poll();
        }
    }

    /// Whether an open client has accepted input not yet answered.
    fn input_pending(&self) -> bool {
        self.instances.values().any(|instance| {
            matches!(instance.phase, Phase::Open(_))
                && instance
                    .pending
                    .values()
                    .any(|request| matches!(request, Request::Input(..)))
        })
    }

    /// Takes every pending command; false once shutdown is asked for.
    fn commands(&mut self, commands: &Receiver<Command>) -> bool {
        loop {
            match commands.try_recv() {
                Ok(command) => {
                    if !self.command(command) {
                        return false;
                    }
                }
                Err(error) => return error == TryRecvError::Empty,
            }
        }
    }

    /// Acts on one command; false for shutdown.
    fn command(&mut self, command: Command) -> bool {
        match command {
            Command::Start(metadata) => self.wait(*metadata),
            Command::Request(id, request) => self.submit(&id, request),
            Command::Forget(id) => self.forget(id),
            Command::Shutdown => return false,
        }
        true
    }

    /// Retries of a session proven gone stop, now and later.
    fn forget(&mut self, id: SessionId) {
        self.retrying.remove(&id);
        self.waiting.retain(|(_, waiting), _| *waiting != id);
        self.forgotten.insert(id);
    }

    fn wait(&mut self, metadata: SessionMetadata) {
        let key = (
            Reverse(metadata.created_at.clone()),
            metadata.session_id.clone(),
        );
        self.waiting.insert(key, metadata);
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
        let request_id = instance.next;
        live.client.submit(request_id, &request)?;
        instance.next += 1;
        instance.pending.insert(request_id, request);
        Ok(())
    }

    /// A request's answer goes to the session's terminal as its request.
    fn answer(&mut self, id: &SessionId, request_id: u64, answer: Result<(), String>) {
        let pending = self
            .instances
            .get_mut(id)
            .map(|instance| &mut instance.pending);
        if let Some(request) = pending.and_then(|pending| pending.remove(&request_id)) {
            self.report(
                id,
                ClientEvent::Presentation(Presentation::Answered(request, answer)),
            );
            self.settle(id);
        }
    }

    fn internals(&mut self) {
        while let Ok(internal) = self.internals.try_recv() {
            self.internal(internal);
        }
    }

    fn internal(&mut self, internal: Internal) {
        match internal {
            Internal::Loaded(version, release) => {
                release.log(&version);
                self.releases.insert(version, release);
            }
            Internal::Published(id, client) => self.published(&id, client),
            Internal::Stopped(id, ran) => self.stopped(&id, ran),
            Internal::OwnerDone(id, destroyed) => self.owner_done(&id, destroyed),
        }
    }

    /// Starts loads and clients the lanes have room for, newest created
    /// first, once each retry's backoff has passed. A session waiting on
    /// its release holds no start slot.
    fn schedule(&mut self) {
        self.due();
        let keys: Vec<_> = self.waiting.keys().cloned().collect();
        for key in keys {
            if let Some(step) = self.step(&self.waiting[&key].climon2_version) {
                self.take(key, step);
            }
        }
    }

    /// What a session of `version` can do now, if anything.
    fn step(&self, version: &VersionName) -> Option<Step> {
        let lanes = self.context.lanes;
        match self.releases.get(version) {
            None => room(self.loading(), lanes.loads).then(|| Step::Load(version.clone())),
            Some(Release::Loading) => None,
            Some(Release::Refused(refusal)) => {
                Some(Step::Refuse(refusal.message().to_owned(), refusal.retry()))
            }
            Some(Release::Loaded(functions)) => {
                room(self.starting(), lanes.starts).then_some(Step::Start(functions))
            }
        }
    }

    fn take(&mut self, key: (Reverse<Timestamp>, SessionId), step: Step) {
        match step {
            Step::Load(version) => self.load(version),
            Step::Refuse(why, retry) => {
                let metadata = self.waiting.remove(&key).expect("a waiting session");
                self.unstarted(metadata, why, retry);
            }
            Step::Start(functions) => {
                let metadata = self.waiting.remove(&key).expect("a waiting session");
                self.spawn_owner(functions, metadata);
            }
        }
    }

    fn loading(&self) -> u32 {
        let releases = self.releases.values();
        releases
            .filter(|release| matches!(release, Release::Loading))
            .count() as u32
    }

    fn starting(&self) -> u32 {
        let starting = self.instances.values();
        starting
            .filter(|instance| instance.phase.holds_slot())
            .count() as u32
    }

    fn load(&mut self, version: VersionName) {
        self.releases.insert(version.clone(), Release::Loading);
        let context = Arc::clone(&self.context);
        let internal = self.internal.clone();
        let loading = version.clone();
        let spawned = thread::Builder::new().name("load".into()).spawn(move || {
            let release = (context.load)(&loading).map_or_else(Release::Refused, Release::Loaded);
            let _ = internal.send(Internal::Loaded(loading, release));
        });
        if let Err(error) = spawned {
            let refused = Release::Refused(Refusal::Thread(error.to_string()));
            refused.log(&version);
            self.releases.insert(version, refused);
        }
    }

    /// Starts an instance for `metadata`'s session in its next connection
    /// generation, the first 1.
    fn spawn_owner(&mut self, functions: &'static Functions, metadata: SessionMetadata) {
        let id = metadata.session_id.clone();
        tracing::info!(session = %id, version = %metadata.climon2_version, "client starting");
        let generation = self.generations.get(&id).map_or(1, |latest| latest + 1);
        self.generations.insert(id.clone(), generation);
        let (released, release) = mpsc::channel();
        let internal = self.internal.clone();
        let owned = id.clone();
        let started = StartupData::new(&self.context, &metadata, generation);
        let spawned = started.and_then(|startup| {
            thread::Builder::new()
                .name("client".into())
                .spawn(move || owner::owner(functions, &startup, owned, &internal, &release))
        });
        match spawned {
            Ok(owner) => {
                let instance = Instance {
                    metadata,
                    phase: Phase::Creating,
                    retry: None,
                    finished: false,
                    pending: BTreeMap::new(),
                    next: client::OPEN_ID + 1,
                    stream: Stream::default(),
                    released,
                    owner,
                };
                self.instances.insert(id, instance);
            }
            Err(error) => self.unstarted(metadata, error.to_string(), Retry::Passive),
        }
    }

    /// A session whose client could not start: Unavailable, and retried
    /// unless that is permanent.
    fn unstarted(&mut self, metadata: SessionMetadata, why: String, retry: Retry) {
        let id = metadata.session_id.clone();
        self.tell(&id, ClientEvent::Unavailable(why.clone()));
        self.recover(&id, why, retry);
        if retry == Retry::Passive {
            self.backoff(metadata);
        }
    }

    /// Tells the row whether its client is retried, if it is or never is.
    fn recover(&self, id: &SessionId, why: String, retry: Retry) {
        if let Some(recovery) = retry.recovery(why) {
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
    /// waits for its start, its release loaded again if a later load could
    /// succeed.
    fn due(&mut self) {
        let now = Instant::now();
        let due: Vec<_> = self
            .retrying
            .iter()
            .filter(|(_, (at, _))| *at <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            let (_, metadata) = self.retrying.remove(&id).expect("a due retry");
            let version = &metadata.climon2_version;
            if let Some(Release::Refused(refusal)) = self.releases.get(version)
                && refusal.retry() == Retry::Passive
            {
                self.releases.remove(version);
            }
            self.tell(&id, ClientEvent::Starting);
            self.wait(metadata);
        }
    }

    /// The owner published its handles: look up the descriptor, or fail the
    /// instance without one.
    fn published(&mut self, id: &SessionId, client: Client) {
        let lookup = client.notification();
        let descriptor = lookup.as_ref().ok().copied();
        self.set(id, Phase::Opening(Live { client, descriptor }));
        tracing::info!(session = %id, "client published");
        if let Err(error) = lookup {
            self.fail(id, format!("cannot poll the client: {error}"));
        }
    }

    /// Waits one tick on every descriptor, then drains every live instance.
    fn poll(&mut self) {
        let mut table: Vec<_> = self
            .instances
            .values()
            .filter_map(|instance| instance.phase.live()?.descriptor)
            .map(|fd| libc::pollfd {
                fd,
                events: libc::POLLIN,
                // Output only.
                revents: Default::default(),
            })
            .collect();
        wait(&mut table);
        let live: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.phase.live().is_some())
            .map(|(id, _)| id.clone())
            .collect();
        for id in live {
            self.drain(&id);
        }
    }

    /// Drains `id` until it has nothing more, reporting every delivery.
    fn drain(&mut self, id: &SessionId) {
        while let Some(live) = self.phase(id).and_then(Phase::live) {
            match live.client.drain(id) {
                Drain::Events(events) => events.into_iter().for_each(|event| self.event(id, event)),
                Drain::Stop(stop) => return self.stop(id, stop),
            }
        }
    }

    fn stop(&mut self, id: &SessionId, stop: Stop) {
        match stop {
            Stop::Empty => {}
            Stop::Terminal(message, retry) => self.terminal(id, message, retry),
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
    }

    /// An open client starts its session's backoff afresh; a final outcome
    /// means nothing retries it.
    fn settle_retry(&mut self, id: &SessionId, event: &ClientEvent) {
        match event {
            ClientEvent::Opened { .. } => {
                self.attempts.remove(id);
            }
            ClientEvent::Report(Report::Final(_)) => {
                if let Some(instance) = self.instances.get_mut(id) {
                    instance.finished = true;
                }
            }
            _ => {}
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
        self.tell(id, ClientEvent::Unavailable(why));
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
    fn stopped(&mut self, id: &SessionId, ran: Result<(), (String, Retry)>) {
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
        if let Some(live) = phase.live() {
            live.client.release(id);
        }
        self.set(id, Phase::Done);
        if let Some(instance) = self.instances.get(id) {
            let _ = instance.released.send(());
        }
        if let Some(event) = phase.over(why.clone()) {
            self.tell(id, event);
        }
        self.decide(id, why, retry);
    }

    /// The owner destroyed its instance, or could not, and is joined: only
    /// then may a passive retry be scheduled. One whose cleanup cannot be
    /// confirmed is never replaced (the fail-stop is #118's).
    fn owner_done(&mut self, id: &SessionId, destroyed: Result<(), String>) {
        let Some(instance) = self.instances.remove(id) else {
            return;
        };
        let _ = instance.owner.join();
        if instance.retry != Some(Retry::Passive) {
            return;
        }
        match destroyed {
            Ok(()) => self.backoff(instance.metadata),
            Err(error) => {
                let why = format!("cannot confirm the client's cleanup: {error}");
                self.recover(id, why, Retry::Permanent);
            }
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

    fn tell(&self, id: &SessionId, event: ClientEvent) {
        let _ = self.report.send((id.clone(), event));
    }

    /// Asks every instance to stop, again each round so that late
    /// publishers hear it too, and drains until each has ended and its owner
    /// is joined, or `deadlines.shutdown_ms` passes. Dropping the state then
    /// closes every unconfirmed owner's release channel, so it never
    /// destroys an instance whose handles may be unreleased.
    fn shut_down(mut self) {
        self.waiting.clear();
        let deadline = Instant::now() + self.context.shutdown;
        while !self.instances.is_empty()
            && deadline.checked_duration_since(Instant::now()).is_some()
        {
            let live = self
                .instances
                .values()
                .filter_map(|instance| instance.phase.live());
            live.for_each(|live| live.client.shutdown());
            self.internals();
            self.poll();
        }
        if !self.instances.is_empty() {
            tracing::error!(
                clients = self.instances.len(),
                "client shutdown unconfirmed at its deadline"
            );
        }
    }
}

/// Whether a lane of `bound` has room beside the `used` slots it holds.
fn room(used: u32, bound: u32) -> bool {
    used < bound
}

/// Waits one tick for any of `table`'s descriptors, or simply one tick.
fn wait(table: &mut [libc::pollfd]) {
    if table.is_empty() {
        thread::sleep(Duration::from_millis(TICK_MILLISECONDS as u64));
        return;
    }
    // SAFETY: poll reads and updates exactly `table`'s entries.
    unsafe {
        libc::poll(
            table.as_mut_ptr(),
            table.len() as libc::nfds_t,
            TICK_MILLISECONDS,
        )
    };
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
    use crate::terminal::{Authority, Request as Asked};
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
    fn until(mut done: impl FnMut(&World) -> bool) {
        let deadline = Instant::now() + WAIT;
        let mut world = world();
        while !done(&world) {
            let left = deadline
                .checked_duration_since(Instant::now())
                .expect("the fake library never reached the expected state");
            world = CHANGED
                .wait_timeout(world, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
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
    }

    struct Fake {
        id: [u8; 16],
        script: Script,
        /// The notification pipe: the read end is lent, the write end signals.
        pipe: [i32; 2],
        state: Mutex<State>,
        terminal: Terminal,
    }

    impl Fake {
        fn state(&self) -> std::sync::MutexGuard<'_, State> {
            self.state.lock().unwrap_or_else(PoisonError::into_inner)
        }

        fn push(&self, item: Item) {
            self.state().queue.push_back(item);
            // SAFETY: one byte from a live buffer to the fake's own pipe.
            unsafe { libc::write(self.pipe[1], [1u8].as_ptr().cast(), 1) };
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
        let mut pipe = [0; 2];
        // SAFETY: a two-descriptor output array.
        assert_eq!(unsafe { libc::pipe(pipe.as_mut_ptr()) }, 0);
        for descriptor in pipe {
            // SAFETY: the fake's own new descriptors.
            unsafe { libc::fcntl(descriptor, libc::F_SETFL, libc::O_NONBLOCK) };
        }
        let fake = Box::into_raw(Box::new(Fake {
            id,
            script,
            pipe,
            state: Mutex::default(),
            terminal: Terminal {
                generation: GENERATION,
                client_outcome: native::CLIENT_STOPPED,
                retry: script.retry,
                leave: native::LEAVE_CONFIRMED,
                attachment_detached: 0,
                connection_closed: 1,
                reserved: [0; 2],
                error: Error {
                    message: bytes_of(script.ends_unopened.unwrap_or_default()),
                    ..no_error()
                },
            },
        }));
        OWNER.with(|owner| *owner.borrow_mut() = Some(Exit(id)));
        changed(&mut world(), |world| world.saw(id, Seen::Created));
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

    extern "C" fn run(instance: *mut Instance, _out: *mut RunReport) -> Call {
        let fake = fake(instance);
        if fake.script.run_fails {
            fake.state().over = true;
            return failure(native::INTERNAL_ERROR, b"internal_error", b"run failed");
        }
        if fake.script.ends_unopened.is_none() {
            serve(fake);
        }
        fake.push(Item::Terminal);
        fake.state().over = true;
        success()
    }

    /// Run's opened life: Open's success, as late as the script says, then
    /// shutdown, as late as the script says.
    fn serve(fake: &Fake) {
        let let_go = |world: &World| world.unstalled.contains(&fake.id);
        let shutdown = || fake.state().shutdown;
        wait_while(|world| fake.script.opens_late && !let_go(world) && !shutdown());
        fake.push(Item::Delivery(opened()));
        if fake.script.presents {
            fake.push(Item::Delivery(presentation()));
        }
        if fake.script.drops {
            return;
        }
        wait_while(|world| !(shutdown() && (!fake.script.stalls || let_go(world))));
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
        if request.tag == native::OPEN {
            assert_eq!(request.request_id, OPEN_ID);
            return success();
        }
        let fake = fake(communication);
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
        let mut revoked = record(native::REVOKED);
        revoked.data.revoked = native::Revoked {
            period: 2,
            restoration_id: 4,
            reason: native::SESSION_ENDING,
            reserved: 0,
        };
        vec![
            metadata,
            granted,
            begun,
            display,
            complete,
            reshaped(70_000),
            reshaped(0),
            revoked,
            ended,
            record(native::STATUS),
        ]
    }

    extern "C" fn drain(communication: *mut Communication, out: *mut Drained) -> Call {
        let fake = fake(communication);
        if fake.script.drain_fails {
            return failure(native::INTERNAL_ERROR, b"internal_error", b"drain failed");
        }
        let mut state = fake.state();
        // SAFETY: the supervisor passes an output struct.
        let out = unsafe { &mut *out };
        match state.queue.pop_front() {
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
                out.kind = native::TERMINAL;
                out.terminal = &fake.terminal;
            }
            None => {
                let mut byte = [0u8; 64];
                // SAFETY: reads into a live buffer from the fake's own pipe.
                while unsafe { libc::read(fake.pipe[0], byte.as_mut_ptr().cast(), byte.len()) } > 0
                {
                }
                out.kind = native::EMPTY;
            }
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
        let _world = world();
        fake.state().shutdown = true;
        CHANGED.notify_all();
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
        let mut state = fake(control).state();
        if !state.over {
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
            (fake.pipe[0], success())
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
                shutdown_ms: 0,
            },
            shutdown: Duration::from_millis(shutdown_ms),
            retry,
        };
        let dispatch = log.dispatch(Some("info")).unwrap();
        let supervisor = Supervisor::start(context, dispatch).unwrap();
        Supervised {
            directory,
            supervisor,
            log: log.path().to_owned(),
        }
    }

    fn fake_library() -> Load {
        Box::new(|_| Ok(&FAKE))
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
        supervisor.stop();
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

    /// Client supervisor (Fault, #106; shutdown bound): an instance that ignores
    /// shutdown never holds exit past `deadlines.shutdown_ms`, and it is never
    /// destroyed while its handles are unreleased: its owner leaves it to process
    /// exit (#118 owns the fail-stop), and the monitor log says so.
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
        supervisor.stop();
        assert!(started.elapsed() < Duration::from_secs(10));
        let log = supervised.log();
        assert!(log.contains(&format!("{UNCONFIRMED} clients=1")), "{log}");
        let id = guid(&stalled);
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        assert_eq!(seen(&id), [Seen::Created, Seen::OwnerExited]);
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
            presented(Presentation::Revoked),
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
    /// longer; answered input holds it not at all.
    #[test]
    fn stopping_waits_for_accepted_input_at_most_the_operation_deadline() {
        let held = Authority {
            generation: 1,
            period: 9,
            restoration: 9,
        };
        for (index, silent, operation_ms) in [(1, true, 400), (2, false, 30_000)] {
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
            } = limited(1, 1_000, fake_library(), limits);
            supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
            assert_eq!(
                next(&supervisor),
                (id.clone(), ClientEvent::Opened { status: false })
            );
            supervisor.request(id.clone(), Asked::Input(held, b"z".to_vec()));
            let started = Instant::now();
            supervisor.stop();
            let took = started.elapsed();
            if silent {
                assert!(took >= Duration::from_millis(400), "{took:?}");
            }
            assert!(took < Duration::from_secs(15), "{took:?}");
        }
    }

    const UNCONFIRMED: &str = "client shutdown unconfirmed at its deadline";

    /// A loader that refuses release 0.0.1 and loads the fake library for
    /// any other.
    fn refusing_old() -> Load {
        Box::new(|version| match version.as_str() {
            "0.0.1" => Err(Refusal::Library(Failure::WrongRelease, "old".into())),
            _ => Ok(&FAKE),
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
        for (index, least) in [100, 200, 300, 100].into_iter().enumerate() {
            let waited = created[index + 1] - exited[index];
            assert!(
                waited >= Duration::from_millis(least),
                "{index}: {waited:?}"
            );
            assert!(waited < Duration::from_secs(10), "{index}: {waited:?}");
        }
        supervisor.stop();
    }

    /// REC-01 (Monitor, Fault: permanent rejection and invalid
    /// configuration; a known final outcome), LIB-05 (cleanup that cannot
    /// be confirmed): a client the library rejects permanently, a create
    /// refusing its limits, a client whose final outcome is known before it
    /// is lost and one whose destroy fails are never retried; a create
    /// refused for want of resources is retried and opens.
    #[test]
    fn rejections_known_outcomes_and_unconfirmed_cleanup_retry_nothing() {
        let [rejected, limited_, finished, unclean, scarce] =
            [1, 2, 3, 4, 5].map(|index| session(21, index));
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
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(5, 20_000, fake_library(), (limits, quick));
        let supervisor = &mut supervised.supervisor;
        let all = [&rejected, &limited_, &finished, &unclean, &scarce];
        for (index, id) in all.iter().enumerate() {
            supervisor.start_client(metadata(id, &format!("2026-11-13T10:00:0{index}.000Z")));
        }
        let unconfirmed_why = "cannot confirm the client's cleanup: busy: still in use";
        let reports = reports_until(supervisor, |reports| {
            let has = |id: &SessionId, event: &ClientEvent| {
                reports.get(id).is_some_and(|events| events.contains(event))
            };
            let rejection = ClientEvent::Recovery(Recovery::Rejected(unconfirmed_why.into()));
            has(&scarce, &ClientEvent::Opened { status: false })
                && has(&finished, &ClientEvent::Ended)
                && has(&unclean, &rejection)
                && [&rejected, &limited_]
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
        for id in [&finished, &unclean] {
            let id = guid(id);
            until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        }
        supervisor.stop();
        for id in [&rejected, &limited_, &finished, &unclean] {
            assert_eq!(generations(id), [1], "{id}");
        }
        assert_eq!(generations(&scarce), [1, 2]);
    }

    /// REC-01 (Monitor, Fault: missing library, then a repaired
    /// installation): a release refused for a reason a later load may not
    /// have is loaded again once its session's retry is due, and its
    /// session then opens.
    #[test]
    fn a_missing_library_is_loaded_again_when_its_retry_is_due() {
        let id = session(22, 1);
        let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&loads);
        let repaired: Load =
            Box::new(
                move |_| match counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst) {
                    0 => Err(Refusal::Library(Failure::MissingLibrary, "missing".into())),
                    _ => Ok(&FAKE),
                },
            );
        let quick = Backoff {
            initial_ms: 0,
            cap_ms: 0,
            jitter_percent: 0,
        };
        let limits = climon2_core::config::Limits::default().library();
        let mut supervised = retrying(1, 20_000, repaired, (limits, quick));
        let supervisor = &mut supervised.supervisor;
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let opened = ClientEvent::Opened { status: false };
        let reports = reports_until(supervisor, |reports| {
            reports
                .get(&id)
                .is_some_and(|events| events.contains(&opened))
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
        supervisor.stop();
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 2);
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
    }
}
