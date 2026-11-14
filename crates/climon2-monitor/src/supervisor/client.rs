//! One instance's caller handles and what draining them reports: the only
//! place the supervisor reads the client library's records.
use super::owner::GENERATION;
use crate::{
    registry::ClientEvent,
    terminal::{Authority, Presentation, Request},
};
use climon2_core::{
    SessionId,
    native::{self, Bytes, Call, Communication, Control, Drained},
};
use climon2_platform::library::Functions;
use climon2_vt::Geometry;
use std::mem::MaybeUninit;

/// Monitor Open's request ID.
pub(super) const OPEN_ID: u64 = 1;
const STATUS: &str = "status-v1";

/// An instance's caller handles, published by its owner thread, with the
/// release's calls.
#[derive(Clone, Copy)]
pub(super) struct Client {
    pub(super) functions: &'static Functions,
    pub(super) communication: *mut Communication,
    pub(super) control: *mut Control,
}

// SAFETY: both handles are documented thread-safe for the calls made on them.
unsafe impl Send for Client {}

/// What one drain found.
pub(super) enum Drain {
    /// A delivery, already released, and what its records report.
    Events(Vec<ClientEvent>),
    /// Nothing more for now, or never again.
    Stop(Stop),
}

pub(super) enum Stop {
    Empty,
    /// Terminal evidence, with its error message (empty when stopped).
    Terminal(String),
    /// The drain call failed.
    Failed(String),
}

/// Reads one record kind; the tag names the active member.
type Reader = fn(&SessionId, &native::Record) -> Vec<ClientEvent>;

/// What the supervisor reads from each record kind; every other kind is
/// ignored until a later ticket needs it.
// SAFETY (every reader): the tag names the active member.
const READERS: [(u32, Reader); 11] = [
    (native::RESPONSE, |_, record| {
        vec![response(unsafe { &record.data.response })]
    }),
    (native::INITIAL_METADATA, |_, record| {
        initial(unsafe { &record.data.metadata })
    }),
    (native::TITLE, |_, record| {
        vec![title(unsafe { record.data.title })]
    }),
    (native::FINAL, |_, _| vec![ClientEvent::Final]),
    (native::GRANTED, |_, record| {
        let granted = unsafe { record.data.granted };
        let authority = authority(record, granted.period, granted.restoration_id);
        presented(geometry(granted.geometry).map(|at| Presentation::Granted(authority, at)))
    }),
    (native::REVOKED, |_, _| {
        presented(Some(Presentation::Revoked))
    }),
    (native::RESTORATION_BEGIN, |_, record| {
        let begun = unsafe { record.data.restoration };
        let authority = authority(record, begun.period, begun.restoration_id);
        presented(geometry(begun.geometry).map(|at| Presentation::Begin(authority, at)))
    }),
    (native::DISPLAY, |_, record| {
        let display = unsafe { record.data.display };
        let bytes = unsafe { slice(display.bytes.data, display.bytes.length) };
        presented(Some(Presentation::Display(bytes.to_vec())))
    }),
    (native::RESTORATION_COMPLETE, |_, _| {
        presented(Some(Presentation::Complete))
    }),
    (native::GEOMETRY, |_, record| {
        let changed = unsafe { record.data.geometry };
        presented(geometry(changed.geometry).map(Presentation::Geometry))
    }),
    (native::DIAGNOSTIC, diagnostic),
];

impl Client {
    /// The borrowed notification descriptor.
    pub(super) fn notification(&self) -> Result<i32, String> {
        let mut descriptor = -1;
        // SAFETY: a live communication handle and an output integer.
        checked(unsafe { (self.functions.notification)(self.communication, &mut descriptor) })
            .map(|()| descriptor)
    }

    /// Asks the instance to stop; idempotent and nonblocking.
    pub(super) fn shutdown(&self) {
        // SAFETY: a live control handle.
        unsafe { (self.functions.shutdown)(self.control) };
    }

    /// Submits `request` as `request_id` under the opened generation.
    pub(super) fn submit(&self, request_id: u64, request: Request) -> Result<(), String> {
        let request = native_request(request_id, request);
        // SAFETY: a live communication handle and a complete request.
        checked(unsafe { (self.functions.submit)(self.communication, &request) })
    }

    /// Takes one delivery, reads it and releases it at once.
    pub(super) fn drain(&self, id: &SessionId) -> Drain {
        let mut drained = MaybeUninit::<Drained>::zeroed();
        // SAFETY: a live communication handle and an output struct.
        let call = unsafe { (self.functions.drain)(self.communication, drained.as_mut_ptr()) };
        if let Err(error) = checked(call) {
            return Drain::Stop(Stop::Failed(error));
        }
        // SAFETY: a successful drain initialized the output.
        let drained = unsafe { drained.assume_init() };
        match drained.kind {
            native::DELIVERY => Drain::Events(self.delivery(id, &drained)),
            // SAFETY: terminal evidence lives until destroy.
            native::TERMINAL => Drain::Stop(Stop::Terminal(text(unsafe {
                (*drained.terminal).error.message
            }))),
            _ => Drain::Stop(Stop::Empty),
        }
    }

    fn delivery(&self, id: &SessionId, drained: &Drained) -> Vec<ClientEvent> {
        // SAFETY: a delivery's records live until its release.
        let records = unsafe { slice(drained.records, drained.count) };
        let events = records.iter().flat_map(|record| read(id, record)).collect();
        // SAFETY: released once, through its own library, after reading.
        let call = unsafe { (self.functions.release_delivery)(drained.delivery) };
        warn(id, "delivery release failed", checked(call));
        events
    }

    /// Releases both caller handles; the stream must be over.
    pub(super) fn release(self, id: &SessionId) {
        // SAFETY: nothing else uses these handles any more.
        let calls = unsafe {
            [
                (self.functions.release_communication)(self.communication),
                (self.functions.release_control)(self.control),
            ]
        };
        for call in calls {
            warn(id, "handle release failed", checked(call));
        }
    }
}

fn read(id: &SessionId, record: &native::Record) -> Vec<ClientEvent> {
    READERS
        .iter()
        .find(|(tag, _)| *tag == record.tag)
        .map_or_else(Vec::new, |(_, reader)| reader(id, record))
}

/// The native request for `request`.
fn native_request(request_id: u64, request: Request) -> native::Request {
    let (tag, args) = match request {
        Request::Acquire => (
            native::ACQUIRE,
            native::RequestArgs {
                generation: GENERATION,
            },
        ),
        Request::Ready(held) => (
            native::READY,
            native::RequestArgs {
                authority: native_authority(held),
            },
        ),
        Request::Resize(held, size) => (
            native::RESIZE,
            native::RequestArgs {
                resize: native::Resize {
                    authority: native_authority(held),
                    geometry: native::Geometry {
                        cols: size.cols.into(),
                        rows: size.rows.into(),
                    },
                },
            },
        ),
    };
    native::Request {
        tag,
        reserved: 0,
        request_id,
        args,
    }
}

fn native_authority(held: Authority) -> native::Authority {
    native::Authority {
        generation: held.generation,
        period: held.period,
        restoration_id: held.restoration,
    }
}

fn authority(record: &native::Record, period: u64, restoration: u64) -> Authority {
    Authority {
        generation: record.generation,
        period,
        restoration,
    }
}

/// An effective session geometry; the library reports only positive ones.
fn geometry(geometry: native::Geometry) -> Option<Geometry> {
    let narrow = |value: u32| u16::try_from(value).unwrap_or(u16::MAX);
    Geometry::for_host(narrow(geometry.cols), narrow(geometry.rows))
}

fn presented(presentation: Option<Presentation>) -> Vec<ClientEvent> {
    presentation
        .map(ClientEvent::Presentation)
        .into_iter()
        .collect()
}

/// The Initial metadata's title, and whether the command already ended.
fn initial(metadata: &native::Metadata) -> Vec<ClientEvent> {
    let finished = metadata.command_result.present != 0;
    std::iter::once(title(metadata.title))
        .chain(finished.then_some(ClientEvent::Final))
        .collect()
}

/// Monitor Open's response: opened, with whether `status-v1` was selected,
/// or refused with why. Any other request's answer is matched to its
/// request by the supervisor.
fn response(response: &native::Response) -> ClientEvent {
    if response.request_id == OPEN_ID {
        return opened(response);
    }
    let answer = (response.outcome == native::OK)
        .then_some(())
        .ok_or_else(|| text(response.error.message));
    ClientEvent::Answer(response.request_id, answer)
}

fn opened(response: &native::Response) -> ClientEvent {
    let succeeded = response.outcome == native::OK && response.result_tag == native::OPENED;
    if !succeeded {
        return ClientEvent::Unavailable(text(response.error.message));
    }
    // SAFETY: the result tag names the active member, and its tokens live
    // until the delivery's release.
    let selected = unsafe {
        slice(
            response.result.opened.selected.data,
            response.result.opened.selected.count,
        )
    };
    ClientEvent::Opened {
        status: selected.iter().any(|token| text(*token) == STATUS),
    }
}

fn title(title: native::Title) -> ClientEvent {
    ClientEvent::Title((title.state == native::VALUE).then(|| text(title.value)))
}

fn diagnostic(id: &SessionId, record: &native::Record) -> Vec<ClientEvent> {
    // SAFETY: the tag names the active member.
    let error = unsafe { record.data.diagnostic };
    tracing::warn!(session = %id, code = %text(error.code), message = %text(error.message), "client diagnostic");
    Vec::new()
}

/// A call's success, or its code and message, copied before any other call.
pub(super) fn checked(call: Call) -> Result<(), String> {
    if call.code == native::SUCCESS {
        return Ok(());
    }
    Err(format!(
        "{}: {}",
        text(call.error.code),
        text(call.error.message)
    ))
}

/// Logs a failed call that changes nothing for the caller.
pub(super) fn warn(id: &SessionId, what: &str, result: Result<(), String>) {
    if let Err(error) = result {
        tracing::warn!(session = %id, %error, "{what}");
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
pub(super) fn text(bytes: Bytes) -> String {
    // SAFETY: counted library bytes valid for this call.
    String::from_utf8_lossy(unsafe { slice(bytes.data, bytes.length) }).into_owned()
}
