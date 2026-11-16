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

use crate::registry::ClientEvent;
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
        Self {
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
                Ok(Command::Start(metadata)) => self.wait(*metadata),
                Ok(Command::Shutdown) | Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    fn wait(&mut self, metadata: SessionMetadata) {
        let key = (
            Reverse(metadata.created_at.clone()),
            metadata.session_id.clone(),
        );
        self.waiting.insert(key, metadata);
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
            None => room(self.loading(), lanes.loads).then(|| Step::Load(version.clone())),
            Some(Release::Loading) => None,
            Some(Release::Refused(refusal)) => Some(Step::Refuse(refusal.message().to_owned())),
            Some(Release::Loaded(functions)) => {
                room(self.starting(), lanes.starts).then_some(Step::Start(functions))
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
            Stop::Terminal(message) => self.terminal(id, message),
            Stop::Failed(error) => self.fail(id, format!("cannot drain the client: {error}")),
        }
    }

    /// A report from `id`'s deliveries: Open's refusal fails the instance;
    /// anything else reaches the row while it follows this instance.
    fn event(&mut self, id: &SessionId, event: ClientEvent) {
        match event {
            ClientEvent::Unavailable(why) => self.fail(id, why),
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
    use super::{client::OPEN_ID, owner::GENERATION, *};
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
        /// Run sends Open's success only once the test lets it go (or
        /// shutdown asks), so the instance holds its start slot until then.
        opens_late: bool,
        /// Run ends at once, before Open's response, with terminal evidence
        /// carrying this message.
        ends_unopened: Option<&'static [u8]>,
    }

    /// What the observer saw of one session's instance.
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum Seen {
        Created,
        Destroyed(u32),
        OwnerExited,
    }

    #[derive(Default)]
    struct World {
        scripts: BTreeMap<[u8; 16], Script>,
        seen: Vec<([u8; 16], Seen)>,
        unstalled: BTreeSet<[u8; 16]>,
    }

    static WORLD: Mutex<World> = Mutex::new(World {
        scripts: BTreeMap::new(),
        seen: Vec::new(),
        unstalled: BTreeSet::new(),
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
                error: Error {
                    message: bytes_of(script.ends_unopened.unwrap_or_default()),
                    ..no_error()
                },
            },
        }));
        OWNER.with(|owner| *owner.borrow_mut() = Some(Exit(id)));
        changed(&mut world(), |world| world.seen.push((id, Seen::Created)));
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
        let free = state.communication && state.control && state.outstanding == 0;
        drop(state);
        let call = if free { success() } else { busy() };
        changed(&mut world(), |world| {
            world.seen.push((fake.id, Seen::Destroyed(call.code)));
        });
        call
    }

    extern "C" fn submit(_communication: *mut Communication, request: *const Request) -> Call {
        // SAFETY: the supervisor passes a complete request.
        let request = unsafe { &*request };
        assert_eq!((request.tag, request.request_id), (native::OPEN, OPEN_ID));
        success()
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

    /// A supervisor over `load` in its own climon2 home, logging to that
    /// home's monitor log.
    fn supervisor(starts: u32, shutdown_ms: u64, load: Load) -> Supervised {
        let directory = tempfile::tempdir().unwrap();
        let home = HomeLayout::resolve(Some(directory.path().as_os_str()), Path::new("/")).unwrap();
        let log = climon2_platform::logging::LogFile::monitor(directory.path()).unwrap();
        let context = Context {
            load,
            home,
            runtime: directory.path().join("run"),
            escape: ControlKey::from_byte(0x1d).unwrap(),
            take_control: None,
            limits: climon2_core::config::Limits::default().library(),
            filter: "info".into(),
            lanes: Lanes {
                loads: 1,
                starts,
                shutdown_ms: 0,
            },
            shutdown: Duration::from_millis(shutdown_ms),
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
            5,
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
        let Supervised {
            mut supervisor,
            directory: _home,
            ..
        } = supervisor(4, 20_000, refuse);
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

    const UNCONFIRMED: &str = "client shutdown unconfirmed at its deadline";

    /// A loader that refuses release 0.0.1 and loads the fake library for
    /// any other.
    fn refusing_old() -> Load {
        Box::new(|version| match version.as_str() {
            "0.0.1" => Err(Refusal::Library(Failure::MissingLibrary, "old".into())),
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
        let [first, second, third] = [session(6, 1), session(6, 2), session(6, 3)];
        let [refused, marker] = [session(6, 4), session(6, 5)];
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
        assert_eq!(next(supervisor), (refused, why.clone()));
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
        assert_eq!(next(supervisor), (marker, why));
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
        let [named, silent] = [session(7, 1), session(7, 2)];
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
}
