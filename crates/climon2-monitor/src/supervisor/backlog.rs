//! What each client delivered that the UI thread has not yet processed
//! (#119, LIM-02). The supervisor copies each delivery into reports and
//! releases it at once, so ownership of the bytes moves to the UI thread
//! and so does their accounting: each report is charged here, by the
//! allocation it holds, until the UI has processed it. A client whose
//! reports fill its `queues.event_bytes` is not drained again until the UI
//! catches up; the library holds what it has not delivered, and nothing is
//! dropped.
use crate::{
    registry::{ClientEvent, Recovery, Report},
    terminal::{Presentation, Request},
};
use climon2_core::SessionId;
use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

/// Each session's reports sent to the UI thread and not yet processed, in
/// bytes.
#[derive(Default)]
pub(crate) struct Backlog(Mutex<BTreeMap<SessionId, usize>>);

impl Backlog {
    /// `event`, sent for `id`, is charged until it is processed.
    pub(crate) fn charge(&self, id: &SessionId, event: &ClientEvent) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = held.entry(id.clone()).or_default();
        *entry = entry.saturating_add(footprint(id, event));
    }

    /// The UI processed a report for `id` that held `bytes`
    /// ([`footprint`]): they are free again.
    pub(crate) fn release(&self, id: &SessionId, bytes: usize) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = held.get_mut(id) {
            *entry = entry.saturating_sub(bytes);
        }
    }

    /// The bytes `id`'s unprocessed reports hold.
    pub(crate) fn held(&self, id: &SessionId) -> usize {
        let held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.get(id).copied().unwrap_or_default()
    }

    /// The bytes every session's unprocessed reports hold together.
    pub(crate) fn total(&self) -> usize {
        let held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.values().sum()
    }
}

/// The allocation one report for `id` holds: the report itself, its
/// session ID, and the text, bytes or input it carries.
pub(crate) fn footprint(id: &SessionId, event: &ClientEvent) -> usize {
    size_of::<(SessionId, ClientEvent)>() + id.as_str().len() + carried(event)
}

/// What a report carries beyond itself.
fn carried(event: &ClientEvent) -> usize {
    match event {
        ClientEvent::Presentation(presentation) => presented(presentation),
        ClientEvent::Report(Report::Title(Some(text)))
        | ClientEvent::Unavailable(text)
        | ClientEvent::Recovery(Recovery::Retrying(text) | Recovery::Rejected(text))
        | ClientEvent::Answer(_, Err(text)) => text.capacity(),
        _ => 0,
    }
}

/// What a presentation carries: its display bytes, or an answered
/// request's bytes and refusal.
fn presented(presentation: &Presentation) -> usize {
    match presentation {
        Presentation::Display(bytes) => bytes.capacity(),
        Presentation::Answered(request, answer) => {
            requested(request) + answer.as_ref().err().map_or(0, String::capacity)
        }
        _ => 0,
    }
}

/// The input or paste bytes a request carries.
fn requested(request: &Request) -> usize {
    match request {
        Request::Input(_, bytes) | Request::Paste(_, crate::terminal::Paste { bytes, .. }) => {
            bytes.capacity()
        }
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{Authority, Paste};

    fn id(index: u8) -> SessionId {
        format!("0d5c0000-0000-4000-8000-0000000000{index:02x}")
            .parse()
            .unwrap()
    }

    /// The fixed part of every report: the report and its 36-byte ID.
    fn record() -> usize {
        size_of::<(SessionId, ClientEvent)>() + 36
    }

    fn text(capacity: usize) -> String {
        String::with_capacity(capacity)
    }

    /// LIM-02 (pure supplement): each report is charged by the allocation
    /// it holds, capacity and not length: the record and its session ID,
    /// display bytes, titles and diagnostics, refusals, and the input or
    /// paste an answered request carries; nothing else.
    #[test]
    fn each_report_is_charged_by_the_capacity_it_holds() {
        let held = Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let mut display = Vec::with_capacity(100);
        display.push(b'x');
        let cases = [
            (
                ClientEvent::Presentation(Presentation::Display(display)),
                100,
            ),
            (ClientEvent::Report(Report::Title(Some(text(7)))), 7),
            (ClientEvent::Report(Report::Title(None)), 0),
            (ClientEvent::Unavailable(text(9)), 9),
            (ClientEvent::Recovery(Recovery::Retrying(text(11))), 11),
            (ClientEvent::Recovery(Recovery::Rejected(text(13))), 13),
            (ClientEvent::Recovery(Recovery::Gone), 0),
            (ClientEvent::Answer(4, Err(text(15))), 15),
            (ClientEvent::Answer(4, Ok(())), 0),
            (
                ClientEvent::Presentation(Presentation::Answered(
                    Request::Input(held, Vec::with_capacity(17)),
                    Err(text(19)),
                )),
                36,
            ),
            (
                ClientEvent::Presentation(Presentation::Answered(
                    Request::Paste(
                        held,
                        Paste {
                            bytes: Vec::with_capacity(21),
                            ..Paste::end()
                        },
                    ),
                    Ok(()),
                )),
                21,
            ),
            (
                ClientEvent::Presentation(Presentation::Answered(Request::Acquire, Ok(()))),
                0,
            ),
            (ClientEvent::Presentation(Presentation::Complete), 0),
            (ClientEvent::Ended, 0),
        ];
        for (event, carried) in cases {
            assert_eq!(footprint(&id(1), &event), record() + carried, "{event:?}");
        }
    }

    /// LIM-02 (pure supplement): a session's reports are held until each
    /// is processed, apart from every other session's, and the total is
    /// theirs together; releasing what was never charged frees nothing.
    #[test]
    fn reports_are_held_per_session_until_processed() {
        let backlog = Backlog::default();
        let (first, second) = (id(1), id(2));
        let display = ClientEvent::Presentation(Presentation::Display(Vec::with_capacity(50)));
        backlog.release(&first, record());
        assert_eq!(backlog.held(&first), 0);
        backlog.charge(&first, &display);
        backlog.charge(&first, &ClientEvent::Ended);
        backlog.charge(&second, &ClientEvent::Ended);
        assert_eq!(backlog.held(&first), 2 * record() + 50);
        assert_eq!(backlog.held(&second), record());
        assert_eq!(backlog.total(), 3 * record() + 50);
        backlog.release(&first, footprint(&first, &display));
        assert_eq!(backlog.held(&first), record());
        backlog.release(&first, record());
        assert_eq!(backlog.held(&first), 0);
        assert_eq!(backlog.total(), record());
    }
}
