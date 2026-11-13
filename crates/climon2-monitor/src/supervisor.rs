//! The basic client supervisor (spec #73, "Client supervisor"; #106): one
//! passive monitoring client per live session, matched by full session ID.
//!
//! Its own thread loads each release on a worker thread through the shared
//! validated loader, one attempt per release and at most `scheduling.loads`
//! at once, and starts clients newest created first, then ascending session
//! ID, at most `scheduling.starts` at once. Each instance has a dedicated
//! owner thread that creates it, submits monitor Open, publishes its handles
//! and runs it. The supervisor polls the borrowed notification descriptors,
//! drains every delivery, reports what the registry needs and releases each
//! delivery at once. Retry and resource admission arrive with later tickets:
//! a refused release or a failed client stays Unavailable.
use crate::registry::ClientEvent;
use climon2_core::{
    SessionId, Timestamp, VersionName,
    config::Lanes,
    keys::ControlKey,
    layout::HomeLayout,
    metadata::SessionMetadata,
    native::{self, Bytes, Call, Communication, Control, Created, Drained, Request, RequestArgs},
    wire::Guid,
};
use climon2_platform::library::{Library, Loader};
use std::{
    cmp::Reverse,
    collections::BTreeMap,
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// Monitor Open's families: required, then requested as optional.
const REQUIRED: [&str; 2] = ["daemon-wire-v1", "monitor-v1"];
const OPTIONAL: [&str; 5] = [
    "title-v1",
    "status-v1",
    "notification-v1",
    "progress-v1",
    "focus-v1",
];
const STATUS: &str = "status-v1";
const OPEN_ID: u64 = 1;
const GENERATION: u64 = 1;
/// How long one wait on the notification descriptors lasts before commands
/// are read again.
const TICK_MILLISECONDS: i32 = 20;

/// What every instance's startup shares.
pub(crate) struct Context {
    pub(crate) loader: Loader,
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

    /// Shuts every client down, waiting at most `deadlines.shutdown_ms` for
    /// their terminal evidence, released handles and joined owners.
    pub(crate) fn stop(mut self) {
        let _ = self.commands.send(Command::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// An instance's caller handles, published by its owner thread.
#[derive(Clone, Copy)]
struct Handles {
    communication: *mut Communication,
    control: *mut Control,
}

// SAFETY: both handles are documented thread-safe for the calls made on them.
unsafe impl Send for Handles {}

enum Internal {
    Loaded(VersionName, Result<&'static Library, String>),
    Published(SessionId, Handles),
    CreateFailed(SessionId, String),
    OwnerDone(SessionId),
}

enum Release {
    Loading,
    Loaded(&'static Library),
    Refused(String),
}

struct Instance {
    library: &'static Library,
    handles: Option<Handles>,
    descriptor: i32,
    /// Holding a start slot: not yet opened, failed or ended.
    starting: bool,
    opened: bool,
    /// Tells the owner its handles are released, so it may destroy.
    released: Option<Sender<()>>,
    owner: Option<JoinHandle<()>>,
}

struct State {
    context: Arc<Context>,
    report: Sender<(SessionId, ClientEvent)>,
    internal: Sender<Internal>,
    internals: Receiver<Internal>,
    waiting: BTreeMap<(Reverse<Timestamp>, SessionId), SessionMetadata>,
    releases: BTreeMap<VersionName, Release>,
    loading: u32,
    starting: u32,
    instances: BTreeMap<SessionId, Instance>,
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
            loading: 0,
            starting: 0,
            instances: BTreeMap::new(),
        }
    }

    /// Rounds of commands, internal events, scheduling, then a bounded poll
    /// and the drains it calls for, until shutdown or the UI thread is gone.
    fn serve(mut self, commands: &Receiver<Command>) {
        loop {
            loop {
                match commands.try_recv() {
                    Ok(Command::Start(metadata)) => self.wait(*metadata),
                    Ok(Command::Shutdown) | Err(mpsc::TryRecvError::Disconnected) => {
                        return self.shut_down();
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
            while let Ok(internal) = self.internals.try_recv() {
                self.internal(internal);
            }
            self.schedule();
            self.poll();
        }
    }

    fn wait(&mut self, metadata: SessionMetadata) {
        let key = (
            Reverse(metadata.created_at.clone()),
            metadata.session_id.clone(),
        );
        self.waiting.insert(key, metadata);
    }

    fn internal(&mut self, internal: Internal) {
        match internal {
            Internal::Loaded(version, Ok(library)) => {
                self.loading -= 1;
                tracing::info!(%version, "client library loaded");
                self.releases.insert(version, Release::Loaded(library));
            }
            Internal::Loaded(version, Err(error)) => {
                self.loading -= 1;
                tracing::warn!(%version, %error, "client library refused");
                self.releases.insert(version, Release::Refused(error));
            }
            Internal::Published(id, handles) => self.published(id, handles),
            Internal::CreateFailed(id, error) => {
                tracing::warn!(session = %id, %error, "client create failed");
                if let Some(mut instance) = self.instances.remove(&id) {
                    self.unslot(&mut instance);
                    join(instance.owner.take());
                }
                self.tell(&id, ClientEvent::Unavailable(error));
            }
            Internal::OwnerDone(id) => {
                if let Some(mut instance) = self.instances.remove(&id) {
                    join(instance.owner.take());
                }
            }
        }
    }

    fn published(&mut self, id: SessionId, handles: Handles) {
        let Some(instance) = self.instances.get_mut(&id) else {
            return;
        };
        let functions = instance.library.functions();
        let mut descriptor = -1;
        // SAFETY: a live communication handle and an output integer.
        let call = unsafe { (functions.notification)(handles.communication, &mut descriptor) };
        if call.code != native::SUCCESS {
            tracing::warn!(session = %id, error = %error_text(&call), "notification lookup failed");
        }
        instance.handles = Some(handles);
        instance.descriptor = descriptor;
        tracing::info!(session = %id, "client published");
    }

    /// Starts loads and clients the lanes have room for, newest created
    /// first. A session waiting on its release holds no start slot.
    fn schedule(&mut self) {
        let lanes = self.context.lanes;
        let keys: Vec<_> = self.waiting.keys().cloned().collect();
        for key in keys {
            let version = self.waiting[&key].climon2_version.clone();
            match self.releases.get(&version) {
                None if self.loading < lanes.loads => self.load(version),
                Some(Release::Refused(error)) => {
                    let error = error.clone();
                    self.waiting.remove(&key);
                    self.tell(&key.1, ClientEvent::Unavailable(error));
                }
                Some(Release::Loaded(library)) if self.starting < lanes.starts => {
                    let library = *library;
                    let metadata = self.waiting.remove(&key).expect("a waiting session");
                    self.spawn_owner(library, metadata);
                }
                _ => {}
            }
        }
    }

    fn load(&mut self, version: VersionName) {
        self.loading += 1;
        self.releases.insert(version.clone(), Release::Loading);
        let context = Arc::clone(&self.context);
        let internal = self.internal.clone();
        let loading = version.clone();
        let spawned = thread::Builder::new().name("load".into()).spawn(move || {
            let loaded = context
                .loader
                .load(&loading)
                .map_err(|error| error.to_string());
            let _ = internal.send(Internal::Loaded(loading, loaded));
        });
        if let Err(error) = spawned {
            let _ = self
                .internal
                .send(Internal::Loaded(version, Err(error.to_string())));
        }
    }

    fn spawn_owner(&mut self, library: &'static Library, metadata: SessionMetadata) {
        let id = metadata.session_id.clone();
        tracing::info!(session = %id, version = %metadata.climon2_version, "client starting");
        let startup = match StartupData::new(&self.context, &metadata) {
            Ok(startup) => startup,
            Err(error) => {
                self.tell(&id, ClientEvent::Unavailable(error.to_string()));
                return;
            }
        };
        let (released, release) = mpsc::channel();
        let internal = self.internal.clone();
        let owned = id.clone();
        let spawned = thread::Builder::new()
            .name("client".into())
            .spawn(move || owner(library, &startup, owned, &internal, &release));
        match spawned {
            Ok(owner) => {
                self.starting += 1;
                self.instances.insert(
                    id,
                    Instance {
                        library,
                        handles: None,
                        descriptor: -1,
                        starting: true,
                        opened: false,
                        released: Some(released),
                        owner: Some(owner),
                    },
                );
            }
            Err(error) => self.tell(&id, ClientEvent::Unavailable(error.to_string())),
        }
    }

    /// Waits one tick on every published descriptor, then drains those that
    /// are ready.
    fn poll(&mut self) {
        let ids: Vec<_> = self
            .instances
            .iter()
            .filter(|(_, instance)| instance.handles.is_some() && instance.descriptor >= 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut table: Vec<_> = ids
            .iter()
            .map(|id| libc::pollfd {
                fd: self.instances[id].descriptor,
                events: libc::POLLIN,
                revents: 0,
            })
            .collect();
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
        for (id, entry) in ids.iter().zip(&table) {
            if entry.revents != 0 {
                self.drain(id);
            }
        }
    }

    /// Drains `id`'s deliveries until none is left, releasing each at once.
    fn drain(&mut self, id: &SessionId) {
        loop {
            let Some(instance) = self.instances.get(id) else {
                return;
            };
            let Some(handles) = instance.handles else {
                return;
            };
            let functions = *instance.library.functions();
            let mut drained = MaybeUninit::<Drained>::zeroed();
            // SAFETY: a live communication handle and an output struct.
            let call = unsafe { (functions.drain)(handles.communication, drained.as_mut_ptr()) };
            if call.code != native::SUCCESS {
                tracing::warn!(session = %id, error = %error_text(&call), "drain failed");
                return;
            }
            // SAFETY: a successful drain initialized the output.
            let drained = unsafe { drained.assume_init() };
            match drained.kind {
                native::DELIVERY => {
                    // SAFETY: a delivery's records live until its release.
                    let records = unsafe { slice(drained.records, drained.count) };
                    for record in records {
                        self.record(id, record);
                    }
                    // SAFETY: released once, through its own library.
                    let call = unsafe { (functions.release_delivery)(drained.delivery) };
                    if call.code != native::SUCCESS {
                        tracing::warn!(session = %id, error = %error_text(&call), "delivery release failed");
                    }
                }
                native::TERMINAL => {
                    // SAFETY: terminal evidence lives until destroy.
                    let terminal = unsafe { &*drained.terminal };
                    return self.ended(id, terminal);
                }
                _ => return,
            }
        }
    }

    fn record(&mut self, id: &SessionId, record: &native::Record) {
        match record.tag {
            native::RESPONSE => {
                // SAFETY: the tag names the active member.
                let response = unsafe { record.data.response };
                if response.request_id == OPEN_ID {
                    self.opened(id, &response);
                }
            }
            // SAFETY: each tag names the active member.
            native::INITIAL_METADATA => self.title(id, unsafe { record.data.metadata.title }),
            native::TITLE => self.title(id, unsafe { record.data.title }),
            native::DIAGNOSTIC => {
                let error = unsafe { record.data.diagnostic };
                tracing::warn!(session = %id, code = %text(error.code), message = %text(error.message), "client diagnostic");
            }
            _ => {}
        }
    }

    fn opened(&mut self, id: &SessionId, response: &native::Response) {
        if response.outcome == native::OK && response.result_tag == native::OPENED {
            // SAFETY: the result tag names the active member.
            let selected = unsafe { response.result.opened.selected };
            // SAFETY: the tokens live until the delivery's release.
            let tokens = unsafe { slice(selected.data, selected.count) };
            let status = tokens.iter().any(|token| text(*token) == STATUS);
            tracing::info!(session = %id, status, "client opened");
            if let Some(instance) = self.instances.get_mut(id) {
                instance.opened = true;
            }
            self.unslot_id(id);
            self.tell(id, ClientEvent::Opened { status });
        } else {
            let message = text(response.error.message);
            tracing::warn!(session = %id, code = %text(response.error.code), %message, "client open refused");
            self.unslot_id(id);
            self.tell(id, ClientEvent::Unavailable(message));
        }
    }

    fn title(&mut self, id: &SessionId, title: native::Title) {
        let value = (title.state == native::VALUE).then(|| text(title.value));
        self.tell(id, ClientEvent::Title(value));
    }

    /// After terminal evidence: report it, release both handles and let the
    /// owner destroy its instance.
    fn ended(&mut self, id: &SessionId, terminal: &native::Terminal) {
        let message = text(terminal.error.message);
        tracing::info!(session = %id, outcome = terminal.client_outcome, retry = terminal.retry, %message, "client ended");
        let Some(mut instance) = self.instances.remove(id) else {
            return;
        };
        self.unslot(&mut instance);
        let event = if instance.opened {
            ClientEvent::Ended
        } else if message.is_empty() {
            ClientEvent::Unavailable("the client stopped before it opened".into())
        } else {
            ClientEvent::Unavailable(message)
        };
        self.tell(id, event);
        if let Some(handles) = instance.handles.take() {
            release(id, instance.library, handles);
        }
        if let Some(released) = instance.released.take() {
            let _ = released.send(());
        }
        instance.descriptor = -1;
        self.instances.insert(id.clone(), instance);
    }

    fn unslot_id(&mut self, id: &SessionId) {
        if let Some(mut instance) = self.instances.remove(id) {
            self.unslot(&mut instance);
            self.instances.insert(id.clone(), instance);
        }
    }

    fn unslot(&mut self, instance: &mut Instance) {
        if instance.starting {
            instance.starting = false;
            self.starting -= 1;
        }
    }

    fn tell(&self, id: &SessionId, event: ClientEvent) {
        let _ = self.report.send((id.clone(), event));
    }

    /// Requests shutdown of every instance, then keeps draining until each
    /// has ended and its owner is joined, or `deadlines.shutdown_ms` passes.
    fn shut_down(mut self) {
        self.waiting.clear();
        let deadline = Instant::now() + self.context.shutdown;
        let mut asked = Vec::new();
        while !self.instances.is_empty() && Instant::now() < deadline {
            for (id, instance) in &self.instances {
                if let Some(handles) = instance.handles
                    && !asked.contains(id)
                {
                    // SAFETY: a live control handle; shutdown is idempotent.
                    unsafe { (instance.library.functions().shutdown)(handles.control) };
                    asked.push(id.clone());
                }
            }
            while let Ok(internal) = self.internals.try_recv() {
                self.internal(internal);
            }
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

fn join(owner: Option<JoinHandle<()>>) {
    if let Some(owner) = owner {
        let _ = owner.join();
    }
}

/// Releases both caller handles once the stream is terminal.
fn release(id: &SessionId, library: &Library, handles: Handles) {
    let functions = library.functions();
    // SAFETY: the stream is terminal and nothing else uses these handles.
    let calls = unsafe {
        [
            (functions.release_communication)(handles.communication),
            (functions.release_control)(handles.control),
        ]
    };
    for call in calls.iter().filter(|call| call.code != native::SUCCESS) {
        tracing::warn!(session = %id, error = %error_text(call), "handle release failed");
    }
}

/// An instance's whole life on its owner thread: create, submit Open,
/// publish the handles, run, then destroy once the supervisor released them.
fn owner(
    library: &'static Library,
    startup: &StartupData,
    id: SessionId,
    internal: &Sender<Internal>,
    released: &Receiver<()>,
) {
    let functions = library.functions();
    let mut created = MaybeUninit::<Created>::zeroed();
    let native = startup.native();
    // SAFETY: a complete startup whose buffers outlive the call.
    let call = unsafe { (functions.create)(&native, created.as_mut_ptr()) };
    if call.code != native::SUCCESS {
        let _ = internal.send(Internal::CreateFailed(id, error_text(&call)));
        return;
    }
    // SAFETY: a successful create initialized the output.
    let created = unsafe { created.assume_init() };
    let handles = Handles {
        communication: created.communication,
        control: created.control,
    };
    let call = submit_open(library, handles.communication);
    if call.code != native::SUCCESS {
        tracing::warn!(session = %id, error = %error_text(&call), "monitor open not accepted");
        // SAFETY: a live control handle; run then ends with terminal evidence.
        unsafe { (functions.shutdown)(handles.control) };
    }
    let _ = internal.send(Internal::Published(id.clone(), handles));
    let mut report = MaybeUninit::<native::RunReport>::zeroed();
    // SAFETY: the instance created on this thread and an output struct.
    let call = unsafe { (functions.run)(created.instance, report.as_mut_ptr()) };
    if call.code != native::SUCCESS {
        tracing::warn!(session = %id, error = %error_text(&call), "client run failed");
    }
    // The supervisor releases the handles after terminal evidence.
    let _ = released.recv();
    // SAFETY: run returned and the handles are released.
    let call = unsafe { (functions.destroy)(created.instance) };
    if call.code != native::SUCCESS {
        tracing::warn!(session = %id, error = %error_text(&call), "client destroy failed");
    }
    let _ = internal.send(Internal::OwnerDone(id));
}

/// Submits monitor Open (request 1): `daemon-wire-v1` and `monitor-v1`
/// required, the title, status, notification, progress and focus families
/// optional. Submit copies everything before it returns.
fn submit_open(library: &Library, communication: *mut Communication) -> Call {
    let tokens = |names: &[&str]| -> Vec<Bytes> {
        names.iter().map(|name| bytes(name.as_bytes())).collect()
    };
    let required = tokens(&REQUIRED);
    let optional = tokens(&OPTIONAL);
    let request = Request {
        tag: native::OPEN,
        reserved: 0,
        request_id: OPEN_ID,
        args: RequestArgs {
            open: native::Open {
                mode: native::MONITOR,
                reserved: 0,
                geometry: native::Geometry { cols: 0, rows: 0 },
                required: native::Tokens {
                    data: required.as_ptr(),
                    count: required.len() as u64,
                },
                optional: native::Tokens {
                    data: optional.as_ptr(),
                    count: optional.len() as u64,
                },
            },
        },
    };
    // SAFETY: a live communication handle and a request whose buffers outlive the call.
    unsafe { (library.functions().submit)(communication, &request) }
}

/// One instance's startup inputs, owned until create has copied them.
struct StartupData {
    session: [u8; 16],
    release: Vec<u8>,
    metadata: Vec<u8>,
    home: Vec<u8>,
    runtime: Vec<u8>,
    log: Vec<u8>,
    filter: Vec<u8>,
    settings: native::Settings,
    limits: native::Limits,
}

// SAFETY: plain owned data; raw pointers exist only inside `native()`.
unsafe impl Send for StartupData {}

impl StartupData {
    fn new(context: &Context, metadata: &SessionMetadata) -> std::io::Result<Self> {
        let id = &metadata.session_id;
        let log = climon2_platform::logging::library_log(context.home.root(), id)?;
        let guid: Guid = id.as_str().parse().map_err(std::io::Error::other)?;
        let path = |path: &Path| path.as_os_str().as_bytes().to_vec();
        let (take_control_enabled, take_control_byte) =
            context.take_control.map_or((0, 0), |key| (1, key.byte()));
        Ok(Self {
            session: guid.bytes(),
            release: metadata.climon2_version.as_str().as_bytes().to_vec(),
            metadata: path(&context.home.metadata(id)),
            home: path(context.home.root()),
            runtime: path(&context.runtime),
            log: path(&log),
            filter: context.filter.as_bytes().to_vec(),
            settings: native::Settings {
                escape_enabled: 1,
                escape_byte: context.escape.byte(),
                take_control_enabled,
                take_control_byte,
                reserved: 0,
            },
            limits: context.limits,
        })
    }

    fn native(&self) -> native::Startup {
        let path = |data: &[u8]| native::Path {
            encoding: native::NATIVE_UNIX,
            reserved: 0,
            bytes: bytes(data),
        };
        native::Startup {
            session_id: native::Guid {
                bytes: self.session,
            },
            generation: GENERATION,
            expected_release: bytes(&self.release),
            metadata_path: path(&self.metadata),
            home_path: path(&self.home),
            runtime_path: path(&self.runtime),
            settings: self.settings,
            diagnostics: native::Diagnostics {
                log_path: native::OptionalPath {
                    present: 1,
                    reserved: 0,
                    value: path(&self.log),
                },
                filter: bytes(&self.filter),
            },
            limits: self.limits,
        }
    }
}

fn bytes(data: &[u8]) -> Bytes {
    if data.is_empty() {
        return Bytes {
            data: ptr::null(),
            length: 0,
        };
    }
    Bytes {
        data: data.as_ptr(),
        length: data.len() as u64,
    }
}

/// `count` items at `data`; none for a null pointer.
///
/// # Safety
/// `data` points at `count` initialized items that outlive the slice.
unsafe fn slice<'a, T>(data: *const T, count: u64) -> &'a [T] {
    if data.is_null() || count == 0 {
        return &[];
    }
    // SAFETY: as the caller promises.
    unsafe { std::slice::from_raw_parts(data, count as usize) }
}

/// Library text, copied lossily: it is diagnostic or display text.
fn text(bytes: Bytes) -> String {
    // SAFETY: counted library bytes valid for this call.
    String::from_utf8_lossy(unsafe { slice(bytes.data, bytes.length) }).into_owned()
}

/// A failed call's code and message, copied before any other call.
fn error_text(call: &Call) -> String {
    format!("{}: {}", text(call.error.code), text(call.error.message))
}
