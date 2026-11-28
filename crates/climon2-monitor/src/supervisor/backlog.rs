//! What one report to the UI thread holds (#119, LIM-02). The supervisor
//! copies each delivery into reports and releases it at once, so ownership
//! of the bytes moves to the UI thread and so does their accounting: each
//! report is charged to its session's backlog (client embedding's
//! `Backlog`), by the allocation it holds, until the UI has processed it.
use crate::{
    registry::{Answer, ClientEvent, Recovery, Report, Termination, Verdict},
    terminal::{Presentation, Request},
};
use climon2_core::SessionId;

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
        | ClientEvent::Answer(
            _,
            Answer {
                verdict: Verdict::Rejected(text) | Verdict::Uncertain(text),
                ..
            },
        )
        | ClientEvent::Termination(Termination::Refused(text) | Termination::Unconfirmed(text)) => {
            text.capacity()
        }
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

    fn answer(verdict: Verdict) -> Answer {
        Answer {
            verdict,
            ended: None,
        }
    }

    /// LIM-02 (pure supplement): each report is charged by the allocation
    /// it holds, capacity and not length: the record and its session ID,
    /// display bytes, titles and diagnostics, refusals and Delete's
    /// termination diagnostics (#117), and the input or
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
            (
                ClientEvent::Answer(4, answer(Verdict::Rejected(text(15)))),
                15,
            ),
            (
                ClientEvent::Answer(4, answer(Verdict::Uncertain(text(16)))),
                16,
            ),
            (ClientEvent::Answer(4, answer(Verdict::Accepted)), 0),
            (ClientEvent::Termination(Termination::Refused(text(21))), 21),
            (
                ClientEvent::Termination(Termination::Unconfirmed(text(22))),
                22,
            ),
            (ClientEvent::Removed, 0),
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
}
