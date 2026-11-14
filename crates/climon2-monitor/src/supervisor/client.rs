//! One instance's caller handles and what draining them reports: the only
//! place the supervisor reads the client library's records.
use crate::registry::ClientEvent;
use climon2_core::{
    SessionId,
    native::{self, Bytes, Call, Communication, Control, Drained},
};
use climon2_platform::library::Functions;
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
type Reader = fn(&SessionId, &native::Record) -> Option<ClientEvent>;

/// What the supervisor reads from each record kind; every other kind is
/// ignored until a later ticket needs it.
const READERS: [(u32, Reader); 4] = [
    (native::RESPONSE, response),
    // SAFETY (both titles): the tag names the active member.
    (native::INITIAL_METADATA, |_, record| {
        Some(title(unsafe { record.data.metadata.title }))
    }),
    (native::TITLE, |_, record| {
        Some(title(unsafe { record.data.title }))
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
        let events = records
            .iter()
            .filter_map(|record| read(id, record))
            .collect();
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

fn read(id: &SessionId, record: &native::Record) -> Option<ClientEvent> {
    let (_, reader) = READERS.iter().find(|(tag, _)| *tag == record.tag)?;
    reader(id, record)
}

/// Monitor Open's response: opened, with whether `status-v1` was selected,
/// or refused with why.
fn response(_: &SessionId, record: &native::Record) -> Option<ClientEvent> {
    // SAFETY: the tag names the active member.
    let response = unsafe { record.data.response };
    (response.request_id == OPEN_ID).then(|| opened(&response))
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

fn diagnostic(id: &SessionId, record: &native::Record) -> Option<ClientEvent> {
    // SAFETY: the tag names the active member.
    let error = unsafe { record.data.diagnostic };
    tracing::warn!(session = %id, code = %text(error.code), message = %text(error.message), "client diagnostic");
    None
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
