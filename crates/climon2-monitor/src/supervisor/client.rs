//! One instance's caller handles and what draining them reports: the only
//! place the supervisor reads the client library's records.
use crate::registry::{ClientEvent, Final, Outcome, State, Status};
use climon2_core::{
    SessionId,
    native::{self, Bytes, Call, Communication, Control, Drained},
};
use climon2_platform::library::Functions;
use std::{mem::MaybeUninit, time::SystemTime};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

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
const READERS: [(u32, Reader); 6] = [
    (native::RESPONSE, |id, record| {
        response(id, record).into_iter().collect()
    }),
    (native::INITIAL_METADATA, |_, record| {
        initial(unsafe { &record.data.metadata })
    }),
    (native::TITLE, |_, record| {
        vec![title(unsafe { record.data.title })]
    }),
    (native::STATUS, |_, record| {
        status(unsafe { record.data.status })
            .map(ClientEvent::Status)
            .into_iter()
            .collect()
    }),
    (native::FINAL, |_, record| {
        let ended = unsafe { record.data.r#final };
        finished(ended.command_result, ended.status)
            .into_iter()
            .collect()
    }),
    (native::DIAGNOSTIC, |id, record| {
        diagnostic(id, record);
        Vec::new()
    }),
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

/// The Initial metadata's title, then its final outcome when the command
/// already ended, otherwise its status when `status-v1` reported one. The
/// library has checked that a final status comes exactly with a result.
fn initial(metadata: &native::Metadata) -> Vec<ClientEvent> {
    let result = metadata.command_result;
    let progress = if result.present == 0 {
        available(metadata.status)
            .and_then(status)
            .map(ClientEvent::Status)
    } else {
        finished(result.value, metadata.status)
    };
    std::iter::once(title(metadata.title))
        .chain(progress)
        .collect()
}

/// A final outcome, with the terminal status's change time when
/// `status-v1` reported one.
fn finished(result: native::CommandResult, status: native::AvailableStatus) -> Option<ClientEvent> {
    let changed = available(status).and_then(|status| changed(status.changed_at));
    outcome(result).map(|outcome| ClientEvent::Final(Final { outcome, changed }))
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
/// or refused with why.
fn response(_: &SessionId, record: &native::Record) -> Option<ClientEvent> {
    // SAFETY: the tag names the active member.
    let response = unsafe { record.data.response };
    (response.request_id == OPEN_ID).then(|| opened(&response))
}

fn opened(response: &native::Response) -> ClientEvent {
    if (response.outcome, response.result_tag) != (native::OK, native::OPENED) {
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

fn diagnostic(id: &SessionId, record: &native::Record) {
    // SAFETY: the tag names the active member.
    let error = unsafe { record.data.diagnostic };
    tracing::warn!(session = %id, code = %text(error.code), message = %text(error.message), "client diagnostic");
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
        ClientEvent::Final(Final { outcome, changed })
    }

    /// STA-07 and COM-06 (Monitor, pure supplement): the Initial metadata
    /// reports its title, then the stored status and its exact change time
    /// (never the time it arrived), or, once the command ended, its final
    /// outcome with the terminal status's time when `status-v1` reported
    /// one; without status or result, nothing is fabricated.
    #[test]
    fn the_initial_metadata_reports_its_status_or_final_outcome() {
        let untitled = ClientEvent::Title(None);
        let idle = native_status(native::IDLE, LATER);
        assert_eq!(
            read_one(&metadata(Some(idle), None)),
            [
                untitled.clone(),
                ClientEvent::Status(Status {
                    state: State::Idle,
                    changed: at(1_800_000_001_234),
                })
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
                [ClientEvent::Status(Status {
                    state: expected,
                    changed: at(1_800_000_000_000),
                })]
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
}
