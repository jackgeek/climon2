//! What draining a client reports: the only place the supervisor reads
//! the client library's records. Client embedding takes each delivery and
//! releases it at once; this is its reader.
use crate::{
    registry::{Answer, ClientEvent, Final, Outcome, Recovery, Report, State, Status, Verdict},
    terminal::{Authority, Presentation, Revocation},
};
use climon2_core::{
    SessionId,
    native::{self, Bytes},
};
pub(super) use climon2_embed::instance::OPEN_ID;
use climon2_embed::instance::{self, Retry, slice};
use climon2_vt::Geometry;
use std::time::SystemTime;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const STATUS: &str = "status-v1";
const FOCUS: &str = "focus-v1";

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
    (native::REVOKED, |_, record| {
        let reason = unsafe { record.data.revoked.reason };
        presented(Some(Presentation::Revoked(revocation(reason))))
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

/// What `record`, delivered for `id`, reports.
pub(super) fn read(id: &SessionId, record: &native::Record) -> Vec<ClientEvent> {
    READERS
        .iter()
        .find(|(tag, _)| *tag == record.tag)
        .map_or_else(Vec::new, |(_, reader)| reader(id, record))
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

/// Why control was revoked: a takeover, the session's end, or a reason
/// this monitor does not know.
fn revocation(reason: u32) -> Revocation {
    match reason {
        native::TAKEOVER => Revocation::Takeover,
        native::SESSION_ENDING => Revocation::Ending,
        _ => Revocation::Unknown,
    }
}

fn presented(presentation: Option<Presentation>) -> Vec<ClientEvent> {
    presentation
        .map(ClientEvent::Presentation)
        .into_iter()
        .collect()
}

/// The Initial metadata's title, then its final outcome when the command
/// already ended, otherwise that it had not (#115) and its status when
/// `status-v1` reported one. The library has checked that a final status
/// comes exactly with a result.
fn initial(metadata: &native::Metadata) -> Vec<ClientEvent> {
    let result = metadata.command_result;
    let standing = if result.present == 0 {
        let status = available(metadata.status).and_then(status);
        let status = status.map(|status| ClientEvent::Report(Report::Status(status)));
        [Some(ClientEvent::Report(Report::Unfinished)), status]
    } else {
        [finished(result.value, metadata.status), None]
    };
    std::iter::once(title(metadata.title))
        .chain(standing.into_iter().flatten())
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
        episode: (status.episode_present != 0).then_some(status.episode_id),
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
    vec![ClientEvent::Answer(response.request_id, answer(response))]
}

/// An answer: accepted, refused without effect, or failed or
/// indeterminate, with the command's result when it carries one.
fn answer(response: &native::Response) -> Answer {
    let why = || text(response.error.message);
    let verdict = match response.outcome {
        native::OK => Verdict::Accepted,
        native::REJECTED => Verdict::Rejected(why()),
        _ => Verdict::Uncertain(why()),
    };
    Answer {
        verdict,
        ended: carried(response),
    }
}

/// The command's result an answer carries: a Kill's termination result,
/// when present, or a known-final rejection's (#117).
fn carried(response: &native::Response) -> Option<Outcome> {
    // SAFETY (both): the result tag names the active member.
    match response.result_tag {
        native::TERMINATION => {
            let command = unsafe { response.result.termination }.command_result;
            outcome(command.value).filter(|_| command.present != 0)
        }
        native::KNOWN_FINAL => outcome(unsafe { response.result.known_final }),
        _ => None,
    }
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

/// Library text, copied lossily: it is diagnostic or display text.
fn text(bytes: Bytes) -> String {
    // SAFETY: counted library bytes valid for this call.
    unsafe { instance::text(bytes) }
}

/// What the row learns of `retry` after `why`, if it is retried or
/// rejected.
pub(super) fn recovery(retry: Retry, why: String) -> Option<Recovery> {
    match retry {
        Retry::None => None,
        Retry::Passive => Some(Recovery::Retrying(why)),
        Retry::Permanent => Some(Recovery::Rejected(why)),
    }
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
    /// reports its title, then, before the command ended, that it had not
    /// (#115) and the stored status and its exact change time (never the
    /// time it arrived), or, once it ended, its final outcome with the
    /// terminal status's time when `status-v1` reported one; without status
    /// or result, no status is fabricated.
    #[test]
    fn the_initial_metadata_reports_its_status_or_final_outcome() {
        let untitled = ClientEvent::Report(Report::Title(None));
        let unfinished = ClientEvent::Report(Report::Unfinished);
        let idle = native_status(native::IDLE, LATER);
        assert_eq!(
            read_one(&metadata(Some(idle), None)),
            [
                untitled.clone(),
                unfinished.clone(),
                ClientEvent::Report(Report::Status(Status {
                    state: State::Idle,
                    changed: at(1_800_000_001_234),
                    episode: None,
                }))
            ]
        );
        assert_eq!(
            read_one(&metadata(None, None)),
            [untitled.clone(), unfinished]
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

    /// STA-07 and NOT-01 (Monitor, pure supplement): each nonfinal state
    /// maps to its own status with the daemon's exact time and its
    /// attention episode, if present; a final state, an unknown one or an
    /// unreadable time reports nothing rather than a guess.
    #[test]
    fn status_records_keep_their_state_and_exact_change_time() {
        let changed = |state, time| {
            let status = native_status(state, time);
            read_one(&record(native::STATUS, native::RecordData { status }))
        };
        for (state, expected, episode) in [
            (native::NEEDS_ATTENTION, State::NeedsAttention, Some(1)),
            (native::ACKNOWLEDGED, State::Acknowledged, Some(1)),
            (native::RUNNING, State::Running, None),
            (native::IDLE, State::Idle, None),
        ] {
            assert_eq!(
                changed(state, T0),
                [ClientEvent::Report(Report::Status(Status {
                    state: expected,
                    changed: at(1_800_000_000_000),
                    episode,
                }))]
            );
        }
        let status = native::Status {
            episode_id: 9,
            ..native_status(native::NEEDS_ATTENTION, T0)
        };
        let read = read_one(&record(native::STATUS, native::RecordData { status }));
        let ClientEvent::Report(Report::Status(attention)) = &read[0] else {
            panic!("{read:?}");
        };
        assert_eq!(attention.episode, Some(9));
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

    /// A response to request 5 with `outcome`, its error's message `why`
    /// and the result `tag` and `result`.
    fn answered(
        outcome: u32,
        why: &'static [u8],
        tag: u32,
        result: native::ResponseResult,
    ) -> Vec<ClientEvent> {
        // SAFETY: an all-zero response is a valid plain value.
        let mut response: native::Response = unsafe { std::mem::zeroed() };
        response.request_id = 5;
        response.outcome = outcome;
        response.error.message = bytes(why);
        response.result_tag = tag;
        response.result = result;
        read_one(&record(native::RESPONSE, native::RecordData { response }))
    }

    /// DEL-01 and DEL-02 (pure supplement, the header's `CM1_TERMINATION`
    /// and `CM1_KNOWN_FINAL`; #117): an answer says whether it was accepted,
    /// refused without effect, or failed or indeterminate, with why; a
    /// Kill's termination carries its command result only when present, and
    /// a known-final rejection its result; no other result carries one.
    #[test]
    fn answers_say_how_they_ended_and_carry_a_kills_command_result() {
        let terminated = |present, value| native::ResponseResult {
            termination: native::Termination {
                command_result: native::OptionalCommand {
                    present,
                    reserved: 0,
                    value,
                },
                cleanup: native::CLEANUP_COMPLETE,
                reserved: 0,
            },
        };
        let none = native::ResponseResult { changed: 0 };
        let event = |verdict, ended| vec![ClientEvent::Answer(5, Answer { verdict, ended })];
        assert_eq!(
            answered(
                native::OK,
                b"",
                native::TERMINATION,
                terminated(1, result(native::SIGNAL, 9))
            ),
            event(Verdict::Accepted, Some(Outcome::Signal(9)))
        );
        assert_eq!(
            answered(
                native::INDETERMINATE,
                b"lost",
                native::TERMINATION,
                terminated(0, result(native::EXIT, 1))
            ),
            event(Verdict::Uncertain("lost".into()), None)
        );
        assert_eq!(
            answered(
                native::FAILED,
                b"failed",
                native::TERMINATION,
                terminated(1, result(native::EXIT, 3))
            ),
            event(Verdict::Uncertain("failed".into()), Some(Outcome::Exit(3)))
        );
        let known = native::ResponseResult {
            known_final: result(native::EXIT, 0),
        };
        assert_eq!(
            answered(native::REJECTED, b"ended", native::KNOWN_FINAL, known),
            event(Verdict::Rejected("ended".into()), Some(Outcome::Exit(0)))
        );
        assert_eq!(
            answered(native::REJECTED, b"stale", native::RESULT_NONE, none),
            event(Verdict::Rejected("stale".into()), None)
        );
        let flag = native::ResponseResult {
            flag: native::FlagResult {
                changed: 1,
                value: 1,
                reserved: [0; 2],
                accepted_bytes: 0,
            },
        };
        assert_eq!(
            answered(native::OK, b"", native::VIEWING_RESULT, flag),
            event(Verdict::Accepted, None)
        );
        let answer = |verdict| {
            Answer {
                verdict,
                ended: None,
            }
            .result()
        };
        assert_eq!(answer(Verdict::Accepted), Ok(()));
        assert_eq!(answer(Verdict::Rejected("no".into())), Err("no".into()));
        assert_eq!(
            answer(Verdict::Uncertain("lost".into())),
            Err("lost".into())
        );
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

    /// REC-01 (pure supplement): the row learns that a passive retry is
    /// coming, that a permanent rejection is final, and nothing of an end
    /// that is not retried.
    #[test]
    fn a_retry_class_tells_the_row_whether_it_is_retried_or_rejected() {
        assert_eq!(recovery(Retry::None, "why".into()), None);
        assert_eq!(
            recovery(Retry::Passive, "why".into()),
            Some(Recovery::Retrying("why".into()))
        );
        assert_eq!(
            recovery(Retry::Permanent, "why".into()),
            Some(Recovery::Rejected("why".into()))
        );
    }
}
