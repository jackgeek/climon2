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

    /// Shuts every client down, waiting at most `deadlines.shutdown_ms` (and
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
        self.shut_down();
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

    /// Submits `request` through an open client and remembers it until its
    /// answer; anything else is answered at once with why not.
    fn submit(&mut self, id: &SessionId, request: Request) {
        if let Err(why) = self.submit_one(id, request) {
            self.tell(
                id,
                ClientEvent::Presentation(Presentation::Answered(request, Err(why))),
            );
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
        live.client.submit(request_id, request)?;
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
mod tests {
    //! The supervisor against a fake revision-1 client library (#106, Fault
    //! cases): Rust functions behind the same function table the validated
    //! loader hands the monitor. Each fake instance follows its session's
    //! [`Script`], keeps the native contract's ownership rules (handles release
    //! only once its stream is over and no delivery is outstanding; destroy is
    //! busy while either remains), and records what an independent observer
    //! needs: each destroy's result and when its owner thread exits.
    use super::{client::OPEN_ID, owner::GENERATION, *};
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

    /// How one fake instance behaves.
    #[derive(Clone, Copy, Default)]
    struct Script {
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
    }

    /// What the observer saw of one session's instance.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Seen {
        Destroyed(u32),
        OwnerExited,
    }

    /// A request the fake accepted: tag, request ID and its arguments' words
    /// (generation; or generation, period, restoration; plus columns and rows).
    type Submitted = (u32, u64, Vec<u64>);

    #[derive(Default)]
    struct World {
        scripts: BTreeMap<[u8; 16], Script>,
        seen: Vec<([u8; 16], Seen)>,
        unstalled: BTreeSet<[u8; 16]>,
        submitted: Vec<([u8; 16], Submitted)>,
    }

    static WORLD: Mutex<World> = Mutex::new(World {
        scripts: BTreeMap::new(),
        seen: Vec::new(),
        unstalled: BTreeSet::new(),
        submitted: Vec::new(),
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
                world.seen.push((self.0, Seen::OwnerExited));
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
        let id = unsafe { (*startup).session_id.bytes };
        let script = world().scripts.get(&id).copied().unwrap_or_default();
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
                retry: native::RETRY_NONE,
                leave: native::LEAVE_CONFIRMED,
                attachment_detached: 0,
                connection_closed: 1,
                reserved: [0; 2],
                error: no_error(),
            },
        }));
        OWNER.with(|owner| *owner.borrow_mut() = Some(Exit(id)));
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
        fake.push(Item::Delivery(opened()));
        if fake.script.presents {
            fake.push(Item::Delivery(presentation()));
        }
        let mut world = world();
        while !(fake.state().shutdown
            && (!fake.script.stalls || world.unstalled.contains(&fake.id)))
        {
            world = CHANGED.wait(world).unwrap_or_else(PoisonError::into_inner);
        }
        drop(world);
        fake.push(Item::Terminal);
        fake.state().over = true;
        success()
    }

    extern "C" fn destroy(instance: *mut Instance) -> Call {
        let fake = fake(instance);
        let state = fake.state();
        let free = state.communication && state.control && state.outstanding == 0;
        drop(state);
        let call = if free { success() } else { busy() };
        changed(&mut world(), |world| {
            world.seen.push((fake.id, Seen::Destroyed(call.code)));
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
    /// final and a record the supervisor does not read.
    fn presentation() -> Vec<Record> {
        let geometry = |cols, rows| native::Geometry { cols, rows };
        let mut metadata = record(native::INITIAL_METADATA);
        // SAFETY: an all-zero metadata record is a valid plain value.
        let mut initial: native::Metadata = unsafe { std::mem::zeroed() };
        initial.title.state = native::VALUE;
        initial.title.value = bytes_of(b"T");
        initial.command_result.present = 1;
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
            record(native::FINAL),
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
        world().scripts.insert(guid(id), script);
    }

    fn metadata(id: &SessionId, created: &str) -> SessionMetadata {
        serde_json::from_value(serde_json::json!({
            "format_version": 1, "session_id": id.as_str(), "socket_path": "/tmp/s",
            "daemon_pid": 1, "daemon_start_time": 0, "command": "fake", "args": [],
            "cwd": "/", "depth": 1, "created_at": created, "climon2_version": "0.1.0"
        }))
        .unwrap()
    }

    /// A supervisor over the fake library in its own climon2 home.
    fn supervisor(starts: u32, shutdown_ms: u64, load: Load) -> (tempfile::TempDir, Supervisor) {
        let limits = climon2_core::config::Limits::default().library();
        limited(starts, shutdown_ms, load, limits)
    }

    /// [`supervisor`] with the instance limits `limits`.
    fn limited(
        starts: u32,
        shutdown_ms: u64,
        load: Load,
        limits: native::Limits,
    ) -> (tempfile::TempDir, Supervisor) {
        let directory = tempfile::tempdir().unwrap();
        let home = HomeLayout::resolve(Some(directory.path().as_os_str()), Path::new("/")).unwrap();
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
        };
        let supervisor = Supervisor::start(context, tracing::Dispatch::none()).unwrap();
        (directory, supervisor)
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
    /// released, and the healthy one ends at exit.
    fn a_failure_ends_cleanly_and_frees_its_start_slot(test: u8, failing: Script) -> String {
        let (doomed, healthy) = (session(test, 1), session(test, 2));
        script(&doomed, failing);
        let (_home, mut supervisor) = supervisor(1, 20_000, fake_library());
        supervisor.start_client(metadata(&doomed, "2026-11-13T10:00:02.000Z"));
        supervisor.start_client(metadata(&healthy, "2026-11-13T10:00:01.000Z"));
        let (id, event) = next(&supervisor);
        assert_eq!(id, doomed);
        let why = unavailable(&event).to_owned();
        assert_eq!(
            next(&supervisor),
            (healthy.clone(), ClientEvent::Opened { status: false })
        );
        let destroyed = |id: &SessionId| {
            let id = guid(id);
            move |world: &World| world.seen.contains(&(id, Seen::OwnerExited))
        };
        until(destroyed(&doomed));
        assert_eq!(
            seen(&guid(&doomed)),
            [Seen::Destroyed(native::SUCCESS), Seen::OwnerExited]
        );
        supervisor.stop();
        assert_eq!(
            seen(&guid(&healthy)),
            [Seen::Destroyed(native::SUCCESS), Seen::OwnerExited]
        );
        let rest: Vec<_> = supervisor.reports.try_iter().collect();
        assert_eq!(rest, [(healthy, ClientEvent::Ended)]);
        why
    }

    /// Client supervisor (Fault, #106): a failed notification lookup ends that
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

    /// Client supervisor (Fault, #106): a run that fails without terminal
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
    /// instance progresses): a failed drain ends that instance cleanly: its
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
    /// exit (#118 owns the fail-stop).
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
        let (_home, mut supervisor) = supervisor(1, 200, fake_library());
        supervisor.start_client(metadata(&stalled, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(&supervisor),
            (stalled.clone(), ClientEvent::Opened { status: false })
        );
        let started = Instant::now();
        supervisor.stop();
        assert!(started.elapsed() < Duration::from_secs(10));
        let id = guid(&stalled);
        changed(&mut world(), |world| {
            world.unstalled.insert(id);
        });
        until(|world| world.seen.contains(&(id, Seen::OwnerExited)));
        assert_eq!(seen(&id), [Seen::OwnerExited]);
    }

    /// Client supervisor (Fault, #106; one shared load attempt per release): a
    /// refused release is loaded once however many sessions use it, and each of
    /// them is Unavailable with the loader's diagnosis.
    #[test]
    fn a_refused_release_is_loaded_once_and_its_sessions_are_unavailable() {
        let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = std::sync::Arc::clone(&loads);
        let refuse: Load = Box::new(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(Refusal::Library(Failure::MissingLibrary, "missing".into()))
        });
        let (_home, mut supervisor) = supervisor(4, 20_000, refuse);
        let sessions = [session(4, 1), session(4, 2), session(4, 3)];
        for (second, id) in sessions.iter().enumerate() {
            supervisor.start_client(metadata(id, &format!("2026-11-13T10:00:0{second}.000Z")));
        }
        let mut reported: Vec<_> = (0..3).map(|_| next(&supervisor)).collect();
        reported.sort_by(|a, b| a.0.cmp(&b.0));
        let expected: Vec<_> = sessions
            .iter()
            .map(|id| (id.clone(), ClientEvent::Unavailable("missing".into())))
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
        let (_home, mut supervisor) = supervisor(1, 20_000, fake_library());
        supervisor.start_client(metadata(&id, "2026-11-13T10:00:00.000Z"));
        let authority = |generation, restoration| Authority {
            generation,
            period: 2,
            restoration,
        };
        let expected = [
            ClientEvent::Opened { status: false },
            ClientEvent::Title(Some("T".into())),
            ClientEvent::Final,
            presented(Presentation::Granted(authority(7, 3), geometry(80, 24))),
            presented(Presentation::Begin(
                authority(GENERATION, 4),
                geometry(100, 30),
            )),
            presented(Presentation::Display(DISPLAYED.to_vec())),
            presented(Presentation::Complete),
            presented(Presentation::Geometry(geometry(500, 5))),
            presented(Presentation::Revoked),
            ClientEvent::Final,
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
        let (_home, mut supervisor) = supervisor(1, 20_000, fake_library());
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
            supervisor.request(id.clone(), request);
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

    /// What the fake accepted from `id`'s instance, in order.
    fn submitted(id: &SessionId) -> Vec<Submitted> {
        let id = guid(id);
        let world = world();
        let accepted = world.submitted.iter().filter(|(seen, _)| *seen == id);
        accepted.map(|(_, submitted)| submitted.clone()).collect()
    }

    /// An open client for `id` under `script`, whose pending ledger holds at
    /// most two requests: one queued record and one operation.
    fn opened_with(id: &SessionId, script_of: Script) -> (tempfile::TempDir, Supervisor) {
        script(id, script_of);
        let mut limits = climon2_core::config::Limits::default().library();
        limits.request_records = 1;
        limits.operations = 1;
        let (home, supervisor) = limited(1, 20_000, fake_library(), limits);
        supervisor.start_client(metadata(id, "2026-11-13T10:00:00.000Z"));
        assert_eq!(
            next(&supervisor),
            (id.clone(), ClientEvent::Opened { status: false })
        );
        (home, supervisor)
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
        let (_home, mut supervisor) = opened_with(&id, script);
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
        let (_home, mut supervisor) = opened_with(&id, script);
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
}
