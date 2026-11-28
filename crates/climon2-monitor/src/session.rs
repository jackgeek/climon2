//! The selected session as a consumer shows it (ADR 0011): its label,
//! status, control and error, and what its canvas shows, as read-only
//! views of its terminal's model. What each looks like on a device is the
//! consumer's.
use crate::{
    registry::{self, Kind, Recovery, Registry},
    terminal::{Control, Terminal},
};
use climon2_core::SessionId;
use climon2_vt::view as vt;

/// What a row's recovery says once its daemon is gone (owner decision on
/// #116, 2026-11-16).
pub const GONE: &str = "Session no longer exists";

/// The selected session as the top row and canvas show it.
pub struct Session<'a> {
    pub id: &'a SessionId,
    pub label: &'a str,
    pub status: &'a str,
    pub control: &'a str,
    pub error: Option<&'a str>,
    pub canvas: Canvas<'a>,
}

/// What the canvas below the top row shows of the selected session.
pub enum Canvas<'a> {
    /// Its live or frozen model.
    View(vt::View<'a>),
    /// Connected and not controlled by this monitor: the placeholder,
    /// never its cached output.
    Placeholder(Placeholder),
    /// Not connected, its retained history read-only (#116), under the
    /// overlay its recovery calls for.
    Retained(vt::View<'a>, Option<&'a Recovery>),
    /// Selected without a connected client and with no retained history:
    /// no terminal content, its condition and the overlay its recovery
    /// calls for.
    NoContent(&'a str, Option<&'a Recovery>),
    /// Nothing to show yet: acquiring, before the first restoration.
    Nothing,
}

/// What the non-controller placeholder says of who controls the session.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Placeholder {
    /// Another client took control from this monitor: viewed elsewhere.
    TakenOver,
    /// No other client is known to control it: only that this monitor
    /// does not (#400).
    NotControlled,
}

impl Canvas<'_> {
    /// The live or frozen view, which the host follows.
    pub fn view(&self) -> Option<&vt::View<'_>> {
        match self {
            Self::View(view) => Some(view),
            _ => None,
        }
    }

    /// Any view painted, read-only history included.
    pub fn painted(&self) -> Option<&vt::View<'_>> {
        match self {
            Self::View(view) | Self::Retained(view, _) => Some(view),
            _ => None,
        }
    }
}

/// What the top row's error slot and the read-only view say of a row's
/// recovery: its diagnostic, or that its daemon is gone (owner decision on
/// #116, 2026-11-16).
pub fn diagnostic(recovery: &Recovery) -> &str {
    match recovery {
        Recovery::Retrying(why) | Recovery::Rejected(why) => why,
        Recovery::Gone => GONE,
    }
}

/// What `registry` says of `id`'s row beyond its view, if it has one.
pub(crate) fn shown<'a>(registry: &'a Registry, id: &SessionId) -> Option<Shown<'a>> {
    Some(Shown {
        kind: registry.kind(id)?,
        finished: registry.finished(id),
        recovery: registry.recovery(id),
        condition: registry.condition(id),
        refused: registry.error(id),
    })
}

/// What the registry says of a selected row beyond its view.
pub(crate) struct Shown<'a> {
    pub(crate) kind: Kind,
    /// Its final outcome is known.
    pub(crate) finished: bool,
    recovery: Option<&'a Recovery>,
    /// Why it waits for its client: resource-blocked, or its release
    /// loading slowly (#120).
    condition: Option<&'a str>,
    /// Why a confirmed Delete's Kill was refused or is unconfirmed (#117).
    refused: Option<&'a str>,
}

/// The selected row as the top row shows it: its label, its row status
/// (#107), its control, any error (why its Delete's Kill fell short, #117,
/// otherwise that its daemon is gone, otherwise its terminal's own,
/// otherwise why it waits for its client, #120, or its recovery's
/// diagnostic, owner decision on #116), and what its canvas shows. A gone
/// daemon outranks the terminal's own error, which its lost connection may
/// have left: a request the stream's end cancelled.
pub(crate) fn selected<'a>(
    row: &'a registry::View,
    shown: Shown<'a>,
    terminal: Option<&'a Terminal>,
) -> Session<'a> {
    let control = control(terminal);
    let canvas = terminal.and_then(Terminal::view).map_or_else(
        || {
            let retained = retained(shown.kind, shown.finished, terminal);
            let says = says(terminal);
            without_view(&shown, &row.status, control, says, retained)
        },
        Canvas::View,
    );
    let recovery = shown.recovery.map(diagnostic);
    let gone = recovery.filter(|_| shown.recovery == Some(&Recovery::Gone));
    let diagnostic = shown.condition.or(recovery);
    Session {
        id: &row.id,
        label: &row.label,
        status: &row.status,
        control: control.label(),
        error: shown
            .refused
            .or(gone)
            .or_else(|| terminal.and_then(Terminal::error))
            .or(diagnostic),
        canvas,
    }
}

/// The read-only history a selected row shows (#116): a row not connected
/// and not known final keeps its retained cache, if it ever had one. A
/// final row shows only the view frozen at its end (#108).
pub(crate) fn retained(
    kind: Kind,
    finished: bool,
    terminal: Option<&Terminal>,
) -> Option<vt::View<'_>> {
    let inspected = kind != Kind::Connected && !finished;
    terminal.filter(|_| inspected).and_then(Terminal::retained)
}

/// The control the monitor holds through `terminal`: a session it never
/// made a terminal for is not controlled.
pub(crate) fn control(terminal: Option<&Terminal>) -> Control {
    terminal.map_or(Control::NotControlling, Terminal::control)
}

/// What the placeholder says of the session `terminal` shows: that it is
/// viewed elsewhere only once another client took control from this
/// monitor, otherwise only that this monitor does not control it (#400).
fn says(terminal: Option<&Terminal>) -> Placeholder {
    if terminal.is_some_and(Terminal::taken_over) {
        Placeholder::TakenOver
    } else {
        Placeholder::NotControlled
    }
}

/// The canvas of a selected row whose terminal has no live or frozen view:
/// the placeholder `says` while the monitor does not control a connected
/// session, never its cache; nothing yet while it acquires a connected
/// one; for any other row its `retained` history read-only, or no terminal
/// content and its `status`, under its recovery's overlay.
fn without_view<'a>(
    shown: &Shown<'a>,
    status: &'a str,
    control: Control,
    says: Placeholder,
    retained: Option<vt::View<'a>>,
) -> Canvas<'a> {
    match (shown.kind, retained) {
        (Kind::Connected, _) if placeholder(shown.kind, control) => Canvas::Placeholder(says),
        (Kind::Connected, _) => Canvas::Nothing,
        (_, Some(view)) => Canvas::Retained(view, shown.recovery),
        (_, None) => Canvas::NoContent(status, shown.recovery),
    }
}

/// Whether a selected row shows the non-controller placeholder, on which
/// the take-control key acquires: connected, not known to be final, and
/// neither controlled nor being acquired by this monitor.
pub(crate) fn placeholder(kind: Kind, control: Control) -> bool {
    kind == Kind::Connected && control == Control::NotControlling
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        registry::View,
        terminal::{self, Presentation},
    };

    const CONTROLS: [Control; 5] = [
        Control::Acquiring,
        Control::Restoring,
        Control::Controlling,
        Control::NotControlling,
        Control::Ended,
    ];

    /// CTL-02 and INP-01 takeover (pure supplement, #110): only a connected
    /// row the monitor neither controls, nor is acquiring, nor saw end shows
    /// the placeholder and takes the take-control key; no other kind does,
    /// and a session the monitor never made a terminal for is not
    /// controlled.
    #[test]
    fn only_an_uncontrolled_connected_row_shows_the_placeholder() {
        for kind in [Kind::Connected, Kind::Unconnected, Kind::Retained] {
            let shown: Vec<_> = CONTROLS
                .into_iter()
                .map(|control| placeholder(kind, control))
                .collect();
            let expected = [false, false, false, kind == Kind::Connected, false];
            assert_eq!(shown, expected, "{kind:?}");
        }
        assert_eq!(control(None), Control::NotControlling);
    }

    /// Spec #73 Enter and placeholders (pure supplement, #110, #116): with
    /// no live or frozen view to paint, a connected row the monitor does not
    /// control shows the placeholder, never a cache, and one it is acquiring
    /// nothing yet; any other row shows its retained history read-only, or
    /// that it has no terminal content and its status, under its recovery's
    /// overlay.
    #[test]
    fn a_selected_row_without_a_view_shows_the_placeholder_its_history_or_no_content() {
        let model = climon2_vt::Model::new(
            climon2_vt::Geometry::for_host(10, 3).unwrap(),
            climon2_vt::Limits {
                bytes: 64 << 20,
                history_rows: 200,
            },
        )
        .unwrap();
        let retrying = Recovery::Retrying("lost".into());
        const NOT_HERE: Placeholder = Placeholder::NotControlled;
        let shape = |canvas: Canvas<'_>| match canvas {
            Canvas::View(_) => "view".to_owned(),
            Canvas::Placeholder(says) => format!("placeholder, {says:?}"),
            Canvas::Retained(_, recovery) => format!("history, {recovery:?}"),
            Canvas::NoContent(condition, recovery) => {
                format!("no content, {condition}, {recovery:?}")
            }
            Canvas::Nothing => "nothing".to_owned(),
        };
        let shown = |kind, control, cached: bool| {
            let shown = Shown {
                kind,
                finished: false,
                recovery: Some(&retrying),
                condition: None,
                refused: None,
            };
            let view = cached.then(|| model.view());
            shape(without_view(&shown, "Status", control, NOT_HERE, view))
        };
        assert_eq!(
            shown(Kind::Connected, Control::NotControlling, true),
            "placeholder, NotControlled"
        );
        let taken = without_view(
            &Shown {
                kind: Kind::Connected,
                finished: false,
                recovery: None,
                condition: None,
                refused: None,
            },
            "Status",
            Control::NotControlling,
            Placeholder::TakenOver,
            None,
        );
        assert_eq!(shape(taken), "placeholder, TakenOver");
        assert_eq!(shown(Kind::Connected, Control::Acquiring, true), "nothing");
        for kind in [Kind::Unconnected, Kind::Retained] {
            assert_eq!(
                shown(kind, Control::NotControlling, true),
                "history, Some(Retrying(\"lost\"))"
            );
            assert_eq!(
                shown(kind, Control::NotControlling, false),
                "no content, Status, Some(Retrying(\"lost\"))"
            );
        }
    }

    /// #400: the placeholder says a session is viewed elsewhere only once
    /// another client took control from the monitor; a session it never
    /// made a terminal for, never controlled, or lost its connection to is
    /// only not controlled here.
    #[test]
    fn only_a_session_taken_over_is_viewed_elsewhere() {
        let mut parser = climon2_vt::Parser::new();
        assert_eq!(says(None), Placeholder::NotControlled);
        let mut terminal = Terminal::new(climon2_vt::Limits {
            bytes: 64 << 20,
            history_rows: 200,
        });
        assert_eq!(says(Some(&terminal)), Placeholder::NotControlled);
        let held = terminal::Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let geometry = climon2_vt::Geometry::for_host(10, 3).unwrap();
        terminal.present(&mut parser, Presentation::Granted(held, geometry));
        terminal.present(
            &mut parser,
            Presentation::Revoked(terminal::Revocation::Takeover),
        );
        assert_eq!(says(Some(&terminal)), Placeholder::TakenOver);
        terminal.present(&mut parser, Presentation::Ended);
        assert_eq!(says(Some(&terminal)), Placeholder::NotControlled);
    }

    /// REC-02 (pure supplement, #116): only a row neither connected nor
    /// known final shows a retained cache, and only once one exists; a
    /// final row keeps to the view frozen at its end (#108).
    #[test]
    fn only_a_row_neither_connected_nor_final_shows_its_retained_history() {
        let mut parser = climon2_vt::Parser::new();
        let mut terminal = Terminal::new(climon2_vt::Limits {
            bytes: 64 << 20,
            history_rows: 200,
        });
        assert!(retained(Kind::Retained, false, Some(&terminal)).is_none());
        let held = terminal::Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let geometry = climon2_vt::Geometry::for_host(10, 3).unwrap();
        terminal.present(&mut parser, Presentation::Granted(held, geometry));
        terminal.present(&mut parser, Presentation::Begin(held, geometry));
        terminal.present(&mut parser, Presentation::Complete);
        terminal.present(&mut parser, Presentation::Ended);
        let cases = [
            (Kind::Retained, false, true),
            (Kind::Unconnected, false, true),
            (Kind::Connected, false, false),
            (Kind::Retained, true, false),
            (Kind::Unconnected, true, false),
        ];
        for (kind, finished, expected) in cases {
            let shown = retained(kind, finished, Some(&terminal)).is_some();
            assert_eq!(shown, expected, "{kind:?}, finished {finished}");
        }
        assert!(retained(Kind::Retained, false, None).is_none());
    }

    /// A controlled terminal whose Viewing request was answered as never
    /// executed, its stream then ended if `ended`.
    fn cancelled(ended: bool) -> Terminal {
        let mut parser = climon2_vt::Parser::new();
        let mut terminal = Terminal::new(climon2_vt::Limits {
            bytes: 64 << 20,
            history_rows: 200,
        });
        let held = terminal::Authority {
            generation: 1,
            period: 1,
            restoration: 1,
        };
        let geometry = climon2_vt::Geometry::for_host(10, 3).unwrap();
        terminal.present(&mut parser, Presentation::Granted(held, geometry));
        terminal.present(&mut parser, Presentation::Begin(held, geometry));
        terminal.present(&mut parser, Presentation::Complete);
        let cancelled = "the request was accepted but never executed";
        let viewing = terminal::Request::Viewing(held, true);
        terminal.present(
            &mut parser,
            Presentation::Answered(viewing, Err(cancelled.into())),
        );
        if ended {
            terminal.present(&mut parser, Presentation::Ended);
        }
        terminal
    }

    /// REC-01 (Monitor: a gone daemon, pure supplement, #116): a request
    /// the stream's end cancelled while control was held leaves the
    /// terminal's own error, but once discovery proves the daemon gone the
    /// top row's error slot says `Session no longer exists`, as the
    /// overlay over its history does (owner decision on #116); a retry's
    /// reason never covers the terminal's own error.
    #[test]
    fn a_gone_session_says_so_in_its_error_slot_over_a_cancelled_request() {
        let row = View {
            id: SessionId::from_bytes([1; 16]),
            status: "Unavailable".into(),
            label: "Lost".into(),
            since: std::time::SystemTime::UNIX_EPOCH,
        };
        let error = |terminal: &Terminal, recovery| {
            let shown = Shown {
                kind: Kind::Unconnected,
                finished: false,
                recovery: Some(recovery),
                condition: None,
                refused: None,
            };
            selected(&row, shown, Some(terminal))
                .error
                .map(str::to_owned)
        };
        let retrying = Recovery::Retrying("transport io_error: reset".into());
        let gone = "Session no longer exists";
        // The connection's end also ended the terminal's errors, so the
        // cancelled request no longer shows either way.
        let ended = cancelled(true);
        assert_eq!(
            error(&ended, &retrying).as_deref(),
            Some("transport io_error: reset")
        );
        assert_eq!(error(&ended, &Recovery::Gone).as_deref(), Some(gone));
        let open = cancelled(false);
        let own = "the request was accepted but never executed";
        assert_eq!(error(&open, &retrying).as_deref(), Some(own));
        assert_eq!(error(&open, &Recovery::Gone).as_deref(), Some(gone));
    }
}
