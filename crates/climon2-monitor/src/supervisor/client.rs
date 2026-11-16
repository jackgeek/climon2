//! One instance's caller handles and what draining them reports: the only
//! place the supervisor reads the client library's records.
use super::owner::GENERATION;
use crate::{
    registry::{ClientEvent, Final, Outcome, Report, State, Status},
    terminal::{Authority, Paste, Presentation, Request},
};
use climon2_core::{
    SessionId,
    native::{self, Bytes, Call, Communication, Control, Drained},
};
use climon2_platform::library::Functions;
use climon2_vt::Geometry;
use std::{mem::MaybeUninit, time::SystemTime};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Monitor Open's request ID.
pub(super) const OPEN_ID: u64 = 1;
const STATUS: &str = "status-v1";
const FOCUS: &str = "focus-v1";

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
const READERS: [(u32, Reader); 12] = [
    (native::RESPONSE, |_, record| {
        response(unsafe { &record.data.response })
    }),
    (native::INITIAL_METADATA, |_, record| {
        initial(unsafe { &record.data.metadata })
    }),
    (native::TITLE, |_, record| {
        vec![title(unsafe { record.data.title })]
    }),
    (native::STATUS, |_, record| {
        status(unsafe { record.data.status })
            .map(|status| ClientEvent::Report(Report::Status(status)))
            .into_iter()
            .collect()
    }),
    (native::FINAL, |_, record| {
        let ended = unsafe { record.data.r#final };
        finished(ended.command_result, ended.status)
            .into_iter()
            .collect()
    }),
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
        // Output only: the library writes it on success.
        let mut descriptor = Default::default();
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
    pub(super) fn submit(&self, request_id: u64, request: &Request) -> Result<(), String> {
        let request = native_request(request_id, request);
        // SAFETY: a live communication handle and a complete request whose
        // input bytes outlive the call, which copies them.
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

/// The native request for `request`, borrowing its input bytes.
fn native_request(request_id: u64, request: &Request) -> native::Request {
    let (tag, args) = match *request {
        Request::Acquire => (
            native::ACQUIRE,
            native::RequestArgs {
                generation: GENERATION,
            },
        ),
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

/// The Initial metadata's title, then its final outcome when the command
/// already ended, otherwise its status when `status-v1` reported one. The
/// library has checked that a final status comes exactly with a result.
fn initial(metadata: &native::Metadata) -> Vec<ClientEvent> {
    let result = metadata.command_result;
    let standing = if result.present == 0 {
        available(metadata.status)
            .and_then(status)
            .map(|status| ClientEvent::Report(Report::Status(status)))
    } else {
        finished(result.value, metadata.status)
    };
    std::iter::once(title(metadata.title))
        .chain(standing)
        .collect()
}

/// A final outcome, with the terminal status's change time when
/// `status-v1` reported one.
fn finished(result: native::CommandResult, status: native::AvailableStatus) -> Option<ClientEvent> {
    let changed = available(status).and_then(|status| changed(status.changed_at));
    outcome(result).map(|outcome| ClientEvent::Report(Report::Final(Final { outcome, changed })))
}

fn available(status: native::AvailableStatus) -> Option<native::Status> {
    (status.available != 0).then_some(status.value)
}

/// A status that is not final; a final state comes only with a result.
fn status(status: native::Status) -> Option<Status> {
    let (_, state) = STATES.iter().find(|(code, _)| *code == status.state)?;
    let changed = changed(status.changed_at)?;
    Some(Status {
        state: *state,
        changed,
    })
}

const STATES: [(u32, State); 4] = [
    (native::NEEDS_ATTENTION, State::NeedsAttention),
    (native::ACKNOWLEDGED, State::Acknowledged),
    (native::RUNNING, State::Running),
    (native::IDLE, State::Idle),
];

fn outcome(result: native::CommandResult) -> Option<Outcome> {
    match result.kind {
        native::EXIT => Some(Outcome::Exit(result.value)),
        native::SIGNAL => Some(Outcome::Signal(result.value)),
        _ => None,
    }
}

/// The daemon's stored change time, which the library copies exactly as
/// `YYYY-MM-DDTHH:MM:SS.mmmZ`.
fn changed(at: native::Utc) -> Option<SystemTime> {
    let text = String::from_utf8_lossy(&at.bytes);
    OffsetDateTime::parse(&text, &Rfc3339)
        .ok()
        .map(SystemTime::from)
}

/// Monitor Open's response: opened, with whether `status-v1` was selected,
/// or refused with why. Any other request's answer is matched to its
/// request by the supervisor.
fn response(response: &native::Response) -> Vec<ClientEvent> {
    if response.request_id == OPEN_ID {
        return opened(response);
    }
    let answer = (response.outcome == native::OK)
        .then_some(())
        .ok_or_else(|| text(response.error.message));
    vec![ClientEvent::Answer(response.request_id, answer)]
}

/// Open's success says whether `status-v1` was selected, after telling the
/// session's terminal that `focus-v1` and `status-v1` were, so it knows
/// before the monitor can show the session opened without them.
fn opened(response: &native::Response) -> Vec<ClientEvent> {
    if (response.outcome, response.result_tag) != (native::OK, native::OPENED) {
        return vec![ClientEvent::Unavailable(text(response.error.message))];
    }
    // SAFETY: the result tag names the active member, and its tokens live
    // until the delivery's release.
    let selected = unsafe {
        slice(
            response.result.opened.selected.data,
            response.result.opened.selected.count,
        )
    };
    let selects = |family: &str| selected.iter().any(|token| text(*token) == family);
    let focusable = selects(FOCUS).then_some(Presentation::Focusable);
    let status = selects(STATUS);
    let viewable = status.then_some(Presentation::Viewable);
    let presented = focusable.into_iter().chain(viewable);
    presented
        .map(ClientEvent::Presentation)
        .chain([ClientEvent::Opened { status }])
        .collect()
}

fn title(title: native::Title) -> ClientEvent {
    let value = (title.state == native::VALUE).then(|| text(title.value));
    ClientEvent::Report(Report::Title(value))
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
/// `data` is null, or points at `count` initialized items that outlive the
/// slice.
unsafe fn slice<'a, T>(data: *const T, count: u64) -> &'a [T] {
    if data.is_null() {
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

#[cfg(test)]
mod tests {
    //! What the supervisor reads from status-bearing records, as the native
    //! contract defines them (#107): hand-built records, with expected values
    //! written from the header and the daemon wire reference.
    use super::*;
    use std::{ptr, time::Duration};

    const ID: &str = "00000000-0000-4000-8000-000000000001";
    /// 2027-01-15T08:00:00.000Z, 1,800,000,000,000 ms after the epoch.
    const T0: &str = "2027-01-15T08:00:00.000Z";
    const LATER: &str = "2027-01-15T08:00:01.234Z";

    fn at(millis: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_millis(millis)
    }

    fn utc(text: &str) -> native::Utc {
        native::Utc {
            bytes: text.as_bytes().try_into().unwrap(),
        }
    }

    fn native_status(state: u32, changed_at: &str) -> native::Status {
        let episode = u32::from(matches!(
            state,
            native::NEEDS_ATTENTION | native::ACKNOWLEDGED
        ));
        native::Status {
            state,
            episode_present: episode,
            episode_id: u64::from(episode),
            changed_at: utc(changed_at),
        }
    }

    fn available(status: Option<native::Status>) -> native::AvailableStatus {
        native::AvailableStatus {
            available: u32::from(status.is_some()),
            reserved: 0,
            value: status.unwrap_or(native::Status {
                state: 0,
                episode_present: 0,
                episode_id: 0,
                changed_at: native::Utc { bytes: [0; 24] },
            }),
        }
    }

    fn result(kind: u32, value: u32) -> native::CommandResult {
        native::CommandResult {
            kind,
            value,
            signal_name: native::OptionalText {
                present: 0,
                reserved: 0,
                value: Bytes {
                    data: ptr::null(),
                    length: 0,
                },
            },
        }
    }

    fn record(tag: u32, data: native::RecordData) -> native::Record {
        native::Record {
            tag,
            reserved: 0,
            generation: 1,
            data,
        }
    }

    fn metadata(
        status: Option<native::Status>,
        command: Option<native::CommandResult>,
    ) -> native::Record {
        let unset = native::Title {
            state: native::UNSET,
            reserved: 0,
            value: Bytes {
                data: ptr::null(),
                length: 0,
            },
        };
        let metadata = native::Metadata {
            title: unset,
            status: available(status),
            progress: native::AvailableProgress {
                available: 0,
                reserved: 0,
                value: native::Progress {
                    state: 0,
                    percent_present: 0,
                    percent: 0,
                    reserved: 0,
                },
            },
            command_result: native::OptionalCommand {
                present: u32::from(command.is_some()),
                reserved: 0,
                value: command.unwrap_or_else(|| result(0, 0)),
            },
        };
        record(native::INITIAL_METADATA, native::RecordData { metadata })
    }

    fn read_one(record: &native::Record) -> Vec<ClientEvent> {
        read(&ID.parse().unwrap(), record)
    }

    fn final_event(outcome: Outcome, changed: Option<SystemTime>) -> ClientEvent {
        ClientEvent::Report(Report::Final(Final { outcome, changed }))
    }

    /// STA-07 and COM-06 (Monitor, pure supplement): the Initial metadata
    /// reports its title, then the stored status and its exact change time
    /// (never the time it arrived), or, once the command ended, its final
    /// outcome with the terminal status's time when `status-v1` reported
    /// one; without status or result, nothing is fabricated.
    #[test]
    fn the_initial_metadata_reports_its_status_or_final_outcome() {
        let untitled = ClientEvent::Report(Report::Title(None));
        let idle = native_status(native::IDLE, LATER);
        assert_eq!(
            read_one(&metadata(Some(idle), None)),
            [
                untitled.clone(),
                ClientEvent::Report(Report::Status(Status {
                    state: State::Idle,
                    changed: at(1_800_000_001_234),
                }))
            ]
        );
        assert_eq!(
            read_one(&metadata(None, None)),
            std::slice::from_ref(&untitled)
        );
        let completed = native_status(native::COMPLETED, T0);
        assert_eq!(
            read_one(&metadata(Some(completed), Some(result(native::EXIT, 0)))),
            [
                untitled.clone(),
                final_event(Outcome::Exit(0), Some(at(1_800_000_000_000)))
            ]
        );
        assert_eq!(
            read_one(&metadata(None, Some(result(native::SIGNAL, 15)))),
            [untitled, final_event(Outcome::Signal(15), None)]
        );
    }

    /// STA-07 (Monitor, pure supplement): each nonfinal state maps to its
    /// own status with the daemon's exact time; a final state, an unknown
    /// one or an unreadable time reports nothing rather than a guess.
    #[test]
    fn status_records_keep_their_state_and_exact_change_time() {
        let changed = |state, time| {
            let status = native_status(state, time);
            read_one(&record(native::STATUS, native::RecordData { status }))
        };
        for (state, expected) in [
            (native::NEEDS_ATTENTION, State::NeedsAttention),
            (native::ACKNOWLEDGED, State::Acknowledged),
            (native::RUNNING, State::Running),
            (native::IDLE, State::Idle),
        ] {
            assert_eq!(
                changed(state, T0),
                [ClientEvent::Report(Report::Status(Status {
                    state: expected,
                    changed: at(1_800_000_000_000),
                }))]
            );
        }
        for state in [0, native::COMPLETED, native::STATUS_FAILED, 7] {
            assert_eq!(changed(state, T0), []);
        }
        assert_eq!(changed(native::IDLE, "2027-13-15T08:00:00.000Z"), []);
    }

    /// COM-06 (Monitor, pure supplement): Command final reports exit and
    /// signal apart, exit 143 not signal 15, with the terminal status's time
    /// when `status-v1` is selected and none otherwise; an unknown result
    /// kind reports nothing.
    #[test]
    fn command_final_reports_its_exact_result() {
        let ended = |command_result, status| {
            let data = native::RecordData {
                r#final: native::Final {
                    command_result,
                    status: available(status),
                },
            };
            read_one(&record(native::FINAL, data))
        };
        let failed = native_status(native::STATUS_FAILED, LATER);
        assert_eq!(
            ended(result(native::EXIT, 143), Some(failed)),
            [final_event(Outcome::Exit(143), Some(at(1_800_000_001_234)))]
        );
        assert_eq!(
            ended(result(native::SIGNAL, 15), None),
            [final_event(Outcome::Signal(15), None)]
        );
        assert_eq!(ended(result(3, 1), None), []);
    }

    fn bytes(text: &'static [u8]) -> Bytes {
        Bytes {
            data: text.as_ptr(),
            length: text.len() as u64,
        }
    }

    /// Monitor Open's successful response, selecting `families`.
    fn open_response(families: &[Bytes]) -> Vec<ClientEvent> {
        let selected = native::Tokens {
            data: if families.is_empty() {
                ptr::null()
            } else {
                families.as_ptr()
            },
            count: families.len() as u64,
        };
        // SAFETY: an all-zero error and result are valid plain values.
        let mut response: native::Response = unsafe { std::mem::zeroed() };
        response.request_id = OPEN_ID;
        response.outcome = native::OK;
        response.result_tag = native::OPENED;
        response.result.opened = native::Opened {
            connection_id: native::Guid { bytes: [0; 16] },
            selected,
        };
        read_one(&record(native::RESPONSE, native::RecordData { response }))
    }

    /// FOC-01, FOC-02 and STA-08 (Monitor, pure supplement): Open's success
    /// says first that `focus-v1` was selected, when it was, then that
    /// `status-v1` was, so viewing can be reported (#114), then whether
    /// `status-v1` was; any other family changes nothing.
    #[test]
    fn opening_says_whether_focus_and_status_were_selected() {
        let focusable = ClientEvent::Presentation(Presentation::Focusable);
        let viewable = ClientEvent::Presentation(Presentation::Viewable);
        let opened = |status| ClientEvent::Opened { status };
        let focus = bytes(b"focus-v1");
        let status = bytes(b"status-v1");
        let title = bytes(b"title-v1");
        assert_eq!(
            open_response(&[status, title, focus]),
            [focusable.clone(), viewable.clone(), opened(true)]
        );
        assert_eq!(open_response(&[focus]), [focusable, opened(false)]);
        assert_eq!(open_response(&[title, status]), [viewable, opened(true)]);
        assert_eq!(open_response(&[]), [opened(false)]);
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
            let request = native_request(9, &Request::Viewing(held, viewing));
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
            let request = native_request(11, &request);
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
            let request = native_request(9, &Request::Focus(held, focused));
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
        let request = native_request(10, &paste);
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
}
