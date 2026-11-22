//! The client executable's own client library (#100): the library in the
//! executable's own version folder, loaded through the shared validated
//! loader, and one instance of it, driven through the frozen C ABI only,
//! never a Rust call into the library.
//!
//! The instance lives on an owner thread of its own, which creates it, runs
//! it and, once the executable has released every handle, destroys it. The
//! executable submits, drains and polls from its main thread.

use climon2_core::{
    VersionName,
    diagnostics::default_filter,
    layout::{ExecutableLayout, VersionBinary},
    metadata::SessionMetadata,
    native::{self, Call, Communication, Control, Created, Drained as Out, Instance, RunReport},
};
use climon2_platform::{
    library::{Functions, Library, Loader},
    logging::library_log_name,
    waiting::{Interest, Watch},
};
use std::{
    io,
    mem::MaybeUninit,
    os::{fd::BorrowedFd, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    slice,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

/// The executable's fixed `cm1_limits` (spec #73, Client executable): the
/// operational-limits per-client defaults, except `restoration_total_ms` at
/// its maximum, so that the daemon's size-scaled `Ready` deadline keeps
/// governing large restorations. The executable never reads
/// `monitor.limits`.
pub(crate) const LIMITS: native::Limits = native::Limits {
    client_bytes: 67_108_864,
    request_bytes: 262_144,
    event_bytes: 1_048_576,
    progress_bytes: 65_536,
    drain_payload_bytes: 65_536,
    request_records: 64,
    event_records: 1_024,
    progress_records: 32,
    deliveries: 8,
    operations: 16,
    chunk_bytes: 65_536,
    drain_records: 64,
    wire_frame_bytes: 1_048_576,
    metadata_record_bytes: 16_384,
    title_bytes: 4_096,
    notification_title_bytes: 4_096,
    notification_body_bytes: 8_192,
    max_workers: 2,
    operation_ms: 5_000,
    kill_ms: 10_000,
    restoration_stall_ms: 5_000,
    restoration_total_ms: 3_600_000,
    delivery_stall_ms: 5_000,
    leave_ms: 5_000,
    cleanup_ms: 10_000,
    start_ms: 10_000,
    reserved: 0,
};

/// The only instance an executable creates is its first connection
/// generation.
pub(crate) const GENERATION: u64 = 1;

/// The library in this executable's own version folder, which must describe
/// that folder's release, and the release. `current`, the working directory
/// and search paths never choose it.
fn own_library() -> io::Result<(&'static Library, VersionName)> {
    let executable = std::env::current_exe().and_then(|path| path.canonicalize())?;
    let layout = ExecutableLayout::from_binary_path(&executable, VersionBinary::Client)
        .map_err(|error| io::Error::other(format!("cannot locate the client library: {error}")))?;
    let library = Loader::new(layout.home()).load(layout.version());
    Ok((library.map_err(io::Error::other)?, layout.version().clone()))
}

/// What the client tells its library about the session besides its
/// metadata.
pub(crate) struct Session<'a> {
    /// Where the metadata was read.
    pub(crate) metadata: &'a Path,
    /// The client's own session log, beside which the library writes its own.
    pub(crate) log: &'a Path,
    /// The client's `CLIMON2_LOG` filter, if one was given.
    pub(crate) filter: Option<&'a str>,
}

/// An instance of this release's library for the session `metadata`
/// describes, under `home`, logging into a new file of its own beside the
/// client's session log with the client's filter. The library never
/// interprets Input, so the instance gets no keys: the executable routes
/// them.
pub(crate) fn session_instance(
    metadata: &SessionMetadata,
    session: &Session,
    home: &Path,
) -> io::Result<Embedded> {
    let (library, release) = own_library()?;
    let socket = Path::new(metadata.socket_path.as_str());
    let log = library_log_name(SystemTime::now(), std::process::id())?;
    let startup = Startup {
        session: metadata.session_id.bytes(),
        release,
        metadata: session.metadata.to_owned(),
        home: home.to_owned(),
        runtime: socket.parent().unwrap_or(socket).to_owned(),
        settings: native::Settings::default(),
        log: session.log.with_file_name(log),
        filter: session.filter.unwrap_or(default_filter()).to_owned(),
    };
    Embedded::start(library, startup)
}

/// An instance's startup inputs, owned until `create` has copied them.
pub(crate) struct Startup {
    pub(crate) session: [u8; 16],
    /// The executable's own release, which the library must serve.
    pub(crate) release: VersionName,
    pub(crate) metadata: PathBuf,
    pub(crate) home: PathBuf,
    pub(crate) runtime: PathBuf,
    pub(crate) settings: native::Settings,
    /// The library's own log file, created new.
    pub(crate) log: PathBuf,
    /// A `CLIMON2_LOG` filter for that log.
    pub(crate) filter: String,
}

impl Startup {
    /// The native startup, borrowing these inputs.
    fn native(&self) -> native::Startup {
        native::Startup {
            session_id: native::Guid {
                bytes: self.session,
            },
            generation: GENERATION,
            expected_release: bytes(self.release.as_str().as_bytes()),
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
                filter: bytes(self.filter.as_bytes()),
            },
            limits: LIMITS,
        }
    }
}

fn bytes(bytes: &[u8]) -> native::Bytes {
    native::Bytes {
        data: bytes.as_ptr(),
        length: bytes.len() as u64,
    }
}

fn path(path: &Path) -> native::Path {
    native::Path {
        encoding: native::NATIVE_UNIX,
        reserved: 0,
        bytes: bytes(path.as_os_str().as_bytes()),
    }
}

/// A library error, copied before the next call can reuse its text.
pub(crate) struct NativeError {
    /// Who failed: the library, the daemon or the connection's bytes.
    pub(crate) origin: u32,
    pub(crate) code: String,
    pub(crate) message: String,
}

impl NativeError {
    /// # Safety
    ///
    /// `error`'s buffers are valid, as the library returned them.
    pub(crate) unsafe fn copy(error: &native::Error) -> Self {
        // SAFETY (both): guaranteed by the caller.
        Self {
            origin: error.origin,
            code: unsafe { text(error.code) },
            message: unsafe { text(error.message) },
        }
    }
}

/// # Safety
///
/// A nonnull `bytes` covers `length` readable bytes.
unsafe fn text(bytes: native::Bytes) -> String {
    if bytes.data.is_null() {
        return String::new();
    }
    // SAFETY: guaranteed by the caller.
    let bytes = unsafe { slice::from_raw_parts(bytes.data, bytes.length as usize) };
    String::from_utf8_lossy(bytes).into_owned()
}

/// A failed call's error, which is the library's own text.
fn check(call: Call) -> Result<(), NativeError> {
    // SAFETY: the library just returned this error.
    (call.code == native::SUCCESS)
        .then_some(())
        .ok_or_else(|| unsafe { NativeError::copy(&call.error) })
}

fn failure(error: NativeError) -> io::Error {
    io::Error::other(error.message)
}

/// What a submission came to.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Admission {
    Accepted,
    /// No room now: the same request may be offered again later.
    Full,
    /// The stream has ended: nothing more is accepted.
    Closed,
}

/// What one drain found.
pub(crate) enum Drained {
    Delivered,
    Empty,
    Ended(Ended),
}

/// The end of an instance's stream: whether a Leave detached its
/// attachment, and the failure that ended it, if one did.
pub(crate) struct Ended {
    pub(crate) detached: bool,
    pub(crate) failure: Option<NativeError>,
}

impl Ended {
    /// # Safety
    ///
    /// `terminal` is evidence the library published, with valid buffers.
    unsafe fn copy(terminal: &native::Terminal) -> Self {
        let failed = terminal.client_outcome == native::CLIENT_FAILED;
        Self {
            detached: terminal.attachment_detached == 1,
            // SAFETY: guaranteed by the caller.
            failure: failed.then(|| unsafe { NativeError::copy(&terminal.error) }),
        }
    }
}

/// The handles the owner thread hands the executable.
struct Handles {
    communication: *mut Communication,
    control: *mut Control,
}

// SAFETY: the communication and control handles are thread-safe (core-v1).
unsafe impl Send for Handles {}

/// Whether more can arrive from an instance's stream.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stream {
    Open,
    Ended,
}

/// One running instance of the executable's own library.
pub(crate) struct Embedded {
    functions: Functions,
    handles: Handles,
    notification: i32,
    released: Sender<()>,
    owner: JoinHandle<()>,
}

impl Embedded {
    /// Creates an instance on its owner thread, which then runs it. A
    /// refused create fails here, before any connection.
    pub(crate) fn start(library: &'static Library, startup: Startup) -> io::Result<Self> {
        let functions = *library.functions();
        let (created, handles) = mpsc::channel();
        let (released, release) = mpsc::channel();
        let owner = thread::Builder::new()
            .name("client-instance".into())
            .spawn(move || own(functions, &startup, &created, &release))?;
        let handles = received(&handles)?;
        let notification = notification(functions, &handles)?;
        Ok(Self {
            functions,
            handles,
            notification,
            released,
            owner,
        })
    }

    /// Offers a request: accepted, or refused for now because the queues
    /// are full, or for good because the stream has closed.
    pub(crate) fn submit(&self, request: &native::Request) -> io::Result<Admission> {
        // SAFETY: a live communication handle and a valid request.
        let call = unsafe { (self.functions.submit)(self.handles.communication, request) };
        match call.code {
            native::FULL => Ok(Admission::Full),
            native::CLOSED => Ok(Admission::Closed),
            _ => check(call).map(|()| Admission::Accepted).map_err(failure),
        }
    }

    /// The borrowed descriptor that is readable while there is something
    /// to drain.
    pub(crate) fn descriptor(&self) -> BorrowedFd<'_> {
        // SAFETY: the library keeps the notification descriptor open while
        // the communication handle lives, which is until `finish` takes self.
        unsafe { BorrowedFd::borrow_raw(self.notification) }
    }

    /// One turn: drains once, visiting a delivery's records before it is
    /// released, or, with nothing to drain, waits until the instance
    /// signals work or `timeout` passes. The stream is ended once
    /// nothing more will arrive.
    pub(crate) fn turn(
        &self,
        visit: &mut dyn FnMut(&native::Record),
        timeout: Duration,
    ) -> io::Result<Stream> {
        match self.drain_once(visit)? {
            Drained::Delivered => Ok(Stream::Open),
            Drained::Ended(_) => Ok(Stream::Ended),
            Drained::Empty => self.wait(timeout).map(|()| Stream::Open),
        }
    }

    /// Drains once without waiting: a delivery, whose records `visit` sees
    /// before it is released, nothing, or the end of the stream with its
    /// terminal evidence.
    pub(crate) fn drain_once(&self, visit: &mut dyn FnMut(&native::Record)) -> io::Result<Drained> {
        let out = self.drain()?;
        match out.kind {
            native::DELIVERY => self.deliver(&out, visit).map(|()| Drained::Delivered),
            // SAFETY: a terminal drain's evidence lives until destroy.
            native::TERMINAL => Ok(Drained::Ended(unsafe { Ended::copy(&*out.terminal) })),
            _ => Ok(Drained::Empty),
        }
    }

    fn drain(&self) -> io::Result<Out> {
        let mut out = MaybeUninit::<Out>::zeroed();
        // SAFETY: a live communication handle and output storage.
        check(unsafe { (self.functions.drain)(self.handles.communication, out.as_mut_ptr()) })
            .map_err(failure)?;
        // SAFETY: a successful drain filled the output.
        Ok(unsafe { out.assume_init() })
    }

    fn deliver(&self, out: &Out, visit: &mut dyn FnMut(&native::Record)) -> io::Result<()> {
        // SAFETY: a delivery's records stay valid until it is released.
        let records = unsafe { slice::from_raw_parts(out.records, out.count as usize) };
        records.iter().for_each(visit);
        // SAFETY: a delivery this instance drained, released once.
        check(unsafe { (self.functions.release_delivery)(out.delivery) }).map_err(failure)
    }

    /// Waits until the instance has something to drain or `timeout`
    /// passes.
    pub(crate) fn wait(&self, timeout: Duration) -> io::Result<()> {
        crate::polling::wait(
            &mut [Watch::new(self.descriptor(), Interest::Read)],
            timeout,
        )
    }

    /// After the stream's end: releases both handles, lets the owner thread
    /// destroy the instance, and joins it. The executable's outcome is known
    /// by then and its process is about to exit, so a cleanup failure
    /// changes nothing it reports.
    pub(crate) fn finish(self) {
        // SAFETY: the stream ended and every delivery was released; no
        // other use of either handle remains.
        unsafe {
            (self.functions.release_communication)(self.handles.communication);
            (self.functions.release_control)(self.handles.control);
        }
        let _ = self.released.send(());
        let _ = self.owner.join();
    }
}

/// The handles the owner thread created, or why it could not.
fn received(handles: &Receiver<Result<Handles, NativeError>>) -> io::Result<Handles> {
    handles.recv().map_err(io::Error::other)?.map_err(|error| {
        io::Error::other(format!(
            "cannot start the client library: {}",
            error.message
        ))
    })
}

/// The instance's borrowed notification descriptor.
fn notification(functions: Functions, handles: &Handles) -> io::Result<i32> {
    let mut descriptor = i32::default();
    // SAFETY: a live communication handle and an output descriptor.
    check(unsafe { (functions.notification)(handles.communication, &mut descriptor) })
        .map_err(failure)?;
    Ok(descriptor)
}

/// The owner thread: create, run, and destroy once the executable has
/// released its handles. An executable that ends without releasing them
/// never gets that far.
fn own(
    functions: Functions,
    startup: &Startup,
    created: &Sender<Result<Handles, NativeError>>,
    release: &Receiver<()>,
) {
    let Some(instance) = create(functions, startup, created) else {
        return;
    };
    run(functions, instance);
    if release.recv().is_ok() {
        // SAFETY: run returned and every handle and delivery is released.
        unsafe { (functions.destroy)(instance) };
    }
}

/// Creates the instance and hands the executable its handles, or the
/// refusal.
fn create(
    functions: Functions,
    startup: &Startup,
    created: &Sender<Result<Handles, NativeError>>,
) -> Option<*mut Instance> {
    let mut out = MaybeUninit::<Created>::zeroed();
    // SAFETY: a valid startup whose buffers outlive the call, and output
    // storage.
    let call = check(unsafe { (functions.create)(&startup.native(), out.as_mut_ptr()) });
    // SAFETY: a successful create filled the output.
    let result = call.map(|()| unsafe { out.assume_init() });
    let instance = result.as_ref().ok().map(|created| created.instance);
    let _ = created.send(result.map(|created| Handles {
        communication: created.communication,
        control: created.control,
    }));
    instance
}

fn run(functions: Functions, instance: *mut Instance) {
    let mut report = MaybeUninit::<RunReport>::zeroed();
    // SAFETY: the instance's owner thread, once, with output storage.
    unsafe { (functions.run)(instance, report.as_mut_ptr()) };
}
