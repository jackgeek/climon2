//! An instance's owner thread: it creates the instance, submits monitor
//! Open, publishes the handles, runs it, and destroys it only once the
//! supervisor has released every handle.
use super::{
    Context, Internal,
    client::{Client, OPEN_ID, checked, warn},
};
use climon2_core::{
    SessionId,
    metadata::SessionMetadata,
    native::{self, Bytes, Created, Request, RequestArgs},
    wire::Guid,
};
use climon2_platform::library::Functions;
use std::{
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::Path,
    ptr,
    sync::mpsc::{Receiver, Sender},
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
pub(super) const GENERATION: u64 = 1;

/// An instance's whole life on its owner thread. The supervisor hears
/// `Published` once the handles exist, `Stopped` once run returned (or
/// create failed), and `OwnerDone` last. A closed `released` channel means
/// the supervisor gave up at its shutdown deadline: the handles may still be
/// unreleased, so the instance is left to process exit, never destroyed.
pub(super) fn owner(
    functions: &'static Functions,
    startup: &StartupData,
    id: SessionId,
    internal: &Sender<Internal>,
    released: &Receiver<()>,
) {
    let mut created = MaybeUninit::<Created>::zeroed();
    let native = startup.native();
    // SAFETY: a complete startup whose buffers outlive the call.
    let result = checked(unsafe { (functions.create)(&native, created.as_mut_ptr()) });
    if let Err(error) = result {
        let _ = internal.send(Internal::Stopped(id.clone(), Err(error)));
        let _ = internal.send(Internal::OwnerDone(id));
        return;
    }
    // SAFETY: a successful create initialized the output.
    let created = unsafe { created.assume_init() };
    let client = Client {
        functions,
        communication: created.communication,
        control: created.control,
    };
    if let Err(error) = submit_open(&client) {
        tracing::warn!(session = %id, %error, "monitor open not accepted");
        // Run then ends with terminal evidence.
        client.shutdown();
    }
    let _ = internal.send(Internal::Published(id.clone(), client));
    let mut report = MaybeUninit::<native::RunReport>::zeroed();
    // SAFETY: the instance created on this thread and an output struct.
    let ran = checked(unsafe { (functions.run)(created.instance, report.as_mut_ptr()) });
    let _ = internal.send(Internal::Stopped(id.clone(), ran));
    if released.recv().is_ok() {
        // SAFETY: run returned and the supervisor released every handle.
        warn(
            &id,
            "client destroy failed",
            checked(unsafe { (functions.destroy)(created.instance) }),
        );
    }
    let _ = internal.send(Internal::OwnerDone(id));
}

/// Submits monitor Open (request 1): `daemon-wire-v1` and `monitor-v1`
/// required, the title, status, notification, progress and focus families
/// optional. Submit copies everything before it returns.
fn submit_open(client: &Client) -> Result<(), String> {
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

/// One instance's startup inputs, owned until create has copied them.
pub(super) struct StartupData {
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

impl StartupData {
    pub(super) fn new(context: &Context, metadata: &SessionMetadata) -> std::io::Result<Self> {
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
