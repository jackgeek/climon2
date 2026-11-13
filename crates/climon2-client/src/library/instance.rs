//! One client instance (native revision 1's lifecycle and its queues): the
//! state its owner thread, its communication and control handles, its
//! deliveries and its workers share. Every lock here belongs to one
//! instance; no process-global lock serializes instances.
//!
//! The owner thread's `run` drives the instance's daemon connection
//! (`connection.rs`). `submit` only validates, copies and queues a request,
//! and `drain` and `release_delivery` only move records: none performs
//! daemon I/O or waits. The stream ends once (with shutdown, a departure,
//! the connection's failure or a contained panic), and `run` then publishes
//! terminal evidence behind every record already queued.

#[cfg(feature = "test-hooks")]
use super::REQUIREMENTS;
use super::{
    connection,
    failure::{
        BUSY, CANCELLED, CLOSED, FULL, Failed, Failure, Fault, INVALID_ARGUMENT,
        INVALID_REQUEST_ID, PANIC, RESOURCE_LIMIT, TIMEOUT, WRONG_THREAD, require,
    },
    hook,
    platform::Notifier,
    records::{Batch, Event, RECORD, Response},
    request::{self, Operation, Submitted},
    startup::{Diagnostics, Paths, Startup},
    stream::Departure,
};
use climon2_core::{
    diagnostics::{CappedWriter, subscriber},
    keys::{ClientSettings, ControlKey},
    native,
};
use std::{
    collections::VecDeque,
    fs::OpenOptions,
    mem,
    ops::{AddAssign, SubAssign},
    os::unix::fs::OpenOptionsExt,
    panic::{self, AssertUnwindSafe},
    path::Path,
    sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, PoisonError},
    thread::{self, JoinHandle, ThreadId},
    time::Duration,
};
use tracing::{Dispatch, Span};

/// One delivery's own allocation, beside the records it holds.
const DELIVERY: u64 = mem::size_of::<Delivery>() as u64;
/// The allocation of an instance's own structures: its shared state, its
/// owner's record and both handles.
const FIXED: u64 = mem::size_of::<(Shared, Owner, Communication, Control)>() as u64;

// SAFETY (all three): every field is an integer or a raw pointer, for which
// zero (null) is a valid value.
const NO_ERROR: native::Error = unsafe { mem::zeroed() };
const NO_DRAIN: native::Drained = unsafe { mem::zeroed() };
const NO_METRICS: native::Metrics = unsafe { mem::zeroed() };

const NO_DESCRIPTOR: Failure = Failure::new(
    RESOURCE_LIMIT,
    "the notification descriptor cannot be created",
);
const SUBMISSIONS_CLOSED: Failure = Failure::new(
    CLOSED,
    "the instance accepts no more requests: its stream is ending or ended",
);
const NOT_INCREASING: Failure = Failure::new(
    INVALID_REQUEST_ID,
    "request_id must exceed every accepted request's",
);
const QUEUE_FULL: Failure = Failure::new(
    FULL,
    "the request does not fit the request queue, the operations or the outcome reserve",
);
const DELIVERIES_FULL: Failure = Failure::new(
    FULL,
    "every delivery the limits allow is outstanding; release one first",
);

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

#[derive(Default)]
struct Measures {
    allocation: Gauge<u64>,
    request_bytes: Gauge<u64>,
    request_records: Gauge<u32>,
    /// Accepted requests whose outcome is not queued yet; each reserves one
    /// outcome in the event allowance.
    operations: Gauge<u32>,
    event_bytes: Gauge<u64>,
    event_records: Gauge<u32>,
    deliveries: Gauge<u32>,
    workers: Gauge<u32>,
    descriptors: Gauge<u32>,
    drained: u64,
    released: u64,
}

/// Why the stream ends, once it does.
struct Stop {
    shutdown: bool,
    /// The first failure, and the retry it calls for.
    failure: Option<Failed>,
    departure: Departure,
    /// The connection is closed.
    ended: bool,
}

impl Default for Stop {
    fn default() -> Self {
        Self {
            shutdown: false,
            failure: None,
            departure: Departure::NOT_REQUIRED,
            ended: false,
        }
    }
}

impl Stop {
    fn running(&self) -> bool {
        !self.shutdown && self.failure.is_none() && !self.ended
    }
}

#[derive(Default)]
struct State {
    stop: Stop,
    /// Accepted requests not taken for execution, oldest first.
    requests: VecDeque<Submitted>,
    /// The high-water mark of accepted public request IDs.
    last_request: u64,
    /// Records not drained yet, oldest first.
    events: VecDeque<Event>,
    /// Whether the notification descriptor is currently raised.
    readable: bool,
    communication_released: bool,
    control_released: bool,
    measures: Measures,
}

/// Terminal evidence and the fault its error points into, unchanged until
/// destroy.
struct Evidence {
    terminal: native::Terminal,
    _fault: Option<Fault>,
}

// SAFETY: the evidence is immutable, and its pointers reach only static text
// or the fault it owns.
unsafe impl Send for Evidence {}
// SAFETY: as above.
unsafe impl Sync for Evidence {}

/// What the connection does next.
pub(super) enum Next {
    /// A failure elsewhere ended the stream.
    Stop,
    /// Shutdown was requested: queued requests are already cancelled.
    Shutdown,
    Request(Submitted),
    Wait,
}

pub(super) struct Shared {
    /// The session's GUID in canonical text.
    session: String,
    generation: u64,
    limits: native::Limits,
    paths: Paths,
    settings: ClientSettings,
    notifier: Notifier,
    /// Raised whenever the connection has something new to look at.
    waker: Notifier,
    dispatch: Dispatch,
    /// Names this instance in every record: its generation.
    span: Span,
    terminal: OnceLock<Evidence>,
    state: Mutex<State>,
    changed: Condvar,
}

/// Behind `cm1_instance`: used only by the thread that created it.
pub(super) struct Owner {
    pub(super) shared: Arc<Shared>,
    thread: ThreadId,
    /// `run`'s worker-cleanup result, once it has returned.
    cleanup: Option<u32>,
    workers: Vec<JoinHandle<()>>,
}

/// Behind `cm1_communication`.
pub(super) struct Communication {
    pub(super) shared: Arc<Shared>,
}

/// Behind `cm1_control`.
pub(super) struct Control {
    pub(super) shared: Arc<Shared>,
}

/// Behind `cm1_delivery`: one immutable batch, released through this library.
pub(super) struct Delivery {
    pub(super) shared: Arc<Shared>,
    batch: Batch,
}

pub(super) type Handles = (Box<Owner>, Box<Communication>, Box<Control>);

/// Local setup only: the notification descriptor and the waker, then the
/// optional log file, the last step that can fail, so a failure leaves
/// nothing behind.
pub(super) fn create(startup: Startup) -> Result<Handles, Failure> {
    let notifier = Notifier::new().map_err(|_| NO_DESCRIPTOR)?;
    let waker = Notifier::waker().map_err(|_| NO_DESCRIPTOR)?;
    let (dispatch, logs) = diagnostics(&startup.diagnostics)?;
    let generation = startup.generation;
    let span = tracing::dispatcher::with_default(&dispatch, || {
        tracing::info_span!("client_instance", generation)
    });
    let allocation = FIXED + startup.paths.bytes();
    let shared = Arc::new(Shared {
        session: startup.session,
        generation,
        limits: startup.limits,
        paths: startup.paths,
        settings: startup.settings,
        notifier,
        waker,
        dispatch,
        span,
        terminal: OnceLock::new(),
        state: Mutex::new(State::default()),
        changed: Condvar::new(),
    });
    let mut state = shared.lock();
    state.measures.allocation.add(allocation);
    let descriptors = Notifier::DESCRIPTORS + Notifier::DESCRIPTORS + logs;
    state.measures.descriptors.add(descriptors);
    drop(state);
    shared.scope(|| tracing::info!("client instance created"));
    let owner = Owner {
        shared: Arc::clone(&shared),
        thread: thread::current().id(),
        cleanup: None,
        workers: Vec::new(),
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
fn diagnostics(diagnostics: &Diagnostics) -> Result<(Dispatch, u32), Failure> {
    let Some(path) = &diagnostics.log else {
        return Ok((Dispatch::none(), 0));
    };
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|_| Failure::new(INVALID_ARGUMENT, "log_path cannot be created as a new file"))?;
    let dispatch = subscriber(CappedWriter::new(file), Some(diagnostics.filter))
        .expect("startup validated the filter");
    Ok((dispatch, 1))
}

/// This generation's terminal evidence. The connection is always closed by
/// now; a failure or an unconfirmed departure makes the outcome failed.
fn evidence(generation: u64, stop: &Stop) -> Evidence {
    let failure = stop.failure.as_ref();
    let fault = failure
        .map(|failed| &failed.fault)
        .or(stop.departure.fault())
        .cloned();
    let client_outcome = if fault.is_some() {
        native::CLIENT_FAILED
    } else {
        native::CLIENT_STOPPED
    };
    let terminal = native::Terminal {
        generation,
        client_outcome,
        retry: failure.map_or(native::RETRY_NONE, |failed| failed.retry),
        leave: stop.departure.leave(),
        attachment_detached: u8::from(stop.departure.detached()),
        connection_closed: u8::from(true),
        reserved: [0; 2],
        error: fault.as_ref().map_or(NO_ERROR, Fault::error),
    };
    Evidence {
        terminal,
        _fault: fault,
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn scope<T>(&self, work: impl FnOnce() -> T) -> T {
        tracing::dispatcher::with_default(&self.dispatch, || self.span.in_scope(work))
    }

    pub(super) fn session(&self) -> &str {
        &self.session
    }

    pub(super) fn generation(&self) -> u64 {
        self.generation
    }

    pub(super) fn limits(&self) -> &native::Limits {
        &self.limits
    }

    pub(super) fn metadata_path(&self) -> &Path {
        &self.paths.metadata
    }

    pub(super) fn waker(&self) -> i32 {
        self.waker.descriptor()
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
                self.fail(Failed::passive(PANIC.into()));
                Err(PANIC)
            })
        })
    }

    /// The first failure ends the stream, unless it has already ended. A
    /// failure after shutdown was requested calls for no retry.
    pub(super) fn fail(&self, failed: Failed) {
        let mut state = self.lock();
        if state.stop.failure.is_none() && self.terminal.get().is_none() {
            tracing::warn!(code = failed.fault.code(), "client instance failed");
            let retry = if state.stop.shutdown {
                native::RETRY_NONE
            } else {
                failed.retry
            };
            self.diagnose(&mut state, failed.fault.clone());
            state.stop.failure = Some(Failed::new(failed.fault, retry));
            self.changed.notify_all();
            self.waker.raise();
        }
    }

    /// Queues a failure's diagnostic if it fits the event allowance: it is
    /// best effort, and the terminal evidence keeps the error regardless.
    fn diagnose(&self, state: &mut State, fault: Fault) {
        let event = Event::Diagnostic(fault);
        let reserved = u64::from(state.measures.operations.current);
        if self.events_fit(state, reserved, 1, event.charge()) {
            self.queue(state, event);
        }
    }

    /// Raises the notification descriptor while a record or the terminal
    /// evidence is ready to drain, and clears it otherwise.
    fn signal(&self, state: &mut State) {
        let ready = !state.events.is_empty() || self.terminal.get().is_some();
        if ready != state.readable {
            state.readable = ready;
            self.notifier.set(ready);
        }
    }

    /// Whether shutdown has been requested.
    pub(super) fn shutting_down(&self) -> bool {
        self.lock().stop.shutdown
    }

    pub(super) fn shutdown(&self) -> Result<(), Failure> {
        self.lock().stop.shutdown = true;
        self.changed.notify_all();
        self.waker.raise();
        tracing::debug!("client instance shutdown requested");
        Ok(())
    }

    /// Validates and copies a request, then queues it if the instance still
    /// accepts requests, its ID increases and it fits.
    pub(super) fn submit(&self, request: &native::Request) -> Result<(), Failure> {
        // SAFETY: the caller's buffers cover their lengths during this call.
        let submitted = unsafe { request::parse(request, self.limits.request_bytes) }?;
        let mut state = self.lock();
        self.admit(&state, &submitted)?;
        self.accept(&mut state, submitted);
        Ok(())
    }

    fn admit(&self, state: &State, submitted: &Submitted) -> Result<(), Failure> {
        require(state.stop.running(), SUBMISSIONS_CLOSED)?;
        require(submitted.id > state.last_request, NOT_INCREASING)?;
        require(self.room(state, submitted.bytes), QUEUE_FULL)
    }

    /// Whether one more request of `bytes` fits the request queue, the
    /// outstanding operations and, reserved for its outcome, the event
    /// allowance.
    fn room(&self, state: &State, bytes: u64) -> bool {
        let measures = &state.measures;
        let limits = &self.limits;
        let operations = u64::from(measures.operations.current) + 1;
        let fits = [
            (
                u64::from(measures.request_records.current) + 1,
                u64::from(limits.request_records),
            ),
            (
                measures.request_bytes.current.saturating_add(bytes),
                limits.request_bytes,
            ),
            (operations, u64::from(limits.operations)),
        ];
        fits.into_iter().all(|(need, limit)| need <= limit)
            && self.events_fit(state, operations, 0, 0)
    }

    /// What each outstanding operation reserves for its outcome: a record
    /// and the largest Response the daemon may send.
    fn reserve(&self) -> u64 {
        RECORD + u64::from(self.limits.metadata_record_bytes)
    }

    /// Whether the event allowance holds every queued and delivered record,
    /// the outcomes `operations` reserve, and `records` more records of
    /// `bytes`.
    fn events_fit(&self, state: &State, operations: u64, records: u64, bytes: u64) -> bool {
        let measures = &state.measures;
        let reserved = operations.saturating_mul(self.reserve());
        let fits = [
            (
                u64::from(measures.event_records.current) + operations + records,
                u64::from(self.limits.event_records),
            ),
            (
                measures
                    .event_bytes
                    .current
                    .saturating_add(reserved)
                    .saturating_add(bytes),
                self.limits.event_bytes,
            ),
        ];
        fits.into_iter().all(|(need, limit)| need <= limit)
    }

    fn accept(&self, state: &mut State, submitted: Submitted) {
        state.last_request = submitted.id;
        let measures = &mut state.measures;
        measures.request_records.add(1);
        measures.request_bytes.add(submitted.bytes);
        measures.allocation.add(submitted.bytes);
        measures.operations.add(1);
        state.requests.push_back(submitted);
        self.waker.raise();
    }

    /// What the connection does next: stop after a failure elsewhere, react
    /// to shutdown once, or take the oldest request when it can execute.
    pub(super) fn next(&self, shutdown_seen: bool, leave_ready: bool) -> Next {
        let mut state = self.lock();
        self.waker.clear();
        if state.stop.failure.is_some() {
            return Next::Stop;
        }
        if state.stop.shutdown && !shutdown_seen {
            self.cancel(&mut state);
            return Next::Shutdown;
        }
        take(&mut state, leave_ready).map_or(Next::Wait, Next::Request)
    }

    /// Rejects every queued request: none of them will execute.
    fn cancel(&self, state: &mut State) {
        while let Some(request) = dequeue(state) {
            self.respond(state, Response::rejected(request.id, CANCELLED));
        }
    }

    /// Queues an outcome into the room its request reserved.
    pub(super) fn answer(&self, response: Response) {
        let mut state = self.lock();
        self.respond(&mut state, response);
    }

    fn respond(&self, state: &mut State, response: Response) {
        state.measures.operations.sub(1);
        self.queue(state, Event::Response(response));
    }

    /// Queues a daemon record if the event allowance has room for it, or
    /// gives it back to wait.
    pub(super) fn offer(&self, event: Event) -> Option<Event> {
        let mut state = self.lock();
        let reserved = u64::from(state.measures.operations.current);
        if !self.events_fit(&state, reserved, 1, event.charge()) {
            return Some(event);
        }
        self.queue(&mut state, event);
        None
    }

    fn queue(&self, state: &mut State, event: Event) {
        let charge = event.charge();
        let measures = &mut state.measures;
        measures.event_records.add(1);
        measures.event_bytes.add(charge);
        measures.allocation.add(charge);
        state.events.push_back(event);
        self.signal(state);
    }

    pub(super) fn connected(&self) {
        self.lock().measures.descriptors.add(1);
    }

    pub(super) fn disconnected(&self) {
        self.lock().measures.descriptors.sub(1);
    }

    /// The connection is closed: its departure evidence, and the public
    /// requests it sent that will never be answered.
    fn end(&self, departure: Departure, unanswered: u32) {
        let mut state = self.lock();
        state.stop.departure = departure;
        state.stop.ended = true;
        state.measures.operations.sub(unanswered);
        self.changed.notify_all();
    }

    /// Waits until the stream stops running: only a hooked worker waits.
    #[cfg(feature = "test-hooks")]
    fn wait_for_stop(&self) {
        let state = self.lock();
        drop(
            self.changed
                .wait_while(state, |state| state.stop.running())
                .unwrap_or_else(PoisonError::into_inner),
        );
    }

    /// Publishes terminal evidence, after every record already queued; a
    /// request still queued is rejected first.
    fn finish(&self) -> native::Terminal {
        let mut state = self.lock();
        self.cancel(&mut state);
        let evidence = self
            .terminal
            .get_or_init(|| evidence(self.generation, &state.stop))
            .terminal;
        self.signal(&mut state);
        tracing::info!(
            outcome = evidence.client_outcome,
            retry = evidence.retry,
            leave = evidence.leave,
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
    pub(super) fn drain(self: &Arc<Self>) -> Result<native::Drained, Failure> {
        let mut state = self.lock();
        let drained = if state.events.is_empty() {
            Ok(self.ended())
        } else {
            self.deliver(&mut state)
        };
        self.signal(&mut state);
        drained
    }

    fn deliver(self: &Arc<Self>, state: &mut State) -> Result<native::Drained, Failure> {
        let outstanding = state.measures.deliveries.current;
        require(outstanding < self.limits.deliveries, DELIVERIES_FULL)?;
        let count = self.batch(&state.events);
        let batch = Batch::new(self.generation, state.events.drain(..count).collect());
        let measures = &mut state.measures;
        measures.deliveries.add(1);
        measures.allocation.add(DELIVERY);
        measures.drained += u64::from(batch.len());
        let delivery = Box::new(Delivery {
            shared: Arc::clone(self),
            batch,
        });
        let (records, count) = delivery.batch.records();
        Ok(native::Drained {
            kind: native::DELIVERY,
            delivery: Box::into_raw(delivery).cast(),
            records,
            count,
            ..NO_DRAIN
        })
    }

    /// How many of the oldest records one delivery takes: at least one, and
    /// more while both drain ceilings hold.
    fn batch(&self, events: &VecDeque<Event>) -> usize {
        let mut payload = 0;
        let fitting = events
            .iter()
            .take(self.limits.drain_records as usize)
            .take_while(|event| {
                payload += event.payload();
                payload <= self.limits.drain_payload_bytes
            })
            .count();
        fitting.max(1)
    }

    fn ended(&self) -> native::Drained {
        self.terminal.get().map_or(
            native::Drained {
                kind: native::EMPTY,
                ..NO_DRAIN
            },
            |evidence| native::Drained {
                kind: native::TERMINAL,
                terminal: &raw const evidence.terminal,
                ..NO_DRAIN
            },
        )
    }

    /// The stream has ended for its consumer: terminal evidence is published
    /// and every earlier record drained.
    fn ended_for_consumer(&self, state: &State) -> bool {
        self.terminal.get().is_some() && state.events.is_empty()
    }

    pub(super) fn release_communication(&self) -> Result<(), Failure> {
        let mut state = self.lock();
        (self.ended_for_consumer(&state) && state.measures.deliveries.current == 0)
            .then_some(())
            .ok_or(Failure::new(
                BUSY,
                "the stream has not ended, or deliveries are still outstanding",
            ))?;
        state.communication_released = true;
        Ok(())
    }

    pub(super) fn release_control(&self) -> Result<(), Failure> {
        let mut state = self.lock();
        self.ended_for_consumer(&state)
            .then_some(())
            .ok_or(Failure::new(BUSY, "the stream has not ended"))?;
        state.control_released = true;
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
            request_bytes: measures.request_bytes.current,
            peak_request_bytes: measures.request_bytes.peak,
            event_bytes: measures.event_bytes.current,
            peak_event_bytes: measures.event_bytes.peak,
            records_drained: measures.drained,
            deliveries_released: measures.released,
            request_records: measures.request_records.current,
            peak_request_records: measures.request_records.peak,
            event_records: measures.event_records.current,
            peak_event_records: measures.event_records.peak,
            deliveries: measures.deliveries.current,
            peak_deliveries: measures.deliveries.peak,
            operations: measures.operations.current,
            peak_operations: measures.operations.peak,
            workers: measures.workers.current,
            peak_workers: measures.workers.peak,
            descriptors: measures.descriptors.current,
            peak_descriptors: measures.descriptors.peak,
            ..NO_METRICS
        }
    }
}

/// The oldest queued request, if it can execute now: an Open always (it
/// runs, or is refused), a Leave once `leave_ready`.
fn take(state: &mut State, leave_ready: bool) -> Option<Submitted> {
    let ready = state
        .requests
        .front()
        .is_some_and(|request| leave_ready || matches!(request.operation, Operation::Open(_)));
    ready.then(|| dequeue(state)).flatten()
}

fn dequeue(state: &mut State) -> Option<Submitted> {
    let request = state.requests.pop_front()?;
    let measures = &mut state.measures;
    measures.request_records.sub(1);
    measures.request_bytes.sub(request.bytes);
    measures.allocation.sub(request.bytes);
    Some(request)
}

/// Releases one delivery through its own library; its records end here.
pub(super) fn release(delivery: Box<Delivery>) {
    let shared = Arc::clone(&delivery.shared);
    let (records, bytes) = (delivery.batch.len(), delivery.batch.charge());
    drop(delivery);
    let mut state = shared.lock();
    let measures = &mut state.measures;
    measures.deliveries.sub(1);
    measures.event_records.sub(records);
    measures.event_bytes.sub(bytes);
    measures.allocation.sub(DELIVERY + bytes);
    measures.released += 1;
    shared.waker.raise();
}

impl Owner {
    fn owned(&self) -> Result<(), Failure> {
        (thread::current().id() == self.thread)
            .then_some(())
            .ok_or(Failure::new(
                WRONG_THREAD,
                "only the instance's owner thread may call this",
            ))
    }

    /// Runs the connection until its stream ends, publishes terminal
    /// evidence, then waits up to `cleanup_ms` for its workers. The report
    /// is separate from the call's result: a failed run still reports what
    /// it knows.
    pub(super) fn run(&mut self) -> Result<(Result<(), Failure>, native::RunReport), Failure> {
        self.owned()?;
        self.cleanup
            .is_none()
            .then_some(())
            .ok_or(Failure::new(CLOSED, "run has already returned"))?;
        let shared = Arc::clone(&self.shared);
        let executed = shared.guard("run", || {
            self.execute();
            Ok(())
        });
        let terminal = shared.scope(|| shared.finish());
        let (cleanup, cleaned) = shared.scope(|| self.clean_up());
        self.cleanup = Some(cleanup);
        let report = native::RunReport {
            workers_cleanup: cleanup,
            reserved: 0,
            terminal,
        };
        Ok((executed.and(cleaned), report))
    }

    fn execute(&mut self) {
        let shared = &self.shared;
        let settings = shared.settings;
        tracing::debug!(
            metadata = ?shared.paths.metadata,
            home = ?shared.paths.home,
            runtime = ?shared.paths.runtime,
            escape = ?settings.escape_prefix().map(ControlKey::byte),
            take_control = ?settings.take_control_key().map(ControlKey::byte),
            "client instance running"
        );
        self.start_workers();
        let (departure, unanswered) = connection::run(&self.shared);
        self.shared.end(departure, unanswered);
    }

    /// Waits up to `cleanup_ms` for every worker, then joins them; a worker
    /// still running is reported, never joined or abandoned as complete.
    fn clean_up(&mut self) -> (u32, Result<(), Failure>) {
        let deadline = Duration::from_millis(self.shared.limits.cleanup_ms.into());
        if !self.shared.wait_for_workers(deadline) {
            tracing::warn!("client instance worker cleanup incomplete");
            return (
                native::CLEANUP_INCOMPLETE,
                Err(Failure::new(TIMEOUT, "worker cleanup exceeded cleanup_ms")),
            );
        }
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
        tracing::info!("client instance worker cleanup complete");
        (native::CLEANUP_COMPLETE, Ok(()))
    }

    /// Busy until `run` has returned with complete worker cleanup and both
    /// handles are released; deliveries are released before communication.
    pub(super) fn destroy(&self) -> Result<(), Failure> {
        self.owned()?;
        self.finished().then_some(()).ok_or(Failure::new(
            BUSY,
            "run, workers, handles or deliveries are still outstanding",
        ))?;
        tracing::debug!("client instance destroyed");
        Ok(())
    }

    /// Nothing of the instance remains in use.
    fn finished(&self) -> bool {
        let state = self.shared.lock();
        self.cleanup == Some(native::CLEANUP_COMPLETE)
            && state.communication_released
            && state.control_released
    }

    /// Without the `test-hooks` faults, the connection needs no worker: it
    /// runs on the owner thread.
    #[cfg(not(feature = "test-hooks"))]
    fn start_workers(&mut self) {}

    /// `worker` and `stall-cleanup` start one worker, whose stack is the
    /// release's qualified worker stack.
    #[cfg(feature = "test-hooks")]
    fn start_workers(&mut self) {
        if let Some(stalls) = super::hooks::worker() {
            let shared = Arc::clone(&self.shared);
            shared.lock().measures.workers.add(1);
            let worker = thread::Builder::new()
                .name("climon2-client-worker".into())
                .stack_size(REQUIREMENTS.worker_stack_bytes as usize)
                .spawn(move || work(&shared, stalls))
                .expect("the worker starts");
            self.workers.push(worker);
        }
    }
}

/// A worker that ends with its instance's stream, or whose cleanup stalls.
#[cfg(feature = "test-hooks")]
fn work(shared: &Shared, stalls: bool) {
    shared.wait_for_stop();
    if stalls {
        loop {
            thread::park();
        }
    }
    shared.lock().measures.workers.sub(1);
    shared.changed.notify_all();
}
