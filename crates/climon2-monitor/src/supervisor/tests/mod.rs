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
        Bytes, Call, Communication, Control, Created, Delivery, Drained, Error, Instance, Metrics,
        Record, Request, RunReport, Terminal,
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
    /// Run ignores shutdown until the test lets it go.
    stalls: bool,
    /// After Open, one delivery of control and display records.
    presents: bool,
}

/// What the observer saw of one session's instance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Seen {
    Destroyed(u32),
    OwnerExited,
}

/// A request the fake received: tag, request ID and its arguments' words
/// (generation; or generation, period, restoration; plus columns and rows).
type Submitted = (u32, u64, Vec<u64>);

#[derive(Default)]
struct World {
    scripts: BTreeMap<[u8; 16], Script>,
    seen: Vec<([u8; 16], Seen)>,
    unstalled: BTreeSet<[u8; 16]>,
    submitted: Vec<Submitted>,
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
    while !(fake.state().shutdown && (!fake.script.stalls || world.unstalled.contains(&fake.id))) {
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
            .push((request.tag, request.request_id, words));
    });
    // Ready is refused; anything else accepted, after an answer to a
    // request never made.
    let fake = fake(communication);
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
            while unsafe { libc::read(fake.pipe[0], byte.as_mut_ptr().cast(), byte.len()) } > 0 {}
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
    let directory = tempfile::tempdir().unwrap();
    let home = HomeLayout::resolve(Some(directory.path().as_os_str()), Path::new("/")).unwrap();
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
        world().submitted,
        [
            (native::ACQUIRE, 2, vec![GENERATION]),
            (native::READY, 3, vec![1, 5, 6]),
            (native::RESIZE, 4, vec![1, 5, 6, 120, 29]),
        ]
    );
}
