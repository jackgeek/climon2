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
//! freeing its start slot and letting its owner destroy it. Retry and
//! resource admission arrive with later tickets: a refused release or a
//! failed client stays Unavailable.
mod client;
mod owner;

use crate::{
    registry::ClientEvent,
    terminal::{Presentation, Request},
};
use client::{Client, Drain, Stop};
use climon2_core::{
    SessionId, Timestamp, VersionName, config::Lanes, keys::ControlKey, layout::HomeLayout,
    metadata::SessionMetadata, native,
};
use climon2_platform::library::{Failure, Functions};
use owner::StartupData;
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// How long one wait on the notification descriptors lasts before commands
/// are read again.
const TICK_MILLISECONDS: i32 = 20;

/// Loads a release's validated client library: the one shared load of
/// `climon2_platform::library::Loader` in the monitor.
pub(crate) type Load =
    Box<dyn Fn(&VersionName) -> Result<&'static Functions, Refusal> + Send + Sync>;

/// Why a release cannot be used. The loader's [`Failure`] says whether a
/// later load could succeed, for the retry that arrives with #116.
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
}

enum Command {
    Start(Box<SessionMetadata>),
    Request(SessionId, Request),
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
    /// Run returned, or create failed, with why if it failed.
    Stopped(SessionId, Result<(), String>),
    OwnerDone(SessionId),
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
    phase: Phase,
    /// Accepted requests awaiting their answers, by request ID: at most
    /// [`State::ledger`].
    pending: BTreeMap<u64, Request>,
    /// The next request ID; Open is 1.
    next: u64,
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
    /// The most requests one instance's pending ledger holds: every request
    /// its library may have accepted, queued or under way.
    ledger: usize,
}

/// What scheduling does for one waiting session.
enum Step {
    Load(VersionName),
    Refuse(String),
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
            Command::Shutdown => return false,
        }
        true
    }

    fn wait(&mut self, metadata: SessionMetadata) {
        let key = (
            Reverse(metadata.created_at.clone()),
            metadata.session_id.clone(),
        );
        self.waiting.insert(key, metadata);
    }

    /// Submits `request` through an open client, input in chunks of at most
    /// `chunk_bytes`, in order. The first refusal is answered at once with
    /// why, carrying every byte not submitted, and nothing after it is
    /// submitted.
    fn submit(&mut self, id: &SessionId, request: Request) {
        let chunk = self.context.limits.chunk_bytes as usize;
        let mut unsent: Option<(Request, String)> = None;
        for request in split(request, chunk) {
            match unsent.as_mut() {
                Some((refused, _)) => absorb(refused, request),
                None => unsent = self.submit_one(id, request).err(),
            }
        }
        if let Some((request, why)) = unsent {
            self.tell(
                id,
                ClientEvent::Presentation(Presentation::Answered(request, Err(why))),
            );
        }
    }

    /// Submits one request and enters it in the pending ledger once the
    /// library accepts it. A request refused, here or by the library,
    /// consumes no request ID.
    fn submit_one(&mut self, id: &SessionId, request: Request) -> Result<(), (Request, String)> {
        let capacity = self.ledger;
        let open = self
            .instances
            .get_mut(id)
            .and_then(|instance| match instance.phase {
                Phase::Open(live) => Some((live, instance)),
                _ => None,
            });
        let Some((live, instance)) = open else {
            return Err((request, "the session's client is not connected".to_owned()));
        };
        if instance.pending.len() >= capacity {
            let why = "too many requests are awaiting their answers".to_owned();
            return Err((request, why));
        }
        let request_id = instance.next;
        if let Err(why) = live.client.submit(request_id, &request) {
            return Err((request, why));
        }
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
            Internal::OwnerDone(id) => self.owner_done(&id),
        }
    }

    /// Starts loads and clients the lanes have room for, newest created
    /// first. A session waiting on its release holds no start slot.
    fn schedule(&mut self) {
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
            None => (self.loading() < lanes.loads).then(|| Step::Load(version.clone())),
            Some(Release::Loading) => None,
            Some(Release::Refused(refusal)) => Some(Step::Refuse(refusal.message().to_owned())),
            Some(Release::Loaded(functions)) => {
                (self.starting() < lanes.starts).then_some(Step::Start(functions))
            }
        }
    }

    fn take(&mut self, key: (Reverse<Timestamp>, SessionId), step: Step) {
        match step {
            Step::Load(version) => self.load(version),
            Step::Refuse(why) => {
                self.waiting.remove(&key);
                self.tell(&key.1, ClientEvent::Unavailable(why));
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

    fn spawn_owner(&mut self, functions: &'static Functions, metadata: SessionMetadata) {
        let id = metadata.session_id.clone();
        tracing::info!(session = %id, version = %metadata.climon2_version, "client starting");
        let (released, release) = mpsc::channel();
        let internal = self.internal.clone();
        let owned = id.clone();
        let spawned = StartupData::new(&self.context, &metadata).and_then(|startup| {
            thread::Builder::new()
                .name("client".into())
                .spawn(move || owner::owner(functions, &startup, owned, &internal, &release))
        });
        match spawned {
            Ok(owner) => {
                let phase = Phase::Creating;
                let instance = Instance {
                    phase,
                    pending: BTreeMap::new(),
                    next: client::OPEN_ID + 1,
                    released,
                    owner,
                };
                self.instances.insert(id, instance);
            }
            Err(error) => self.tell(&id, ClientEvent::Unavailable(error.to_string())),
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
                revents: 0,
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
            Stop::Terminal(message) => self.terminal(id, message),
            Stop::Failed(error) => self.fail(id, format!("cannot drain the client: {error}")),
        }
    }

    /// A report from `id`'s deliveries: Open's refusal fails the instance;
    /// anything else reaches the row while it follows this instance.
    fn event(&mut self, id: &SessionId, event: ClientEvent) {
        match event {
            ClientEvent::Unavailable(why) => self.fail(id, why),
            ClientEvent::Answer(request_id, answer) => self.answer(id, request_id, answer),
            event => self.report(id, event),
        }
    }

    fn report(&mut self, id: &SessionId, event: ClientEvent) {
        let Some(phase) = self.phase(id) else {
            return;
        };
        self.set(id, phase.after(&event));
        if phase.reporting() {
            self.tell(id, event);
        }
    }

    /// Terminal evidence: report what it means, then release the handles.
    fn terminal(&mut self, id: &SessionId, message: String) {
        tracing::info!(session = %id, %message, "client ended");
        let message = Some(message).filter(|message| !message.is_empty());
        let why = message.unwrap_or_else(|| "the client stopped before it opened".into());
        self.finish(id, why);
    }

    /// A failed instance: report it Unavailable once, ask it to stop and keep
    /// draining it until its stream is over.
    fn fail(&mut self, id: &SessionId, why: String) {
        let Some(phase) = self.phase(id) else {
            return;
        };
        let Some(live) = phase.live().filter(|_| phase.reporting()) else {
            return;
        };
        tracing::warn!(session = %id, %why, "client failed");
        live.client.shutdown();
        self.set(id, Phase::Closing(live));
        self.tell(id, ClientEvent::Unavailable(why));
    }

    /// Run returned, or create failed: whatever is left to drain is
    /// drained, and an instance still not over ends here.
    fn stopped(&mut self, id: &SessionId, ran: Result<(), String>) {
        self.drain(id);
        let why = ran
            .err()
            .unwrap_or_else(|| "the client stopped without terminal evidence".into());
        if !matches!(self.phase(id), Some(Phase::Done) | None) {
            tracing::warn!(session = %id, %why, "client stopped");
            self.finish(id, why);
        }
    }

    /// Reports the end, releases the handles and lets the owner destroy.
    fn finish(&mut self, id: &SessionId, why: String) {
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
        if let Some(event) = phase.over(why) {
            self.tell(id, event);
        }
    }

    fn owner_done(&mut self, id: &SessionId) {
        if let Some(instance) = self.instances.remove(id) {
            let _ = instance.owner.join();
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
        while !self.instances.is_empty() && Instant::now() < deadline {
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
mod tests;

/// The requests `request` is submitted as: input in chunks of at most
/// `chunk` bytes (the library refuses larger ones), anything else whole.
fn split(request: Request, chunk: usize) -> Vec<Request> {
    match request {
        Request::Input(authority, bytes) => bytes
            .chunks(chunk)
            .map(|bytes| Request::Input(authority, bytes.to_vec()))
            .collect(),
        request => vec![request],
    }
}

/// Adds a later chunk's bytes to the refused input they follow.
fn absorb(refused: &mut Request, request: Request) {
    if let (Request::Input(_, unsent), Request::Input(_, more)) = (refused, request) {
        unsent.extend(more);
    }
}
