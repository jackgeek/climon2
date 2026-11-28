//! The client-instance interface, and the local instance: a loaded client
//! library's caller handles, the one place its native calls are made.
use crate::request::{Authority, Paste, Request};
use climon2_core::{
    SessionId,
    native::{self, Bytes, Call, Communication, Control, Drained},
};
use climon2_platform::library::Functions;
use std::mem::MaybeUninit;

/// Monitor Open's request ID.
pub const OPEN_ID: u64 = 1;
/// A Kill-only instance's Kill, its only request (#117): never mistaken for
/// Open's answer.
pub const KILL_ID: u64 = OPEN_ID + 1;

/// One client instance for one session, as its consumer drives it once its
/// handles are published: requests in, deliveries out, then its release.
/// A local instance ([`Local`]) is one adapter; a remote instance, a
/// session stream, is the monitor library's (ADR 0011).
pub trait Instance {
    /// What the consumer reads from each delivered record.
    type Event;

    /// The borrowed notification descriptor: readable while there may be
    /// something to drain.
    fn notification(&self) -> Result<i32, String>;

    /// Asks the instance to stop; idempotent and nonblocking.
    fn shutdown(&self);

    /// Submits `request` as `request_id` under the opened generation.
    fn submit(&self, request_id: u64, request: &Request) -> Result<(), String>;

    /// Submits a Kill of the instance's session as `request_id`, requiring
    /// nothing beyond core support, as `c2 kill` does (#117).
    fn kill(&self, request_id: u64) -> Result<(), String>;

    /// Takes one delivery, reads it and releases it at once.
    fn drain(&self, id: &SessionId) -> Drain<Self::Event>;

    /// Releases the instance's handles; its stream must be over. Whether
    /// every one was released is evidence of its own (#118).
    fn release(self, id: &SessionId) -> Result<(), String>;
}

/// Reads one delivered record, which lives until its delivery's release:
/// what it tells the consumer, copied.
pub type Reader<E> = fn(&SessionId, &native::Record) -> Vec<E>;

/// A local instance's caller handles, published by its owner thread, with
/// the release's calls, the connection generation it was created with and
/// its consumer's reader.
pub struct Local<E> {
    pub(crate) functions: &'static Functions,
    pub(crate) communication: *mut Communication,
    pub(crate) control: *mut Control,
    pub(crate) generation: u64,
    pub(crate) read: Reader<E>,
}

impl<E> Clone for Local<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E> Copy for Local<E> {}

// SAFETY: both handles are documented thread-safe for the calls made on them.
unsafe impl<E> Send for Local<E> {}

/// Whether a client that is over is retried: its terminal evidence's
/// retry, or its consumer's own when the consumer stopped it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Retry {
    None,
    /// Recoverable: a fresh instance after a backoff.
    Passive,
    /// Rejected for good: a missing revision-1 interface or capability,
    /// or invalid configuration.
    Permanent,
}

impl Retry {
    /// The terminal evidence's retry class; anything but passive or
    /// permanent retries nothing.
    fn of(evidence: u32) -> Self {
        match evidence {
            native::RETRY_PASSIVE => Self::Passive,
            native::RETRY_PERMANENT => Self::Permanent,
            _ => Self::None,
        }
    }
}

/// What one drain found.
pub enum Drain<E> {
    /// A delivery, already released, and what its records report.
    Events(Vec<E>),
    /// Nothing more for now, or never again.
    Stop(Stop),
}

pub enum Stop {
    Empty,
    /// Terminal evidence, with its error message (empty when stopped),
    /// whether it is retried and what it proves of the departure (#118).
    Terminal(String, Retry, Terminal),
    /// The drain call failed.
    Failed(String),
}

/// How an instance's Leave ended, from its terminal evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Leave {
    /// No registration or attachment can remain.
    NotRequired,
    /// The daemon accepted the Leave.
    Confirmed,
    Rejected,
    Failed,
    /// Lost, late or answered against the wire's rules.
    Indeterminate,
}

impl Leave {
    /// The native Leave code's meaning; an unknown code proves nothing.
    pub fn of(code: u32) -> Self {
        const CODES: [(u32, Leave); 4] = [
            (native::LEAVE_NOT_REQUIRED, Leave::NotRequired),
            (native::LEAVE_CONFIRMED, Leave::Confirmed),
            (native::LEAVE_REJECTED, Leave::Rejected),
            (native::LEAVE_FAILED, Leave::Failed),
        ];
        let known = CODES.into_iter().find(|(known, _)| *known == code);
        known.map_or(Self::Indeterminate, |(_, leave)| leave)
    }
}

/// What an instance's terminal evidence proves of its departure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Terminal {
    /// The client outcome: failed with this error, or stopped.
    pub failed: Option<String>,
    pub leave: Leave,
    /// A confirmed Leave detached an attachment.
    pub detached: bool,
    /// Every connection the instance owned is closed.
    pub closed: bool,
}

impl<E> Instance for Local<E> {
    type Event = E;

    fn notification(&self) -> Result<i32, String> {
        // Output only: the library writes it on success.
        let mut descriptor = Default::default();
        // SAFETY: a live communication handle and an output integer.
        checked(unsafe { (self.functions.notification)(self.communication, &mut descriptor) })
            .map(|()| descriptor)
    }

    fn shutdown(&self) {
        // SAFETY: a live control handle.
        unsafe { (self.functions.shutdown)(self.control) };
    }

    fn submit(&self, request_id: u64, request: &Request) -> Result<(), String> {
        let request = native_request(request_id, request, self.generation);
        // SAFETY: a live communication handle and a complete request whose
        // input bytes outlive the call, which copies them.
        checked(unsafe { (self.functions.submit)(self.communication, &request) })
    }

    fn kill(&self, request_id: u64) -> Result<(), String> {
        let request = kill_request(request_id);
        // SAFETY: a live communication handle and a complete request.
        checked(unsafe { (self.functions.submit)(self.communication, &request) })
    }

    fn drain(&self, id: &SessionId) -> Drain<E> {
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
            native::TERMINAL => {
                // SAFETY: terminal evidence lives until destroy.
                let terminal = unsafe { *drained.terminal };
                let retry = Retry::of(terminal.retry);
                // SAFETY: its error's bytes live as long.
                let message = unsafe { text(terminal.error.message) };
                Drain::Stop(Stop::Terminal(message, retry, evidence(&terminal)))
            }
            _ => Drain::Stop(Stop::Empty),
        }
    }

    fn release(self, id: &SessionId) -> Result<(), String> {
        // SAFETY: nothing else uses these handles any more.
        let calls = unsafe {
            [
                (self.functions.release_communication)(self.communication),
                (self.functions.release_control)(self.control),
            ]
        };
        let released = calls.map(checked);
        for call in released.clone() {
            warn(id, "handle release failed", call);
        }
        released.into_iter().collect()
    }
}

impl<E> Local<E> {
    fn delivery(&self, id: &SessionId, drained: &Drained) -> Vec<E> {
        // SAFETY: a delivery's records live until its release.
        let records = unsafe { slice(drained.records, drained.count) };
        let events = records
            .iter()
            .flat_map(|record| (self.read)(id, record))
            .collect();
        // SAFETY: released once, through its own library, after reading.
        let call = unsafe { (self.functions.release_delivery)(drained.delivery) };
        warn(id, "delivery release failed", checked(call));
        events
    }
}

/// The native request for `request` in `generation`, borrowing its input
/// bytes.
fn native_request(request_id: u64, request: &Request, generation: u64) -> native::Request {
    let (tag, args) = match *request {
        Request::Acquire => (native::ACQUIRE, native::RequestArgs { generation }),
        Request::Ready(held)
        | Request::Acknowledge(held)
        | Request::Focus(held, _)
        | Request::Viewing(held, _) => authorized(request, held),
        Request::Input(held, ref bytes) | Request::Paste(held, Paste { ref bytes, .. }) => (
            native::INPUT,
            native::RequestArgs {
                input: native::Input {
                    authority: native_authority(held),
                    bytes: Bytes {
                        data: bytes.as_ptr(),
                        length: bytes.len() as u64,
                    },
                },
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

/// A Kill as `request_id`, with no required family.
fn kill_request(request_id: u64) -> native::Request {
    native::Request {
        tag: native::KILL,
        reserved: 0,
        request_id,
        args: native::RequestArgs {
            required: native::Tokens {
                data: std::ptr::null(),
                count: 0,
            },
        },
    }
}

/// Ready, Acknowledge, Focus and Set viewing carry their authority, and
/// Focus and Set viewing their value as a flag.
fn authorized(request: &Request, held: Authority) -> (u32, native::RequestArgs) {
    let authority = native_authority(held);
    match *request {
        Request::Focus(_, focused) => flag(native::FOCUS, authority, focused),
        Request::Viewing(_, viewing) => flag(native::VIEWING, authority, viewing),
        Request::Acknowledge(_) => (native::ACKNOWLEDGE, native::RequestArgs { authority }),
        _ => (native::READY, native::RequestArgs { authority }),
    }
}

/// A request that sets `value` under `authority`.
fn flag(tag: u32, authority: native::Authority, value: bool) -> (u32, native::RequestArgs) {
    let flag = native::Flag {
        authority,
        value: u8::from(value),
        reserved: [0; 7],
    };
    (tag, native::RequestArgs { flag })
}

fn native_authority(held: Authority) -> native::Authority {
    native::Authority {
        generation: held.generation,
        period: held.period,
        restoration_id: held.restoration,
    }
}

/// A call's success, or its code and message, copied before any other call.
pub(crate) fn checked(call: Call) -> Result<(), String> {
    if call.code == native::SUCCESS {
        return Ok(());
    }
    Err(error(&call.error))
}

/// What terminal evidence proves: its client outcome (failed with its
/// error), Leave, detachment and closure, each on its own.
fn evidence(terminal: &native::Terminal) -> Terminal {
    let failed = terminal.client_outcome != native::CLIENT_STOPPED;
    Terminal {
        failed: failed.then(|| error(&terminal.error)),
        leave: Leave::of(terminal.leave),
        detached: terminal.attachment_detached != 0,
        closed: terminal.connection_closed != 0,
    }
}

/// A native error as its consumer shows it: its code, then its message.
fn error(error: &native::Error) -> String {
    // SAFETY (both): counted library bytes valid until the next call.
    let (code, message) = unsafe { (text(error.code), text(error.message)) };
    format!("{code}: {message}")
}

/// What run's report says of its workers: cleaned up after a successful
/// run, or as the report says after a failed one (#118). Anything but
/// complete cleanup is unconfirmed.
pub(crate) fn workers(ran: &Result<(), String>, report: &native::RunReport) -> Result<(), String> {
    match (ran, report.workers_cleanup) {
        (Ok(()), _) | (Err(_), native::CLEANUP_COMPLETE) => Ok(()),
        (Err(_), native::CLEANUP_INCOMPLETE) => Err("worker cleanup incomplete".to_owned()),
        (Err(_), _) => Err("worker cleanup unknown".to_owned()),
    }
}

/// Create's refusal and whether it is retried: invalid arguments or
/// configuration, an unsupported revision or limit and a release mismatch
/// are permanent; anything else, such as exhausted resources, passive.
pub(crate) fn created(call: Call) -> Result<(), (String, Retry)> {
    const PERMANENT: [u32; 4] = [
        native::INVALID_ARGUMENT,
        native::UNSUPPORTED_REVISION,
        native::UNSUPPORTED_LIMIT,
        native::RELEASE_MISMATCH,
    ];
    let retry = [Retry::Passive, Retry::Permanent][usize::from(PERMANENT.contains(&call.code))];
    checked(call).map_err(|why| (why, retry))
}

/// Logs a failed call that changes nothing for the caller.
pub(crate) fn warn(id: &SessionId, what: &str, result: Result<(), String>) {
    if let Err(error) = result {
        tracing::warn!(session = %id, %error, "{what}");
    }
}

/// `count` items at `data`; none for a null pointer.
///
/// # Safety
/// `data` is null, or points at `count` initialized items that outlive the
/// slice.
pub unsafe fn slice<'a, T>(data: *const T, count: u64) -> &'a [T] {
    if data.is_null() {
        return &[];
    }
    // SAFETY: as the caller promises.
    unsafe { std::slice::from_raw_parts(data, count as usize) }
}

/// Library text, copied lossily: it is diagnostic or display text.
///
/// # Safety
/// `bytes` is empty with a null pointer, or counts initialized bytes that
/// are valid for this call.
pub unsafe fn text(bytes: Bytes) -> String {
    // SAFETY: as the caller promises.
    String::from_utf8_lossy(unsafe { slice(bytes.data, bytes.length) }).into_owned()
}

#[cfg(test)]
mod tests {
    //! The native requests, terminal evidence, run reports and retry
    //! classes, as the native contract defines them: expected values
    //! written from the header.
    use super::*;

    fn bytes(text: &'static [u8]) -> Bytes {
        Bytes {
            data: text.as_ptr(),
            length: text.len() as u64,
        }
    }

    /// DEL-01 (pure supplement, the header's `CM1_KILL`; #117): a Kill
    /// names its request and requires no family, as `c2 kill`'s does.
    #[test]
    fn a_kill_requires_no_family() {
        let request = kill_request(7);
        assert_eq!(
            (request.tag, request.request_id, request.reserved),
            (native::KILL, 7, 0)
        );
        // SAFETY: the tag names the active member.
        let required = unsafe { request.args.required };
        assert!(required.data.is_null());
        assert_eq!(required.count, 0);
    }

    /// FOC-01 and STA-08 (pure supplement, the header's `CM1_VIEWING` and
    /// `CM1_ACKNOWLEDGE`; #114): Set viewing goes as a flag under its
    /// authority, 1 for viewing and 0 for not, with zero reserved bytes;
    /// Acknowledge as its authority alone, as Ready does.
    #[test]
    fn viewing_and_acknowledge_become_their_native_requests() {
        let held = Authority {
            generation: 4,
            period: 5,
            restoration: 6,
        };
        let triple = |authority: native::Authority| {
            (
                authority.generation,
                authority.period,
                authority.restoration_id,
            )
        };
        for (viewing, value) in [(true, 1), (false, 0)] {
            let request = native_request(9, &Request::Viewing(held, viewing), 1);
            assert_eq!((request.tag, request.request_id), (native::VIEWING, 9));
            // SAFETY: the tag names the active member.
            let flag = unsafe { request.args.flag };
            assert_eq!(triple(flag.authority), (4, 5, 6));
            assert_eq!((flag.value, flag.reserved), (value, [0; 7]));
        }
        for (request, tag) in [
            (Request::Acknowledge(held), native::ACKNOWLEDGE),
            (Request::Ready(held), native::READY),
        ] {
            let request = native_request(11, &request, 1);
            assert_eq!((request.tag, request.request_id), (tag, 11));
            // SAFETY: the tag names the active member.
            assert_eq!(triple(unsafe { request.args.authority }), (4, 5, 6));
        }
    }

    /// FOC-01 and INP-03 (pure supplement, the header's `CM1_FOCUS` and
    /// `CM1_INPUT`): Focus goes as a flag under its authority, 1 for in and 0
    /// for out, with zero reserved bytes; a paste part goes as input of its
    /// bytes.
    #[test]
    fn focus_and_paste_parts_become_their_native_requests() {
        let held = Authority {
            generation: 4,
            period: 5,
            restoration: 6,
        };
        for (focused, value) in [(true, 1), (false, 0)] {
            let request = native_request(9, &Request::Focus(held, focused), 1);
            assert_eq!((request.tag, request.request_id), (native::FOCUS, 9));
            // SAFETY: the tag names the active member.
            let flag = unsafe { request.args.flag };
            let authority = flag.authority;
            assert_eq!(
                (
                    authority.generation,
                    authority.period,
                    authority.restoration_id
                ),
                (4, 5, 6)
            );
            assert_eq!((flag.value, flag.reserved), (value, [0; 7]));
        }
        let paste = Paste {
            bracketed: true,
            starts: true,
            bytes: b"pasted".to_vec(),
            ends: true,
        };
        let paste = Request::Paste(held, paste);
        let request = native_request(10, &paste, 1);
        assert_eq!(request.tag, native::INPUT);
        // SAFETY: the tag names the active member, whose bytes `paste` holds.
        let input = unsafe { request.args.input };
        assert_eq!(input.authority.restoration_id, 6);
        // SAFETY: as above.
        assert_eq!(
            unsafe { slice(input.bytes.data, input.bytes.length) },
            b"pasted"
        );
    }

    /// EXT-01 (pure supplement, #118): terminal evidence proves its client
    /// outcome (failed with its error), Leave, detachment and closure, each
    /// on its own and as the header defines them.
    #[test]
    fn terminal_evidence_proves_each_kind_of_departure_evidence_alone() {
        let terminal = |outcome, leave, detached, closed| native::Terminal {
            generation: 1,
            client_outcome: outcome,
            retry: native::RETRY_NONE,
            leave,
            attachment_detached: detached,
            connection_closed: closed,
            reserved: [0; 2],
            error: native::Error {
                origin: native::ORIGIN_DAEMON,
                reserved: 0,
                code: bytes(b"timeout"),
                message: bytes(b"no answer"),
            },
        };
        let stopped = terminal(native::CLIENT_STOPPED, native::LEAVE_CONFIRMED, 1, 1);
        assert_eq!(
            evidence(&stopped),
            Terminal {
                failed: None,
                leave: Leave::Confirmed,
                detached: true,
                closed: true,
            }
        );
        let failed = terminal(native::CLIENT_FAILED, native::LEAVE_INDETERMINATE, 0, 0);
        assert_eq!(
            evidence(&failed),
            Terminal {
                failed: Some("timeout: no answer".into()),
                leave: Leave::Indeterminate,
                detached: false,
                closed: false,
            }
        );
    }

    /// EXT-02 and LIB-06 (pure supplement, #118): a successful run cleaned
    /// up its workers; after a failed one only a report of complete
    /// cleanup confirms it, and incomplete or unknown cleanup never does.
    #[test]
    fn only_a_successful_run_or_a_complete_report_confirms_worker_cleanup() {
        let report = |workers_cleanup| native::RunReport {
            workers_cleanup,
            // SAFETY: an all-zero terminal is a valid plain value.
            ..unsafe { std::mem::zeroed() }
        };
        let failed = Err("internal_error: run failed".to_owned());
        let cases = [
            (Ok(()), 0, Ok(())),
            (failed.clone(), native::CLEANUP_COMPLETE, Ok(())),
            (
                failed.clone(),
                native::CLEANUP_INCOMPLETE,
                Err("worker cleanup incomplete"),
            ),
            (
                failed.clone(),
                native::CLEANUP_UNKNOWN,
                Err("worker cleanup unknown"),
            ),
            (failed, 0, Err("worker cleanup unknown")),
        ];
        for (ran, code, expected) in cases {
            let expected = expected.map_err(str::to_owned);
            assert_eq!(workers(&ran, &report(code)), expected, "{code}");
        }
    }

    /// REC-01 (pure supplement): create's refusal of invalid arguments or
    /// configuration, an unsupported revision or limit, or another release
    /// is permanent; any other, such as exhausted resources, passive; the
    /// terminal evidence's retry is read as it says, anything else as none.
    #[test]
    fn create_refusals_and_terminal_evidence_say_whether_to_retry() {
        let call = |code| Call {
            code,
            reserved: 0,
            error: native::Error {
                origin: native::ORIGIN_LIBRARY,
                reserved: 0,
                code: Bytes {
                    data: b"code".as_ptr(),
                    length: 4,
                },
                message: Bytes {
                    data: b"why".as_ptr(),
                    length: 3,
                },
            },
        };
        let permanent = [
            native::INVALID_ARGUMENT,
            native::UNSUPPORTED_REVISION,
            native::UNSUPPORTED_LIMIT,
            native::RELEASE_MISMATCH,
        ];
        for code in permanent {
            assert_eq!(
                created(call(code)),
                Err(("code: why".into(), Retry::Permanent))
            );
        }
        for code in [native::RESOURCE_LIMIT, native::INTERNAL_ERROR] {
            assert_eq!(
                created(call(code)),
                Err(("code: why".into(), Retry::Passive))
            );
        }
        assert_eq!(created(call(native::SUCCESS)), Ok(()));
        let evidence = [
            (native::RETRY_NONE, Retry::None),
            (native::RETRY_PASSIVE, Retry::Passive),
            (native::RETRY_PERMANENT, Retry::Permanent),
            (0, Retry::None),
        ];
        for (retry, expected) in evidence {
            assert_eq!(Retry::of(retry), expected);
        }
    }

    /// Native contract (pure supplement): each terminal evidence Leave code
    /// reads as its own Leave, and an unknown one as unconfirmed.
    #[test]
    fn leave_codes_read_as_their_leave() {
        let codes = [
            (native::LEAVE_NOT_REQUIRED, Leave::NotRequired),
            (native::LEAVE_CONFIRMED, Leave::Confirmed),
            (native::LEAVE_REJECTED, Leave::Rejected),
            (native::LEAVE_FAILED, Leave::Failed),
            (native::LEAVE_INDETERMINATE, Leave::Indeterminate),
            (0, Leave::Indeterminate),
            (6, Leave::Indeterminate),
        ];
        for (code, leave) in codes {
            assert_eq!(Leave::of(code), leave, "{code}");
        }
    }

    /// A call that succeeded says nothing; a failed one says its code and
    /// message, whatever its numeric code.
    #[test]
    fn a_failed_call_says_its_code_and_message() {
        let call = |code| Call {
            code,
            reserved: 0,
            error: native::Error {
                origin: native::ORIGIN_LIBRARY,
                reserved: 0,
                code: bytes(b"busy"),
                message: bytes(b"still in use"),
            },
        };
        assert_eq!(checked(call(native::SUCCESS)), Ok(()));
        for code in [native::BUSY, native::INTERNAL_ERROR, u32::MAX] {
            assert_eq!(checked(call(code)), Err("busy: still in use".into()));
        }
    }

    /// Counted library data is read as it is counted, a null pointer as
    /// nothing, and text that is not UTF-8 lossily.
    #[test]
    fn library_data_is_read_as_counted_and_null_as_nothing() {
        let items = [3_u32, 5, 8];
        // SAFETY (all): null, or counted data that outlives the read.
        unsafe {
            assert_eq!(slice(items.as_ptr(), 3), [3, 5, 8]);
            assert_eq!(slice(items.as_ptr(), 2), [3, 5]);
            assert!(slice::<u32>(std::ptr::null(), 7).is_empty());
            assert_eq!(text(bytes(b"hello")), "hello");
            assert_eq!(text(bytes(b"a\xffb")), "a\u{fffd}b");
            let null = Bytes {
                data: std::ptr::null(),
                length: 9,
            };
            assert_eq!(text(null), "");
        }
    }

    /// A failed call that changes nothing for the caller is logged with
    /// its session and why; a call that succeeded is not.
    #[test]
    fn a_failed_call_that_changes_nothing_is_logged_with_its_session() {
        let home = tempfile::tempdir().unwrap();
        let log = climon2_platform::logging::LogFile::monitor(home.path()).unwrap();
        let dispatch = log.dispatch(Some("info")).unwrap();
        let id: SessionId = "109ca11f-0000-4000-8000-000000000001".parse().unwrap();
        tracing::dispatcher::with_default(&dispatch, || {
            warn(&id, "delivery release failed", Ok(()));
            warn(
                &id,
                "handle release failed",
                Err("busy: still in use".into()),
            );
        });
        let written = std::fs::read_to_string(log.path()).unwrap();
        let warned: Vec<_> = written
            .lines()
            .filter(|line| line.contains("WARN"))
            .collect();
        assert_eq!(warned.len(), 1, "{written}");
        for expected in ["handle release failed", "busy: still in use", id.as_str()] {
            assert!(warned[0].contains(expected), "{written}");
        }
        assert!(!written.contains("delivery release failed"), "{written}");
    }
}
