//! One client instance (native revision 1's lifecycle): the state its owner
//! thread, its communication and control handles, its deliveries and its
//! workers share. Every lock here belongs to one instance; no process-global
//! lock serializes instances.
//!
//! The stream ends once: with shutdown, or with the first failure, such as a
//! contained panic. A failure queues one best-effort library diagnostic, and
//! `run` then publishes terminal evidence after it. Without an opened
//! connection (Open arrives in #93) Leave is never required and no daemon
//! connection remains open.

#[cfg(feature = "test-hooks")]
use super::REQUIREMENTS;
use super::{
    failure::{
        BUSY, CLOSED, Failure, INVALID_ARGUMENT, PANIC, RESOURCE_LIMIT, TIMEOUT, WRONG_THREAD,
        require,
    },
    hook,
    platform::Notifier,
    startup::{Diagnostics, Paths, Startup},
};
use climon2_core::{
    diagnostics::{CappedWriter, dispatch},
    keys::{ClientSettings, ControlKey},
    native,
};
use std::{
    fs::OpenOptions,
    mem,
    ops::{AddAssign, SubAssign},
    os::unix::fs::OpenOptionsExt,
    panic::{self, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    thread::{self, JoinHandle, ThreadId},
    time::Duration,
};
use tracing::{Dispatch, Span};

/// Bytes one record is charged while queued or delivered and unreleased: the
/// record itself, since library diagnostics point at static text.
const RECORD: u64 = mem::size_of::<native::Record>() as u64;
/// One delivery's allocation.
const DELIVERY: u64 = mem::size_of::<Delivery>() as u64;
/// The allocation of an instance's own structures: its shared state, its
/// owner's record and both handles.
const FIXED: u64 = mem::size_of::<(Shared, Owner, Communication, Control)>() as u64;

// SAFETY (all three): every field is an integer or a raw pointer, for which
// zero (null) is a valid value.
const NO_ERROR: native::Error = unsafe { mem::zeroed() };
const NO_DRAIN: native::Drained = unsafe { mem::zeroed() };
const NO_METRICS: native::Metrics = unsafe { mem::zeroed() };

/// The communication and control handles: both must be released before
/// `destroy`.
const HANDLES: u8 = 2;

/// A native value that may cross threads: `Send` and `Sync`, because its
/// pointers reach only immutable static data (the library's error text).
#[derive(Clone, Copy)]
struct Shareable<T>(T);

// SAFETY: the wrapped value's pointers only reach immutable static data.
unsafe impl<T> Send for Shareable<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Shareable<T> {}

/// A current measurement and its peak.
#[derive(Clone, Copy, Default)]
struct Gauge<T> {
    current: T,
    peak: T,
}

impl<T: Copy + Ord + AddAssign + SubAssign> Gauge<T> {
    fn add(&mut self, amount: T) {
        self.current += amount;
        self.peak = self.peak.max(self.current);
    }

    fn sub(&mut self, amount: T) {
        self.current -= amount;
    }
}

/// What `snapshot` reports. Records and deliveries are accounted here
/// alone: a record is charged from its queueing until its delivery's release.
#[derive(Default)]
struct Measures {
    allocation: Gauge<u64>,
    event_bytes: Gauge<u64>,
    event_records: Gauge<u32>,
    deliveries: Gauge<u32>,
    workers: Gauge<u32>,
    drained: u64,
    released: u64,
}

impl Measures {
    /// One record queued for drain.
    fn queue(&mut self) {
        self.event_records.add(1);
        self.event_bytes.add(RECORD);
    }

    /// The queued record drained in a delivery of its own.
    fn deliver(&mut self) {
        self.deliveries.add(1);
        self.allocation.add(DELIVERY);
        self.drained += 1;
    }

    /// One delivery released through this library, and its record with it.
    fn release(&mut self) {
        self.deliveries.sub(1);
        self.event_records.sub(1);
        self.event_bytes.sub(RECORD);
        self.allocation.sub(DELIVERY);
        self.released += 1;
    }
}

/// The first failure, and the retry it calls for: passive, unless shutdown
/// had already been requested.
#[derive(Clone, Copy)]
struct Failed {
    failure: Failure,
    retry: u32,
}

/// Why the stream ends, once it does.
#[derive(Default)]
struct Stop {
    shutdown: bool,
    failure: Option<Failed>,
}

impl Stop {
    fn running(&self) -> bool {
        !self.shutdown && self.failure.is_none()
    }
}

#[derive(Default)]
struct State {
    stop: Stop,
    /// The failure diagnostic, until drained.
    pending: Option<Shareable<native::Record>>,
    /// Whether the notification descriptor is currently raised.
    readable: bool,
    /// How many of the [`HANDLES`] are released.
    released_handles: u8,
    measures: Measures,
}

pub(super) struct Shared {
    generation: u64,
    limits: native::Limits,
    paths: Paths,
    settings: ClientSettings,
    descriptors: u32,
    notifier: Notifier,
    dispatch: Dispatch,
    /// Names this instance in every record: its generation.
    span: Span,
    terminal: OnceLock<Shareable<native::Terminal>>,
    state: Mutex<State>,
    changed: Condvar,
}

/// Behind `cm1_instance`, which only the thread that created it may run or
/// destroy. It is only ever borrowed shared, so a call from another thread is
/// refused safely while the owner thread runs.
pub(super) struct Owner {
    shared: Arc<Shared>,
    thread: ThreadId,
    /// The owner thread's own state, used only after its thread check.
    run: Mutex<Run>,
}

/// What `run` leaves behind: its workers, and its cleanup once it returned.
#[derive(Default)]
struct Run {
    workers: Vec<JoinHandle<()>>,
    cleanup: Option<Cleanup>,
}

/// `run`'s worker cleanup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cleanup {
    /// Every worker ended and was joined.
    Complete,
    /// A worker outlived `cleanup_ms`: never joined, never abandoned.
    Incomplete,
}

impl Cleanup {
    const fn code(self) -> u32 {
        match self {
            Self::Complete => native::CLEANUP_COMPLETE,
            Self::Incomplete => native::CLEANUP_INCOMPLETE,
        }
    }
}

/// Behind `cm1_communication`.
pub(super) struct Communication {
    pub(super) shared: Arc<Shared>,
}

/// Behind `cm1_control`.
pub(super) struct Control {
    pub(super) shared: Arc<Shared>,
}

/// Behind `cm1_delivery`: one immutable batch. Dropping it, which only its
/// release does, returns everything it was charged.
pub(super) struct Delivery {
    shared: Arc<Shared>,
    record: native::Record,
}

impl Drop for Delivery {
    fn drop(&mut self) {
        self.shared.lock().measures.release();
    }
}

/// A library object behind a native handle, owned by one instance.
pub(super) trait Object {
    fn shared(&self) -> &Arc<Shared>;
}

impl Object for Owner {
    fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

impl Object for Communication {
    fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

impl Object for Control {
    fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

impl Object for Delivery {
    fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }
}

pub(super) type Handles = (Box<Owner>, Box<Communication>, Box<Control>);

/// Local setup only: the notification descriptor, then the optional log
/// file, the last step that can fail, so a failure leaves nothing behind.
pub(super) fn create(startup: Startup) -> Result<Handles, Failure> {
    let notifier = Notifier::new().map_err(|_| {
        Failure::new(
            RESOURCE_LIMIT,
            "the notification descriptor cannot be created",
        )
    })?;
    let Startup {
        generation,
        paths,
        settings,
        diagnostics,
        limits,
    } = startup;
    let (dispatch, logs) = log(diagnostics)?;
    let span = tracing::dispatcher::with_default(&dispatch, || {
        tracing::info_span!("client_instance", generation)
    });
    let allocation = FIXED + paths.bytes();
    let shared = Arc::new(Shared {
        generation,
        limits,
        paths,
        settings,
        descriptors: Notifier::DESCRIPTORS + logs,
        notifier,
        dispatch,
        span,
        terminal: OnceLock::new(),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
    });
    shared.lock().measures.allocation.add(allocation);
    shared.scope(|| tracing::info!("client instance created"));
    let owner = Owner {
        shared: Arc::clone(&shared),
        thread: thread::current().id(),
        run: Mutex::default(),
    };
    let communication = Communication {
        shared: Arc::clone(&shared),
    };
    Ok((
        Box::new(owner),
        Box::new(communication),
        Box::new(Control { shared }),
    ))
}

/// The instance's own diagnostics and the descriptors they hold: a capped,
/// private log file created at the caller's path, or nothing.
fn log(diagnostics: Diagnostics) -> Result<(Dispatch, u32), Failure> {
    let Some(path) = diagnostics.log else {
        return Ok((Dispatch::none(), 0));
    };
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| Failure::new(INVALID_ARGUMENT, "log_path cannot be created as a new file"))?;
    Ok((dispatch(CappedWriter::new(file), diagnostics.filter), 1))
}

/// The record a failure leaves in the stream.
fn diagnostic(generation: u64, failure: Failure) -> native::Record {
    // SAFETY: every field of a record is an integer, a raw pointer or a union
    // of them, for which zero is valid.
    let mut record: native::Record = unsafe { mem::zeroed() };
    record.tag = native::DIAGNOSTIC;
    record.generation = generation;
    record.data.diagnostic = failure.error();
    record
}

/// This generation's terminal evidence: never opened, so Leave is not
/// required and no daemon connection remains open.
fn terminal(generation: u64, stop: &Stop) -> native::Terminal {
    let failed = stop.failure;
    native::Terminal {
        generation,
        client_outcome: failed.map_or(native::CLIENT_STOPPED, |_| native::CLIENT_FAILED),
        retry: failed.map_or(native::RETRY_NONE, |failed| failed.retry),
        leave: native::LEAVE_NOT_REQUIRED,
        attachment_detached: 0,
        connection_closed: 1,
        reserved: [0; 2],
        error: failed.map_or(NO_ERROR, |failed| failed.failure.error()),
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn scope<T>(&self, work: impl FnOnce() -> T) -> T {
        tracing::dispatcher::with_default(&self.dispatch, || self.span.in_scope(work))
    }

    /// One exported call's work on this instance, under its diagnostics. A
    /// panic is contained: the call fails explicitly and so does the instance.
    pub(super) fn guard<T>(
        &self,
        call: &'static str,
        work: impl FnOnce() -> Result<T, Failure>,
    ) -> Result<T, Failure> {
        self.scope(|| {
            panic::catch_unwind(AssertUnwindSafe(|| {
                hook(call);
                work()
            }))
            .unwrap_or_else(|_| {
                tracing::error!(call, "client library contained a panic");
                self.fail(PANIC);
                Err(PANIC)
            })
        })
    }

    /// The first failure ends the stream, unless it has already ended.
    fn fail(&self, failure: Failure) {
        let mut state = self.lock();
        if state.stop.failure.is_none() && self.terminal.get().is_none() {
            let retry = if state.stop.shutdown {
                native::RETRY_NONE
            } else {
                native::RETRY_PASSIVE
            };
            state.stop.failure = Some(Failed { failure, retry });
            self.queue(&mut state, failure);
            self.changed.notify_all();
        }
    }

    /// Queues the failure's diagnostic if it fits the event allowance; it is
    /// best effort, and the terminal evidence keeps the error regardless.
    fn queue(&self, state: &mut State, failure: Failure) {
        if RECORD <= self.limits.event_bytes {
            state.pending = Some(Shareable(diagnostic(self.generation, failure)));
            state.measures.queue();
        }
        self.signal(state);
    }

    /// Raises the notification descriptor while a record or the terminal
    /// evidence is ready to drain, and clears it otherwise.
    fn signal(&self, state: &mut State) {
        let ready = state.pending.is_some() || self.terminal.get().is_some();
        if ready != state.readable {
            state.readable = ready;
            self.notifier.set(ready);
        }
    }

    pub(super) fn shutdown(&self) -> Result<(), Failure> {
        self.lock().stop.shutdown = true;
        self.changed.notify_all();
        tracing::debug!("client instance shutdown requested");
        Ok(())
    }

    fn wait_for_stop(&self) {
        let state = self.lock();
        drop(
            self.changed
                .wait_while(state, |state| state.stop.running())
                .unwrap_or_else(PoisonError::into_inner),
        );
    }

    /// Publishes terminal evidence, after any record already queued.
    fn finish(&self) -> native::Terminal {
        let mut state = self.lock();
        let evidence = self
            .terminal
            .get_or_init(|| Shareable(terminal(self.generation, &state.stop)))
            .0;
        self.signal(&mut state);
        tracing::info!(
            outcome = evidence.client_outcome,
            retry = evidence.retry,
            "client instance terminal evidence published"
        );
        evidence
    }

    /// Whether every worker ended before `deadline` passed.
    fn wait_for_workers(&self, deadline: Duration) -> bool {
        let state = self.lock();
        let (state, waited) = self
            .changed
            .wait_timeout_while(state, deadline, |state| state.measures.workers.current > 0)
            .unwrap_or_else(PoisonError::into_inner);
        drop(state);
        !waited.timed_out()
    }

    /// One delivery, empty, or terminal evidence once every earlier record
    /// has been drained.
    pub(super) fn drain(self: &Arc<Self>) -> native::Drained {
        let mut state = self.lock();
        let drained = match state.pending.take() {
            Some(record) => {
                state.measures.deliver();
                self.deliver(record.0)
            }
            None => self.ended(),
        };
        self.signal(&mut state);
        drained
    }

    fn deliver(self: &Arc<Self>, record: native::Record) -> native::Drained {
        let delivery = Box::new(Delivery {
            shared: Arc::clone(self),
            record,
        });
        let records = &raw const delivery.record;
        native::Drained {
            kind: native::DELIVERY,
            delivery: Box::into_raw(delivery).cast(),
            records,
            count: 1,
            ..NO_DRAIN
        }
    }

    fn ended(&self) -> native::Drained {
        self.terminal.get().map_or(
            native::Drained {
                kind: native::EMPTY,
                ..NO_DRAIN
            },
            |evidence| native::Drained {
                kind: native::TERMINAL,
                terminal: &raw const evidence.0,
                ..NO_DRAIN
            },
        )
    }

    /// The stream has ended for its consumer: terminal evidence is published
    /// and every earlier record drained.
    fn ended_for_consumer(&self, state: &State) -> bool {
        self.terminal.get().is_some() && state.pending.is_none()
    }

    /// Releases one of the [`HANDLES`] once the stream has ended for its
    /// consumer and nothing else keeps that handle `in_use`.
    fn release_handle(
        &self,
        in_use: impl FnOnce(&State) -> bool,
        busy: &'static str,
    ) -> Result<(), Failure> {
        let mut state = self.lock();
        let idle = self.ended_for_consumer(&state) && !in_use(&state);
        require(idle, Failure::new(BUSY, busy))?;
        state.released_handles += 1;
        Ok(())
    }

    pub(super) fn descriptor(&self) -> i32 {
        self.notifier.descriptor()
    }

    pub(super) fn snapshot(&self) -> native::Metrics {
        let state = self.lock();
        let measures = &state.measures;
        native::Metrics {
            allocation_bytes: measures.allocation.current,
            peak_allocation_bytes: measures.allocation.peak,
            event_bytes: measures.event_bytes.current,
            peak_event_bytes: measures.event_bytes.peak,
            records_drained: measures.drained,
            deliveries_released: measures.released,
            event_records: measures.event_records.current,
            peak_event_records: measures.event_records.peak,
            deliveries: measures.deliveries.current,
            peak_deliveries: measures.deliveries.peak,
            workers: measures.workers.current,
            peak_workers: measures.workers.peak,
            descriptors: self.descriptors,
            peak_descriptors: self.descriptors,
            ..NO_METRICS
        }
    }
}

impl Communication {
    /// Busy until the stream has ended for its consumer and every delivery
    /// is released.
    pub(super) fn release(&self) -> Result<(), Failure> {
        self.shared.release_handle(
            |state| state.measures.deliveries.current > 0,
            "the stream has not ended, or deliveries are still outstanding",
        )
    }
}

impl Control {
    /// Busy until the stream has ended for its consumer.
    pub(super) fn release(&self) -> Result<(), Failure> {
        self.shared
            .release_handle(|_| false, "the stream has not ended")
    }
}

impl Owner {
    fn owned(&self) -> Result<(), Failure> {
        require(
            thread::current().id() == self.thread,
            Failure::new(
                WRONG_THREAD,
                "only the instance's owner thread may call this",
            ),
        )
    }

    fn run_state(&self) -> MutexGuard<'_, Run> {
        self.run.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs until shutdown or failure, publishes terminal evidence, then
    /// waits up to `cleanup_ms` for its workers. The report is separate from
    /// the call's result: a failed run still reports what it knows.
    pub(super) fn run(&self) -> Result<(Result<(), Failure>, native::RunReport), Failure> {
        self.owned()?;
        let mut run = self.run_state();
        require(
            run.cleanup.is_none(),
            Failure::new(CLOSED, "run has already returned"),
        )?;
        let shared = &self.shared;
        let executed = shared.guard("run", || {
            run.execute(shared);
            Ok(())
        });
        let terminal = shared.scope(|| shared.finish());
        let (cleanup, cleaned) = shared.scope(|| run.clean_up(shared));
        run.cleanup = Some(cleanup);
        let report = native::RunReport {
            workers_cleanup: cleanup.code(),
            reserved: 0,
            terminal,
        };
        Ok((executed.and(cleaned), report))
    }

    /// Busy until `run` has returned with complete worker cleanup and both
    /// handles are released; deliveries are released before communication.
    pub(super) fn destroy(&self) -> Result<(), Failure> {
        self.owned()?;
        require(
            self.finished(),
            Failure::new(
                BUSY,
                "run, workers, handles or deliveries are still outstanding",
            ),
        )?;
        tracing::debug!("client instance destroyed");
        Ok(())
    }

    /// Nothing of the instance remains in use.
    fn finished(&self) -> bool {
        self.run_state().cleanup == Some(Cleanup::Complete)
            && self.shared.lock().released_handles == HANDLES
    }
}

impl Run {
    fn execute(&mut self, shared: &Arc<Shared>) {
        let settings = shared.settings;
        tracing::debug!(
            metadata = ?shared.paths.metadata,
            home = ?shared.paths.home,
            runtime = ?shared.paths.runtime,
            escape = ?settings.escape_prefix().map(ControlKey::byte),
            take_control = ?settings.take_control_key().map(ControlKey::byte),
            "client instance running"
        );
        self.start_workers(shared);
        shared.wait_for_stop();
    }

    /// Waits up to `cleanup_ms` for every worker, then joins them; a worker
    /// still running is reported, never joined or abandoned as complete.
    fn clean_up(&mut self, shared: &Shared) -> (Cleanup, Result<(), Failure>) {
        let deadline = Duration::from_millis(shared.limits.cleanup_ms.into());
        if !shared.wait_for_workers(deadline) {
            tracing::warn!("client instance worker cleanup incomplete");
            return (
                Cleanup::Incomplete,
                Err(Failure::new(TIMEOUT, "worker cleanup exceeded cleanup_ms")),
            );
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        tracing::info!("client instance worker cleanup complete");
        (Cleanup::Complete, Ok(()))
    }

    /// Without the `test-hooks` faults, nothing in this release needs a
    /// worker: Open (#93) brings the first.
    #[cfg(not(feature = "test-hooks"))]
    fn start_workers(&mut self, _: &Arc<Shared>) {}

    /// `worker` and `stall-cleanup` start one worker, whose stack is the
    /// release's qualified worker stack.
    #[cfg(feature = "test-hooks")]
    fn start_workers(&mut self, shared: &Arc<Shared>) {
        if let Some(stalls) = super::hooks::worker() {
            let counted = Counted::new(shared);
            let worker = thread::Builder::new()
                .name("climon2-client-worker".into())
                .stack_size(REQUIREMENTS.worker_stack_bytes as usize)
                .spawn(move || work(counted, stalls))
                .expect("the worker starts");
            self.workers.push(worker);
        }
    }
}

/// One worker in its instance's count, from its start until it ends, even
/// by unwinding.
#[cfg(feature = "test-hooks")]
struct Counted(Arc<Shared>);

#[cfg(feature = "test-hooks")]
impl Counted {
    fn new(shared: &Arc<Shared>) -> Self {
        shared.lock().measures.workers.add(1);
        Self(Arc::clone(shared))
    }
}

#[cfg(feature = "test-hooks")]
impl Drop for Counted {
    fn drop(&mut self) {
        self.0.lock().measures.workers.sub(1);
        self.0.changed.notify_all();
    }
}

/// A worker that ends with its instance's stream, or whose cleanup stalls.
#[cfg(feature = "test-hooks")]
fn work(worker: Counted, stalls: bool) {
    worker.0.wait_for_stop();
    if stalls {
        loop {
            thread::park();
        }
    }
}
