//! A local instance on its owner thread, over a fake client library: the
//! native calls client embedding makes, in their order, and what it tells
//! its consumer of each (ADR 0011; #489). Expected values are written from
//! the native header.
use climon2_core::{
    SessionId,
    keys::ControlKey,
    layout::HomeLayout,
    metadata::SessionMetadata,
    native::{
        self, Bytes, Call, Communication, Control, Created, Delivery, Drained, Error, Instance,
        Metrics, Record, RunReport,
    },
    protocol::TerminalSize,
};
use climon2_embed::{
    instance::{Drain, Instance as _, KILL_ID, Leave, Local, OPEN_ID, Retry, Stop, Terminal},
    owner::{self, Environment, Owner, Role, Signal, Startup},
    request::{Authority, Request},
};
use climon2_platform::{library::Functions, native_path};
use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Condvar, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

const WAIT: Duration = Duration::from_secs(20);
const GENERATION: u64 = 7;

/// How one fake instance behaves.
#[derive(Clone, Copy, Default)]
struct Script {
    create_fails: Option<u32>,
    submit_fails: bool,
    run_fails: Option<u32>,
    drain_fails: bool,
    notification_fails: bool,
    release_delivery_fails: bool,
    release_communication_fails: bool,
    release_control_fails: bool,
    destroy_fails: bool,
}

/// What create was given, copied before it returned.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct Started {
    generation: u64,
    release: Vec<u8>,
    metadata: (u32, Vec<u8>),
    home: (u32, Vec<u8>),
    runtime: (u32, Vec<u8>),
    /// Escape enabled and its byte, take control enabled and its byte.
    settings: (u8, u8, u8, u8),
    log: (u32, u32, Vec<u8>),
    filter: Vec<u8>,
    client_bytes: u64,
    thread: Option<String>,
}

enum Item {
    Delivery(Vec<Record>),
    Terminal,
}

#[derive(Default)]
struct State {
    /// Every call made, in order.
    calls: Vec<String>,
    queue: VecDeque<Item>,
    /// Each delivery's records, kept until the process exits.
    delivered: Vec<Vec<Record>>,
    shutdown: bool,
}

struct Fake {
    script: Script,
    started: Started,
    state: Mutex<State>,
    changed: Condvar,
    terminal: native::Terminal,
}

// SAFETY: its records point only at static bytes, and its state is locked.
unsafe impl Send for Fake {}
// SAFETY: as above.
unsafe impl Sync for Fake {}

impl Fake {
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn called(&self, call: impl Into<String>) {
        self.state().calls.push(call.into());
    }

    fn calls(&self) -> Vec<String> {
        self.state().calls.clone()
    }
}

#[derive(Default)]
struct World {
    scripts: BTreeMap<[u8; 16], Script>,
    fakes: BTreeMap<[u8; 16], &'static Fake>,
}

static WORLD: Mutex<World> = Mutex::new(World {
    scripts: BTreeMap::new(),
    fakes: BTreeMap::new(),
});

fn world() -> MutexGuard<'static, World> {
    WORLD.lock().unwrap_or_else(PoisonError::into_inner)
}

fn fake<'a, T>(handle: *mut T) -> &'a Fake {
    // SAFETY: every handle the fake hands out is its leaked `Fake`.
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
        error: Error {
            origin: native::ORIGIN_NONE,
            reserved: 0,
            code: NONE,
            message: NONE,
        },
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

fn busy(fails: bool) -> Call {
    if fails {
        failure(native::BUSY, b"busy", b"still in use")
    } else {
        success()
    }
}

/// Copies counted bytes the caller owns for this call.
fn copied(bytes: Bytes) -> Vec<u8> {
    if bytes.data.is_null() {
        return Vec::new();
    }
    // SAFETY: the caller's buffers outlive the call.
    unsafe { std::slice::from_raw_parts(bytes.data, bytes.length as usize) }.to_vec()
}

/// Whether counted bytes are described as the header requires: an empty
/// buffer as a null pointer and zero length, any other with its pointer.
fn described(bytes: Bytes) -> bool {
    bytes.data.is_null() == (bytes.length == 0)
}

fn tokens(tokens: native::Tokens) -> Vec<String> {
    // SAFETY: the caller's tokens outlive the call.
    let tokens = unsafe { std::slice::from_raw_parts(tokens.data, tokens.count as usize) };
    let text = |token: &Bytes| String::from_utf8(copied(*token)).unwrap();
    tokens.iter().map(text).collect()
}

extern "C" fn create(startup: *const native::Startup, out: *mut Created) -> Call {
    // SAFETY: the owner passes a complete startup.
    let startup = unsafe { &*startup };
    let id = startup.session_id.bytes;
    let script = world().scripts.get(&id).copied().unwrap_or_default();
    if let Some(code) = script.create_fails {
        return failure(code, b"refused", b"create refused");
    }
    let buffers = [
        startup.expected_release,
        startup.metadata_path.bytes,
        startup.home_path.bytes,
        startup.runtime_path.bytes,
        startup.diagnostics.log_path.value.bytes,
        startup.diagnostics.filter,
    ];
    if !buffers.into_iter().all(described) {
        return failure(
            native::INVALID_ARGUMENT,
            b"invalid_argument",
            b"an empty buffer is not null and zero",
        );
    }
    let path = |path: native::Path| (path.encoding, copied(path.bytes));
    let settings = startup.settings;
    let log = startup.diagnostics.log_path;
    let started = Started {
        generation: startup.generation,
        release: copied(startup.expected_release),
        metadata: path(startup.metadata_path),
        home: path(startup.home_path),
        runtime: path(startup.runtime_path),
        settings: (
            settings.escape_enabled,
            settings.escape_byte,
            settings.take_control_enabled,
            settings.take_control_byte,
        ),
        log: (log.present, log.value.encoding, copied(log.value.bytes)),
        filter: copied(startup.diagnostics.filter),
        client_bytes: startup.limits.client_bytes,
        thread: std::thread::current().name().map(str::to_owned),
    };
    let fake: &'static Fake = Box::leak(Box::new(Fake {
        script,
        started,
        state: Mutex::default(),
        changed: Condvar::new(),
        terminal: native::Terminal {
            generation: GENERATION,
            client_outcome: native::CLIENT_FAILED,
            retry: native::RETRY_PASSIVE,
            leave: native::LEAVE_CONFIRMED,
            attachment_detached: 1,
            connection_closed: 1,
            reserved: [0; 2],
            error: Error {
                origin: native::ORIGIN_DAEMON,
                reserved: 0,
                code: bytes_of(b"io_error"),
                message: bytes_of(b"connection reset"),
            },
        },
    }));
    fake.called("create");
    world().fakes.insert(id, fake);
    let handle = ptr::from_ref(fake).cast_mut();
    // SAFETY: the owner passes an output struct.
    unsafe {
        let out = &mut *out;
        out.instance = handle.cast();
        out.communication = handle.cast();
        out.control = handle.cast();
    }
    success()
}

/// Runs until shutdown, then leaves terminal evidence to drain.
extern "C" fn run(instance: *mut Instance, out: *mut RunReport) -> Call {
    let fake = fake(instance);
    if let Some(cleanup) = fake.script.run_fails {
        // SAFETY: the owner passes an output struct.
        unsafe { (*out).workers_cleanup = cleanup };
        return failure(native::INTERNAL_ERROR, b"internal_error", b"run failed");
    }
    let mut state = fake.state();
    while !state.shutdown {
        state = fake.changed.wait(state).unwrap();
    }
    state.queue.push_back(Item::Terminal);
    state.calls.push("run returned".into());
    success()
}

extern "C" fn destroy(instance: *mut Instance) -> Call {
    let fake = fake(instance);
    fake.called("destroy");
    busy(fake.script.destroy_fails)
}

extern "C" fn submit(communication: *mut Communication, request: *const native::Request) -> Call {
    let fake = fake(communication);
    // SAFETY: the caller passes a complete request for this call.
    let request = unsafe { &*request };
    // SAFETY (each arm): the tag names the active member.
    let args = match request.tag {
        native::OPEN => {
            let open = unsafe { request.args.open };
            format!(
                "mode {} geometry {}x{} required {:?} optional {:?}",
                open.mode,
                open.geometry.cols,
                open.geometry.rows,
                tokens(open.required),
                tokens(open.optional)
            )
        }
        native::INPUT => {
            let input = unsafe { request.args.input };
            let held = input.authority;
            format!(
                "authority {}/{}/{} bytes {:?}",
                held.generation,
                held.period,
                held.restoration_id,
                copied(input.bytes)
            )
        }
        native::RESIZE => {
            let resize = unsafe { request.args.resize };
            format!(
                "restoration {} geometry {}x{}",
                resize.authority.restoration_id, resize.geometry.cols, resize.geometry.rows
            )
        }
        native::ACQUIRE => format!("generation {}", unsafe { request.args.generation }),
        native::KILL => format!("required {}", unsafe { request.args.required }.count),
        _ => String::new(),
    };
    fake.called(format!(
        "submit tag {} id {} reserved {}: {args}",
        request.tag, request.request_id, request.reserved
    ));
    busy(fake.script.submit_fails)
}

extern "C" fn drain(communication: *mut Communication, out: *mut Drained) -> Call {
    let fake = fake(communication);
    fake.called("drain");
    if fake.script.drain_fails {
        return failure(native::INTERNAL_ERROR, b"internal_error", b"drain failed");
    }
    let mut state = fake.state();
    // SAFETY: the caller passes an output struct.
    let out = unsafe { &mut *out };
    match state.queue.pop_front() {
        Some(Item::Delivery(records)) => {
            state.delivered.push(records);
            let records = state.delivered.last().unwrap();
            out.kind = native::DELIVERY;
            out.delivery = ptr::from_ref(fake).cast_mut().cast();
            out.records = records.as_ptr();
            out.count = records.len() as u64;
        }
        Some(Item::Terminal) => {
            out.kind = native::TERMINAL;
            out.terminal = &fake.terminal;
        }
        None => out.kind = native::EMPTY,
    }
    success()
}

extern "C" fn release_delivery(delivery: *mut Delivery) -> Call {
    let fake = fake(delivery);
    fake.called("release_delivery");
    busy(fake.script.release_delivery_fails)
}

extern "C" fn shutdown(control: *mut Control) -> Call {
    let fake = fake(control);
    let mut state = fake.state();
    state.calls.push("shutdown".into());
    state.shutdown = true;
    fake.changed.notify_all();
    success()
}

extern "C" fn release_communication(communication: *mut Communication) -> Call {
    let fake = fake(communication);
    fake.called("release_communication");
    busy(fake.script.release_communication_fails)
}

extern "C" fn release_control(control: *mut Control) -> Call {
    let fake = fake(control);
    fake.called("release_control");
    busy(fake.script.release_control_fails)
}

extern "C" fn notification(communication: *mut Communication, out: *mut i32) -> Call {
    let fake = fake(communication);
    fake.called("notification");
    if fake.script.notification_fails {
        return failure(native::INTERNAL_ERROR, b"internal_error", b"no descriptor");
    }
    // SAFETY: the caller passes an output integer.
    unsafe { *out = 41 };
    success()
}

extern "C" fn snapshot(_: *mut Communication, _: *mut Metrics) -> Call {
    success()
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

fn session(index: u8) -> SessionId {
    format!("e3bed000-0000-4000-8000-0000000000{index:02x}")
        .parse()
        .unwrap()
}

fn guid(id: &SessionId) -> [u8; 16] {
    id.as_str()
        .parse::<climon2_core::wire::Guid>()
        .unwrap()
        .bytes()
}

fn metadata(id: &SessionId) -> SessionMetadata {
    serde_json::from_value(serde_json::json!({
        "format_version": 1, "session_id": id.as_str(), "socket_path": "/tmp/s",
        "daemon_pid": 1, "daemon_start_time": 0, "command": "fake", "args": [],
        "cwd": "/", "depth": 1, "created_at": "2026-11-13T10:00:00.000Z",
        "climon2_version": "0.1.0"
    }))
    .unwrap()
}

/// What the test's reader copies from each record: its session, tag and
/// generation, and a display record's bytes, read before its release.
type Read = (String, u32, u64, Vec<u8>);

fn read(id: &SessionId, record: &Record) -> Vec<Read> {
    world().fakes[&guid(id)].called("read");
    let bytes = match record.tag {
        // SAFETY: the tag names the active member.
        native::DISPLAY => copied(unsafe { record.data.display }.bytes),
        _ => Vec::new(),
    };
    vec![(id.as_str().to_owned(), record.tag, record.generation, bytes)]
}

fn record(tag: u32) -> Record {
    // SAFETY: an all-zero record is a valid plain value.
    let mut record: Record = unsafe { std::mem::zeroed() };
    record.tag = tag;
    record.generation = GENERATION;
    record
}

fn display(bytes: &'static [u8]) -> Record {
    let mut record = record(native::DISPLAY);
    record.data.display = native::Display {
        authority: native::Barrier {
            period: 1,
            restoration_id: 1,
        },
        bytes: bytes_of(bytes),
    };
    record
}

/// One instance's owner thread in a climon2 home of its own.
struct Running {
    id: SessionId,
    home: tempfile::TempDir,
    owner: Owner,
    signals: Receiver<Signal<Read>>,
}

impl Running {
    fn fake(&self) -> &'static Fake {
        world().fakes[&guid(&self.id)]
    }

    fn signal(&self) -> Signal<Read> {
        self.signals.recv_timeout(WAIT).expect("an owner signal")
    }

    fn published(&self) -> (Local<Read>, Option<String>) {
        match self.signal() {
            Signal::Published(local, refused) => (local, refused),
            _ => panic!("the handles are published first"),
        }
    }

    fn stopped(&self) -> (Result<(), (String, Retry)>, Result<(), String>) {
        match self.signal() {
            Signal::Stopped(ran, workers) => (ran, workers),
            _ => panic!("run's return is signalled next"),
        }
    }

    fn done(&self) -> Result<(), String> {
        match self.signal() {
            Signal::Done(destroyed) => destroyed,
            _ => panic!("the owner's end is signalled last"),
        }
    }
}

fn environment<'a>(home: &'a HomeLayout, runtime: &'a Path) -> Environment<'a> {
    Environment {
        home,
        runtime,
        escape: ControlKey::from_byte(0x1d).unwrap(),
        take_control: None,
        limits: climon2_core::config::Limits::default().library(),
        filter: "debug",
    }
}

fn layout(directory: &Path) -> HomeLayout {
    HomeLayout::resolve(Some(directory.as_os_str()), Path::new("/")).unwrap()
}

fn start(index: u8, role: Role, script: Script) -> Running {
    let id = session(index);
    world().scripts.insert(guid(&id), script);
    let home = tempfile::tempdir().unwrap();
    let layout = layout(home.path());
    let runtime = home.path().join("run");
    let startup = Startup::new(&environment(&layout, &runtime), &metadata(&id), GENERATION);
    let (tell, signals) = mpsc::channel();
    let tell = move |signal| {
        let _ = tell.send(signal);
    };
    let owner = owner::spawn(
        &FAKE,
        1 << 20,
        startup.unwrap(),
        id.clone(),
        role,
        read,
        tell,
    );
    Running {
        id,
        home,
        owner: owner.unwrap(),
        signals,
    }
}

const OPEN: &str = "submit tag 1 id 1 reserved 0: mode 1 geometry 0x0 \
    required [\"daemon-wire-v1\", \"monitor-v1\"] \
    optional [\"title-v1\", \"status-v1\", \"notification-v1\", \"progress-v1\", \"focus-v1\"]";

/// LIB-06 and COM-05 (over the fake library): the owner thread creates the
/// instance with its session's startup, submits monitor Open as request 1
/// and publishes the handles; requests go in under their IDs and the
/// created generation; each delivery is read, then released at once; run
/// returns after shutdown, leaving terminal evidence; the instance is
/// destroyed only once its consumer has released its handles.
#[test]
fn a_monitoring_instance_runs_from_create_to_destroy_in_order() {
    let running = start(1, Role::Monitor, Script::default());
    let (local, refused) = running.published();
    assert_eq!(refused, None);
    let fake = running.fake();
    assert_eq!(fake.calls(), ["create", OPEN]);
    assert_eq!(OPEN_ID, 1);

    let path = |path: PathBuf| (native_path::ENCODING, native_path::encode(&path));
    let home = running.home.path();
    let started = &fake.started;
    assert_eq!(started.generation, GENERATION);
    assert_eq!(started.release, b"0.1.0");
    let layout = layout(home);
    assert_eq!(started.metadata, path(layout.metadata(&running.id)));
    assert_eq!(started.home, path(layout.root().to_owned()));
    assert_eq!(started.runtime, path(home.join("run")));
    assert_eq!(started.settings, (1, 0x1d, 0, 0));
    assert_eq!(started.filter, b"debug");
    let limits = climon2_core::config::Limits::default().library();
    assert_eq!(started.client_bytes, limits.client_bytes);
    assert_eq!(started.thread.as_deref(), Some("client"));
    let (present, encoding, log) = started.log.clone();
    assert_eq!((present, encoding), (1, native_path::ENCODING));
    let logs = layout.root().join("logs").join(running.id.as_str());
    assert!(logs.is_dir(), "the session's log directory is created");
    let prefix = native_path::encode(&logs.join("library-"));
    assert!(log.starts_with(&prefix), "{log:?}");
    assert!(log.ends_with(format!("-{GENERATION}.log").as_bytes()));

    assert_eq!(local.notification(), Ok(41));
    let held = Authority {
        generation: 4,
        period: 5,
        restoration: 6,
    };
    assert_eq!(
        local.submit(9, &Request::Input(held, b"hi".to_vec())),
        Ok(())
    );
    assert_eq!(local.submit(10, &Request::Acquire), Ok(()));
    let size = TerminalSize { cols: 80, rows: 24 };
    assert_eq!(local.submit(11, &Request::Resize(held, size)), Ok(()));
    assert_eq!(local.kill(12), Ok(()));
    assert_eq!(
        fake.calls()[2..],
        [
            "notification",
            "submit tag 2 id 9 reserved 0: authority 4/5/6 bytes [104, 105]",
            "submit tag 4 id 10 reserved 0: generation 7",
            "submit tag 3 id 11 reserved 0: restoration 6 geometry 80x24",
            "submit tag 9 id 12 reserved 0: required 0",
        ]
    );

    let delivery = vec![display(b"shown"), record(native::RESTORATION_COMPLETE)];
    fake.state().queue.push_back(Item::Delivery(delivery));
    let id = running.id.as_str().to_owned();
    let Drain::Events(events) = local.drain(&running.id) else {
        panic!("a delivery");
    };
    assert_eq!(
        events,
        [
            (id.clone(), native::DISPLAY, GENERATION, b"shown".to_vec()),
            (id, native::RESTORATION_COMPLETE, GENERATION, Vec::new()),
        ]
    );
    assert_eq!(
        fake.calls()[7..],
        ["drain", "read", "read", "release_delivery"]
    );
    assert!(matches!(local.drain(&running.id), Drain::Stop(Stop::Empty)));

    local.shutdown();
    assert_eq!(running.stopped(), (Ok(()), Ok(())));
    let Drain::Stop(Stop::Terminal(message, retry, terminal)) = local.drain(&running.id) else {
        panic!("terminal evidence");
    };
    assert_eq!(message, "connection reset");
    assert_eq!(retry, Retry::Passive);
    assert_eq!(
        terminal,
        Terminal {
            failed: Some("io_error: connection reset".into()),
            leave: Leave::Confirmed,
            detached: true,
            closed: true,
        }
    );
    // The owner waits for the release: nothing is destroyed before it.
    assert!(
        running
            .signals
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    assert!(!fake.calls().contains(&"destroy".to_owned()));
    assert_eq!(local.release(&running.id), Ok(()));
    running.owner.release();
    assert_eq!(running.done(), Ok(()));
    running.owner.join();
    assert_eq!(
        fake.calls()[11..],
        [
            "drain",
            "shutdown",
            "run returned",
            "drain",
            "release_communication",
            "release_control",
            "destroy",
        ]
    );
}

/// DEL-01 (over the fake library, #117): a Kill-only instance submits its
/// Kill as request 2, requiring no family, and never monitor Open.
#[test]
fn a_kill_only_instance_submits_its_kill_and_never_opens() {
    let running = start(2, Role::KillOnly, Script::default());
    let (local, refused) = running.published();
    assert_eq!(refused, None);
    assert_eq!(KILL_ID, 2);
    assert_eq!(
        running.fake().calls(),
        ["create", "submit tag 9 id 2 reserved 0: required 0"]
    );
    local.shutdown();
    assert_eq!(running.stopped(), (Ok(()), Ok(())));
    assert_eq!(local.release(&running.id), Ok(()));
    running.owner.release();
    assert_eq!(running.done(), Ok(()));
}

/// COM-05 (over the fake library): a first request the library refuses is
/// published with its refusal, and the instance is asked to stop before
/// it runs, so run ends with terminal evidence.
#[test]
fn a_refused_first_request_is_published_with_why_and_stops_the_instance() {
    let script = Script {
        submit_fails: true,
        ..Script::default()
    };
    for (index, role) in [(3, Role::Monitor), (4, Role::KillOnly)] {
        let running = start(index, role, script);
        let (local, refused) = running.published();
        assert_eq!(refused.as_deref(), Some("busy: still in use"));
        assert_eq!(running.fake().calls()[2], "shutdown", "{role:?}");
        assert_eq!(running.stopped(), (Ok(()), Ok(())));
        assert_eq!(
            local.submit(3, &Request::Acquire),
            Err("busy: still in use".into())
        );
        assert_eq!(local.kill(3), Err("busy: still in use".into()));
    }
}

/// REC-01 and LIB-06 (over the fake library): a refused create stops with
/// why and its retry class, publishes no handles and destroys nothing.
#[test]
fn a_refused_create_stops_without_handles_and_destroys_nothing() {
    for (index, code, retry) in [
        (5, native::INVALID_ARGUMENT, Retry::Permanent),
        (6, native::RESOURCE_LIMIT, Retry::Passive),
    ] {
        let script = Script {
            create_fails: Some(code),
            ..Script::default()
        };
        let running = start(index, Role::Monitor, script);
        let refusal = ("refused: create refused".to_owned(), retry);
        assert_eq!(running.stopped(), (Err(refusal), Ok(())));
        assert_eq!(running.done(), Ok(()));
        assert!(!world().fakes.contains_key(&guid(&running.id)));
        running.owner.join();
    }
}

/// EXT-02 and LIB-06 (over the fake library, #118): a failed run is
/// retried passively, with what its report says of its workers.
#[test]
fn a_failed_run_stops_with_why_and_what_its_report_says_of_its_workers() {
    for (index, cleanup, workers) in [
        (7, native::CLEANUP_COMPLETE, Ok(())),
        (
            8,
            native::CLEANUP_INCOMPLETE,
            Err("worker cleanup incomplete".to_owned()),
        ),
    ] {
        let script = Script {
            run_fails: Some(cleanup),
            ..Script::default()
        };
        let running = start(index, Role::Monitor, script);
        let (local, _) = running.published();
        let failed = ("internal_error: run failed".to_owned(), Retry::Passive);
        assert_eq!(running.stopped(), (Err(failed), workers));
        assert_eq!(local.release(&running.id), Ok(()));
        running.owner.release();
        assert_eq!(running.done(), Ok(()));
    }
}

/// EXT-02 (over the fake library, #118): an owner dropped without the
/// release never destroys its instance, whose handles may still be in
/// use, and says so; a destroy the library refuses is reported as itself.
#[test]
fn an_instance_is_never_destroyed_unreleased_and_a_failed_destroy_is_reported() {
    let running = start(9, Role::Monitor, Script::default());
    let (local, _) = running.published();
    local.shutdown();
    assert_eq!(running.stopped(), (Ok(()), Ok(())));
    let Running {
        owner,
        signals,
        id,
        home: _home,
    } = running;
    drop(owner);
    let Ok(Signal::Done(destroyed)) = signals.recv_timeout(WAIT) else {
        panic!("the owner's end");
    };
    assert_eq!(destroyed, Err("its handles may still be in use".to_owned()));
    let calls = world().fakes[&guid(&id)].calls();
    assert!(!calls.contains(&"destroy".to_owned()), "{calls:?}");

    let script = Script {
        destroy_fails: true,
        ..Script::default()
    };
    let running = start(10, Role::Monitor, script);
    let (local, _) = running.published();
    local.shutdown();
    assert_eq!(running.stopped(), (Ok(()), Ok(())));
    running.owner.release();
    assert_eq!(running.done(), Err("busy: still in use".to_owned()));
    assert_eq!(running.fake().calls().last().unwrap(), "destroy");
}

/// Sets its flag once the owner thread has let go of what it was given.
struct Ended(Arc<AtomicBool>);

impl Drop for Ended {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// EXT-02 (over the fake library, #118): joining an owner waits for its
/// thread to end, however long that takes after its last signal.
#[test]
fn joining_an_owner_waits_for_its_thread_to_end() {
    let id = session(14);
    let script = Script {
        create_fails: Some(native::RESOURCE_LIMIT),
        ..Script::default()
    };
    world().scripts.insert(guid(&id), script);
    let home = tempfile::tempdir().unwrap();
    let layout = layout(home.path());
    let runtime = home.path().join("run");
    let startup = Startup::new(&environment(&layout, &runtime), &metadata(&id), 1).unwrap();
    let ended = Arc::new(AtomicBool::new(false));
    let guard = Ended(Arc::clone(&ended));
    let (tell, signals) = mpsc::channel();
    let tell = move |signal: Signal<Read>| {
        let _held = &guard;
        let last = matches!(signal, Signal::Done(_));
        let _ = tell.send(signal);
        if last {
            std::thread::sleep(Duration::from_millis(200));
        }
    };
    let owner = owner::spawn(&FAKE, 1 << 20, startup, id, Role::Monitor, read, tell).unwrap();
    assert!(matches!(
        signals.recv_timeout(WAIT),
        Ok(Signal::Stopped(..))
    ));
    assert!(matches!(signals.recv_timeout(WAIT), Ok(Signal::Done(_))));
    assert!(!ended.load(Ordering::SeqCst), "the thread is still ending");
    owner.join();
    assert!(ended.load(Ordering::SeqCst));
}

/// LIB-06 (over the fake library): a failed notification lookup and a
/// failed drain say why; a delivery whose release fails is still read;
/// both handles are released even when one release fails, and the failure
/// is reported.
#[test]
fn failed_calls_are_reported_as_their_code_and_message() {
    let script = Script {
        notification_fails: true,
        release_delivery_fails: true,
        release_communication_fails: true,
        ..Script::default()
    };
    let running = start(11, Role::Monitor, script);
    let (local, _) = running.published();
    let fake = running.fake();
    assert_eq!(
        local.notification(),
        Err("internal_error: no descriptor".into())
    );
    let delivery = vec![record(native::RESTORATION_COMPLETE)];
    fake.state().queue.push_back(Item::Delivery(delivery));
    let Drain::Events(events) = local.drain(&running.id) else {
        panic!("a delivery");
    };
    assert_eq!(events.len(), 1);
    local.shutdown();
    assert_eq!(running.stopped(), (Ok(()), Ok(())));
    assert_eq!(local.release(&running.id), Err("busy: still in use".into()));
    let calls = fake.calls();
    assert_eq!(
        calls[calls.len() - 2..],
        ["release_communication", "release_control"]
    );

    let script = Script {
        drain_fails: true,
        release_control_fails: true,
        ..Script::default()
    };
    let running = start(12, Role::Monitor, script);
    let (local, _) = running.published();
    let Drain::Stop(Stop::Failed(why)) = local.drain(&running.id) else {
        panic!("a failed drain");
    };
    assert_eq!(why, "internal_error: drain failed");
    local.shutdown();
    assert_eq!(running.stopped(), (Ok(()), Ok(())));
    assert_eq!(local.release(&running.id), Err("busy: still in use".into()));
}

/// What create was given for session `index`'s startup in connection 1,
/// with `take_control` and the library log's `filter` in a fresh home.
fn started(index: u8, take_control: Option<ControlKey>, filter: &str) -> Started {
    let id = session(index);
    world().scripts.insert(guid(&id), Script::default());
    let home = tempfile::tempdir().unwrap();
    let layout = layout(home.path());
    let runtime = home.path().join("run");
    let environment = Environment {
        take_control,
        filter,
        ..environment(&layout, &runtime)
    };
    let startup = Startup::new(&environment, &metadata(&id), 1).unwrap();
    let (tell, signals) = mpsc::channel();
    let tell = move |signal| {
        let _ = tell.send(signal);
    };
    let owner = owner::spawn(
        &FAKE,
        1 << 20,
        startup,
        id.clone(),
        Role::Monitor,
        read,
        tell,
    );
    let Ok(Signal::Published(local, None)) = signals.recv_timeout(WAIT) else {
        panic!("create accepted the startup and the handles were published");
    };
    local.shutdown();
    drop(owner);
    world().fakes[&guid(&id)].started.clone()
}

/// A configured take-control key is enabled with its byte; the library
/// log's directory that cannot be created is the startup's error.
#[test]
fn a_startup_carries_the_take_control_key_and_fails_without_its_log_directory() {
    let started = started(13, ControlKey::from_byte(0x14), "debug");
    assert_eq!(started.settings, (1, 0x1d, 1, 0x14));
    assert_eq!(started.generation, 1);

    let home = tempfile::tempdir().unwrap();
    let blocked = home.path().join("blocked");
    std::fs::write(&blocked, b"a file where a home is wanted").unwrap();
    let nowhere = layout(&blocked);
    let runtime = home.path().join("run");
    let environment = environment(&nowhere, &runtime);
    assert!(Startup::new(&environment, &metadata(&session(13)), 1).is_err());
}

/// An empty library log filter reaches create as the header's empty
/// buffer, a null pointer and zero length, which create accepts.
#[test]
fn an_empty_log_filter_reaches_create_as_an_empty_buffer() {
    assert_eq!(started(15, None, "").filter, b"");
}
