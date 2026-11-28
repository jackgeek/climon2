//! An instance's owner thread: it creates the instance, submits its first
//! request, publishes the handles, runs it, and destroys it only once its
//! consumer has released every handle.
use crate::instance::{self, Instance as _, KILL_ID, Local, OPEN_ID, Reader, Retry, checked, warn};
use climon2_core::{
    SessionId,
    keys::ControlKey,
    layout::HomeLayout,
    metadata::SessionMetadata,
    native::{self, Bytes, Created, Request, RequestArgs},
    wire::Guid,
};
use climon2_platform::{library::Functions, native_path};
use std::{
    io,
    mem::MaybeUninit,
    path::Path,
    ptr,
    sync::mpsc::{self, Receiver, Sender},
    thread::{self, JoinHandle},
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

/// What an instance is for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    /// A passive monitoring client: monitor Open, then the session's
    /// reports.
    Monitor,
    /// One Kill and nothing else (#117): it never opens, and it is never
    /// retried.
    KillOnly,
}

/// What an owner thread tells its consumer, in this order.
pub enum Signal<E> {
    /// The handles exist, with the first request's refusal, if refused.
    Published(Local<E>, Option<String>),
    /// Run returned, or create failed, with why if it failed and whether
    /// that is retried, and what its report says of its workers.
    Stopped(Result<(), (String, Retry)>, Result<(), String>),
    /// The owner is done, with whether it destroyed the instance.
    Done(Result<(), String>),
}

/// A running owner thread, as its consumer holds it. Dropped without
/// [`Owner::release`], it tells the thread that the handles may still be
/// in use: the instance is then left to process exit, never destroyed.
pub struct Owner {
    released: Sender<()>,
    thread: JoinHandle<()>,
}

impl Owner {
    /// Every handle is released: the owner may destroy its instance once
    /// run has returned.
    pub fn release(&self) {
        let _ = self.released.send(());
    }

    /// Waits for the thread, which has sent [`Signal::Done`], to end.
    pub fn join(self) {
        let _ = self.thread.join();
    }
}

/// Starts the owner thread of an instance in `role` for session `id`, on a
/// stack of `stack` bytes, telling `tell` each [`Signal`]; or why no
/// thread could start.
pub fn spawn<E: 'static>(
    functions: &'static Functions,
    stack: u64,
    startup: Startup,
    id: SessionId,
    role: Role,
    read: Reader<E>,
    tell: impl Fn(Signal<E>) + Send + 'static,
) -> io::Result<Owner> {
    let (released, release) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("client".into())
        .stack_size(crate::stack(stack))
        .spawn(move || owner(functions, &startup, &id, role, read, &tell, &release))?;
    Ok(Owner { released, thread })
}

/// An instance's whole life on its owner thread. A monitoring instance
/// first submits monitor Open, a Kill-only one its Kill (#117). Its
/// consumer hears `Published` once the handles exist, with that first
/// request's refusal, if it was refused, `Stopped` once run returned (or
/// create failed) with what its report says of its workers, and `Done`
/// last, with whether destroy succeeded (with nothing created, there was
/// nothing to destroy). A closed `released` channel means the consumer
/// gave up at its shutdown deadline: the handles may still be unreleased,
/// so the instance is left to process exit, never destroyed.
fn owner<E>(
    functions: &'static Functions,
    startup: &Startup,
    id: &SessionId,
    role: Role,
    read: Reader<E>,
    tell: &dyn Fn(Signal<E>),
    released: &Receiver<()>,
) {
    let mut created = MaybeUninit::<Created>::zeroed();
    let native = startup.native();
    // SAFETY: a complete startup whose buffers outlive the call.
    let result = instance::created(unsafe { (functions.create)(&native, created.as_mut_ptr()) });
    if let Err(refused) = result {
        tell(Signal::Stopped(Err(refused), Ok(())));
        tell(Signal::Done(Ok(())));
        return;
    }
    // SAFETY: a successful create initialized the output.
    let created = unsafe { created.assume_init() };
    let client = Local {
        functions,
        communication: created.communication,
        control: created.control,
        generation: startup.generation,
        read,
    };
    let first = match role {
        Role::Monitor => submit_open(&client),
        Role::KillOnly => client.kill(KILL_ID),
    };
    if let Err(error) = &first {
        tracing::warn!(session = %id, %error, ?role, "first request not accepted");
        // Run then ends with terminal evidence.
        client.shutdown();
    }
    tell(Signal::Published(client, first.err()));
    let mut report = MaybeUninit::<native::RunReport>::zeroed();
    // SAFETY: the instance created on this thread and an output struct.
    let ran = checked(unsafe { (functions.run)(created.instance, report.as_mut_ptr()) });
    // SAFETY: zeroed, then written by run if it reported: plain values.
    let report = unsafe { report.assume_init() };
    let workers = instance::workers(&ran, &report);
    let ran = ran.map_err(|why| (why, Retry::Passive));
    tell(Signal::Stopped(ran, workers));
    let destroyed = released
        .recv()
        .map_err(|_| "its handles may still be in use".to_owned())
        .and_then(|()| {
            // SAFETY: run returned and the consumer released every handle.
            let destroyed = checked(unsafe { (functions.destroy)(created.instance) });
            warn(id, "client destroy failed", destroyed.clone());
            destroyed
        });
    tell(Signal::Done(destroyed));
}

/// Submits monitor Open (request 1): `daemon-wire-v1` and `monitor-v1`
/// required, the title, status, notification, progress and focus families
/// optional. Submit copies everything before it returns.
fn submit_open<E>(client: &Local<E>) -> Result<(), String> {
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
    checked(unsafe { (client.functions.submit)(client.communication, &request) })
}

/// What every instance's startup shares, as its consumer configured it.
pub struct Environment<'a> {
    pub home: &'a HomeLayout,
    /// The runtime directory.
    pub runtime: &'a Path,
    /// The client escape prefix.
    pub escape: ControlKey,
    pub take_control: Option<ControlKey>,
    pub limits: native::Limits,
    /// The library log's filter.
    pub filter: &'a str,
}

/// One instance's startup inputs, owned until create has copied them.
pub struct Startup {
    session: [u8; 16],
    release: Vec<u8>,
    metadata: Vec<u8>,
    home: Vec<u8>,
    runtime: Vec<u8>,
    log: Vec<u8>,
    filter: Vec<u8>,
    settings: native::Settings,
    limits: native::Limits,
    /// This attempt's connection generation: fresh for every retry.
    generation: u64,
}

impl Startup {
    /// The startup of `metadata`'s session in connection `generation`,
    /// with its library log opened in `environment`'s home.
    pub fn new(
        environment: &Environment<'_>,
        metadata: &SessionMetadata,
        generation: u64,
    ) -> std::io::Result<Self> {
        let id = &metadata.session_id;
        let log = climon2_platform::logging::library_log(environment.home.root(), id, generation)?;
        let guid: Guid = id.as_str().parse().map_err(std::io::Error::other)?;
        let path = |path: &Path| native_path::encode(path);
        let (take_control_enabled, take_control_byte) = environment
            .take_control
            .map_or(Default::default(), |key| (1, key.byte()));
        Ok(Self {
            session: guid.bytes(),
            release: metadata.climon2_version.as_str().as_bytes().to_vec(),
            metadata: path(&environment.home.metadata(id)),
            home: path(environment.home.root()),
            runtime: path(environment.runtime),
            log: path(&log),
            filter: environment.filter.as_bytes().to_vec(),
            settings: native::Settings {
                escape_enabled: 1,
                escape_byte: environment.escape.byte(),
                take_control_enabled,
                take_control_byte,
                reserved: 0,
            },
            limits: environment.limits,
            generation,
        })
    }

    fn native(&self) -> native::Startup {
        let path = |data: &[u8]| native::Path {
            encoding: native_path::ENCODING,
            reserved: 0,
            bytes: bytes(data),
        };
        native::Startup {
            session_id: native::Guid {
                bytes: self.session,
            },
            generation: self.generation,
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
