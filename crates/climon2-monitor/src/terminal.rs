//! One session's terminal as the monitor keeps it (spec #73, "Terminal
//! models"; #108): its allocation-bounded `climon2-vt` model, kept across
//! generations, and the control the monitor holds over the session. Pure:
//! the client's presentation records are its inputs, and the requests the
//! monitor must submit (Ready after each applied restoration, Resize for the
//! selected controller) are its outputs. The model never answers queries
//! and nothing here writes to the host.
use climon2_core::protocol::TerminalSize;
use climon2_vt::{Geometry, Limits, Model, Parser, Rejected, view::View};

/// A control period's authority: the connection generation, period and
/// restoration ID the daemon judges each request by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Authority {
    pub(crate) generation: u64,
    pub(crate) period: u64,
    pub(crate) restoration: u64,
}

/// A request the monitor submits through the session's client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Request {
    /// Take control: explicit selection.
    Acquire,
    /// The restoration under this authority is applied.
    Ready(Authority),
    /// The selected controller's desired application area.
    Resize(Authority, TerminalSize),
    /// Application input for the ready controller, at most `chunk_bytes`
    /// once submitted. Refused, it carries the bytes that were not sent.
    Input(Authority, Vec<u8>),
    /// Part of a paste for the ready controller, submitted as input; once
    /// refused, the rest of its paste is discarded.
    Paste(Authority, Paste),
    /// The application's desired keyboard focus (`focus-v1`), under ready
    /// authority: non-user, never input.
    Focus(Authority, bool),
}

/// Part of a paste, as the application receives it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Paste {
    /// The application's paste markers go around the paste: it had
    /// bracketed paste on when the paste began.
    pub(crate) bracketed: bool,
    /// The paste begins with this part.
    pub(crate) starts: bool,
    pub(crate) bytes: Vec<u8>,
    /// The paste ends with this part.
    pub(crate) ends: bool,
}

/// What the session's client reported about control and display. The
/// client library checks that every record names the authority the role
/// holds, so only grants and beginnings carry theirs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Presentation {
    Granted(Authority, Geometry),
    Revoked,
    Begin(Authority, Geometry),
    Display(Vec<u8>),
    Complete,
    /// The effective session geometry changed.
    Geometry(Geometry),
    /// The command's final result is known: the view freezes here.
    Final,
    /// A request was answered: accepted, or why not.
    Answered(Request, Result<(), String>),
    /// The client opened with application focus (`focus-v1`) selected.
    Focusable,
    /// The client's connection ended.
    Ended,
}

/// How a refusal of `request` is shown: refused input says how much was not
/// sent, and that the input before it was accepted; a refused paste that
/// it was cut short (owner decision on #111).
fn refusal(request: &Request, error: String) -> String {
    match request {
        Request::Input(_, unsent) => format!(
            "Input not sent ({} bytes; earlier input was accepted): {error}",
            unsent.len()
        ),
        Request::Paste(..) => PASTE_TRUNCATED.to_owned(),
        _ => error,
    }
}

const PASTE_TRUNCATED: &str = "Paste truncated";

/// The control the monitor holds, as the top row names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Control {
    /// Acquire is submitted and not yet answered or granted.
    Acquiring,
    /// Control is held, but its restoration is not yet applied and ready.
    Restoring,
    /// Control is held with its restoration applied and Ready accepted,
    /// whether or not the canvas shows now; input additionally needs a
    /// painted frame ([`Terminal::input`]).
    Controlling,
    NotControlling,
    /// The command ended: its view is frozen.
    Ended,
}

impl Control {
    pub(crate) fn label(self) -> &'static str {
        LABELS[self as usize]
    }
}

/// Each [`Control`]'s label, in declaration order.
const LABELS: [&str; 5] = [
    "Acquiring",
    "Restoring",
    "Controlling",
    "Not controlling",
    "Ended",
];

/// Where the held control period's restoration stands. Applied, Ready
/// accepted and painted are distinct.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Phase {
    /// Granted; no restoration has begun yet.
    Granted,
    /// A restoration is staged; the current model, if one was applied in
    /// this period, is the last coherent frame.
    Restoring,
    Applied {
        ready: bool,
        painted: bool,
    },
    /// A Display arrived at a geometry the model does not have: it is never
    /// reflowed in place, only replaced by the next restoration.
    Stale,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    authority: Authority,
    /// The latest effective session geometry.
    effective: Geometry,
    phase: Phase,
    /// A restoration was applied in this period: the model is live.
    live: bool,
    /// The desire last sent in this period, if any.
    resized: Option<TerminalSize>,
}

impl Held {
    /// Controlling once the period's restoration is applied and its Ready
    /// accepted, painted or not; Restoring before that.
    fn control(self) -> Control {
        match self.phase {
            Phase::Applied { ready: true, .. } => Control::Controlling,
            _ => Control::Restoring,
        }
    }
}

/// One session's terminal: kept per session ID for the monitor's whole run.
pub(crate) struct Terminal {
    limits: Limits,
    model: Option<Model>,
    held: Option<Held>,
    acquiring: bool,
    /// Once the command ended (the model changes no more): whether its
    /// frozen view shows, which it does only if the control period held then
    /// had applied a restoration.
    finished: Option<bool>,
    /// The latest failure to show, until the next applied restoration.
    error: Option<String>,
    /// The client opened with `focus-v1`, until it ends.
    focusable: bool,
    /// The application focus last submitted, with the authority it was
    /// submitted under.
    focused: Option<(Authority, bool)>,
}

impl Terminal {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            model: None,
            held: None,
            acquiring: false,
            finished: None,
            error: None,
            focusable: false,
            focused: None,
        }
    }

    /// Explicit selection asks for control.
    pub(crate) fn acquire(&mut self) -> Request {
        self.acquiring = true;
        self.error = None;
        Request::Acquire
    }

    /// Applies one report, feeding Display through the monitor's one shared
    /// `parser` (#166); returns Ready once a restoration is applied.
    pub(crate) fn present(
        &mut self,
        parser: &mut Parser,
        presentation: Presentation,
    ) -> Option<Request> {
        match presentation {
            Presentation::Complete => return self.complete(),
            Presentation::Display(bytes) => self.display(parser, &bytes),
            other => self.reported(other),
        }
        None
    }

    /// An answer, `focus-v1` support, or a change of control.
    fn reported(&mut self, presentation: Presentation) {
        match presentation {
            Presentation::Answered(request, answer) => self.answered(request, answer),
            Presentation::Focusable => self.focusable = true,
            other => self.control_changed(other),
        }
    }

    /// A grant, a beginning, a new effective geometry or the end of control.
    fn control_changed(&mut self, presentation: Presentation) {
        match presentation {
            Presentation::Granted(authority, geometry) => self.granted(authority, geometry),
            Presentation::Begin(authority, geometry) => self.begin(authority, geometry),
            Presentation::Geometry(geometry) => self.reshaped(geometry),
            other => self.ended(&other),
        }
    }

    fn granted(&mut self, authority: Authority, effective: Geometry) {
        self.acquiring = false;
        self.held = Some(Held {
            authority,
            effective,
            phase: Phase::Granted,
            live: false,
            resized: None,
        });
    }

    /// Stages the restoration in a separate model; a newer beginning
    /// supersedes a staged one, which never becomes current.
    fn begin(&mut self, authority: Authority, geometry: Geometry) {
        let Some(held) = self.held.as_mut().filter(|_| self.finished.is_none()) else {
            return;
        };
        held.authority = authority;
        held.effective = geometry;
        held.phase = Phase::Restoring;
        let staged = self.model(geometry).and_then(|model| model.stage(geometry));
        self.rejected(staged);
    }

    /// The session's model, created blank at `geometry` the first time.
    fn model(&mut self, geometry: Geometry) -> Result<&mut Model, Rejected> {
        if self.model.is_none() {
            self.model = Some(Model::new(geometry, self.limits)?);
        }
        Ok(self.model.as_mut().expect("the model was just created"))
    }

    fn display(&mut self, parser: &mut Parser, bytes: &[u8]) {
        let Some(held) = self.held.as_mut().filter(|_| self.finished.is_none()) else {
            return;
        };
        let Some(model) = self.model.as_mut() else {
            return;
        };
        if held.phase != Phase::Restoring && model.geometry() != held.effective {
            held.phase = Phase::Stale;
        }
        if held.phase != Phase::Stale {
            let fed = model.feed(parser, bytes);
            self.rejected(fed);
        }
    }

    fn complete(&mut self) -> Option<Request> {
        let held = self.held.as_mut().filter(|_| self.finished.is_none())?;
        let model = self.model.as_mut()?;
        (held.phase == Phase::Restoring && model.apply()).then(|| {
            held.phase = Phase::Applied {
                ready: false,
                painted: false,
            };
            held.live = true;
            self.error = None;
            Request::Ready(held.authority)
        })
    }

    fn reshaped(&mut self, geometry: Geometry) {
        if let Some(held) = self.held.as_mut() {
            held.effective = geometry;
        }
    }

    /// An answer: Ready accepted, or a refusal shown as the error. A
    /// refusal of work under a superseded authority is expected after a race
    /// and not shown; a Resize refused so is sent again under the current one.
    fn answered(&mut self, request: Request, answer: Result<(), String>) {
        if request == Request::Acquire {
            self.acquiring = false;
        }
        match (answer, self.current(&request)) {
            (Ok(()), _) => self.accepted(&request),
            (Err(error), true) => self.error = Some(refusal(&request, error)),
            (Err(_), false) => self.superseded(&request),
        }
    }

    /// Whether `request`'s authority is the one held; Acquire has none.
    fn current(&self, request: &Request) -> bool {
        match *request {
            Request::Acquire => true,
            Request::Ready(authority)
            | Request::Resize(authority, _)
            | Request::Input(authority, _)
            | Request::Paste(authority, _)
            | Request::Focus(authority, _) => {
                self.held.is_some_and(|held| held.authority == authority)
            }
        }
    }

    fn superseded(&mut self, request: &Request) {
        if let (Request::Resize(..), Some(held)) = (request, self.held.as_mut()) {
            held.resized = None;
        }
    }

    /// Ready accepted for the restoration still applied.
    fn accepted(&mut self, request: &Request) {
        let Some(held) = self.held.as_mut() else {
            return;
        };
        if let (&Request::Ready(authority), Phase::Applied { painted, .. }) = (request, held.phase)
            && authority == held.authority
        {
            held.phase = Phase::Applied {
                ready: true,
                painted,
            };
        }
    }

    /// Revocation, the end of the connection or the command's final result:
    /// control, if held, is over, and a staged restoration is abandoned. The
    /// first final result freezes the live model's view, if there is one.
    fn ended(&mut self, presentation: &Presentation) {
        if *presentation == Presentation::Final {
            let live = self.held.is_some_and(|held| held.live);
            self.finished.get_or_insert(live);
        } else {
            self.held = None;
            self.acquiring = false;
            self.focusable &= *presentation != Presentation::Ended;
        }
        if let Some(model) = self.model.as_mut() {
            model.discard();
        }
    }

    fn rejected(&mut self, fed: Result<(), Rejected>) {
        if let Err(rejected) = fed {
            self.error = Some(format!(
                "terminal model over its limit: {} bytes needed, {} allowed",
                rejected.needed, rejected.limit
            ));
        }
    }

    /// Whether the frame now painted on the host shows this terminal's
    /// applied restoration: selected, uncovered and with room. Any other
    /// frame closes input until one shows it again.
    pub(crate) fn painted(&mut self, shown: bool) {
        if let Some(held) = self.held.as_mut()
            && let Phase::Applied { ready, .. } = held.phase
        {
            held.phase = Phase::Applied {
                ready,
                painted: shown,
            };
        }
    }

    /// The Resize to submit for `desire`, while this is the selected
    /// controller and the host has room: once per period and desire, so a
    /// grant (a new period) always gets one.
    pub(crate) fn resize(&mut self, desire: TerminalSize) -> Option<Request> {
        let held = self.held.as_mut().filter(|_| self.finished.is_none())?;
        (held.resized != Some(desire)).then(|| {
            held.resized = Some(desire);
            Request::Resize(held.authority, desire)
        })
    }

    /// The authority input may use: control held, its restoration applied,
    /// Ready accepted and painted, and the model current.
    pub(crate) fn input(&self) -> Option<Authority> {
        let held = self.held.filter(|_| self.finished.is_none())?;
        let ready = Phase::Applied {
            ready: true,
            painted: true,
        };
        (held.phase == ready).then_some(held.authority)
    }

    /// The Focus to submit so the application's focus is `desired`: only
    /// with `focus-v1`, under the authority of a ready restoration, once per
    /// authority and desire. A newly ready restoration so submits the
    /// current desire once, never an earlier one.
    pub(crate) fn focus(&mut self, desired: bool) -> Option<Request> {
        let held = self
            .held
            .filter(|_| self.focusable && self.finished.is_none())?;
        let wanted = (held.authority, desired);
        let ready = matches!(held.phase, Phase::Applied { ready: true, .. });
        (ready && self.focused != Some(wanted)).then(|| {
            self.focused = Some(wanted);
            Request::Focus(held.authority, desired)
        })
    }

    /// The authority of the control period and restoration held, if any:
    /// it changes with every grant, restoration, revocation and end.
    pub(crate) fn authority(&self) -> Option<Authority> {
        self.held.map(|held| held.authority)
    }

    /// The model's geometry, once it has one.
    pub(crate) fn geometry(&self) -> Option<Geometry> {
        self.model.as_ref().map(Model::geometry)
    }

    pub(crate) fn control(&self) -> Control {
        if self.finished.is_some() {
            return Control::Ended;
        }
        match self.held {
            Some(held) => held.control(),
            None if self.acquiring => Control::Acquiring,
            None => Control::NotControlling,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// What to paint: the live model while control is held and a restoration
    /// was applied in this period (the last coherent frame while a newer one
    /// is staged), or the view frozen at the command's end if it was live
    /// then. A cache from an earlier period is never shown.
    pub(crate) fn view(&self) -> Option<View<'_>> {
        let live = self.held.is_some_and(|held| held.live);
        let shown = self.finished.unwrap_or(live);
        self.model.as_ref().filter(|_| shown).map(Model::view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: Limits = Limits {
        bytes: 64 << 20,
        history_rows: 1000,
    };

    fn authority(period: u64, restoration: u64) -> Authority {
        Authority {
            generation: 1,
            period,
            restoration,
        }
    }

    fn geometry(cols: u16, rows: u16) -> Geometry {
        Geometry::for_host(cols, rows).unwrap()
    }

    fn screen(terminal: &Terminal) -> Option<Vec<String>> {
        terminal.view().map(|view| {
            let climon2_vt::view::Screen::Normal { rows, .. } = view.screen() else {
                panic!("the normal screen");
            };
            rows.iter()
                .map(|row| {
                    let text: String = row
                        .cells()
                        .map(|cell| match cell.content {
                            climon2_vt::view::Content::Narrow(glyph)
                            | climon2_vt::view::Content::Wide(glyph) => glyph.character,
                            climon2_vt::view::Content::Spacer => ' ',
                        })
                        .collect();
                    text.trim_end().to_owned()
                })
                .collect()
        })
    }

    /// A terminal granted period 2 at 10x3 that applied restoration 3
    /// holding `text`; Ready for it has been returned.
    fn restored(text: &[u8]) -> Terminal {
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(LIMITS);
        let held = authority(2, 3);
        assert_eq!(terminal.acquire(), Request::Acquire);
        terminal.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(text.to_vec()));
        assert_eq!(
            terminal.present(&mut parser, Presentation::Complete),
            Some(Request::Ready(held))
        );
        terminal
    }

    /// CTL-05 and COM-05 (pure supplement): Ready follows only the applied
    /// restoration; input additionally needs Ready accepted and a painted
    /// frame, in either order, and each of the three alone is not enough.
    #[test]
    fn ready_applied_and_painted_stay_distinct_and_input_needs_all_three() {
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(LIMITS);
        let held = authority(2, 3);
        terminal.acquire();
        assert_eq!(terminal.control(), Control::Acquiring);
        terminal.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        assert_eq!(terminal.control(), Control::Restoring);
        terminal.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(b"hi".to_vec()));
        terminal.painted(true);
        assert_eq!(terminal.input(), None);
        assert_eq!(
            terminal.present(&mut parser, Presentation::Complete),
            Some(Request::Ready(held))
        );
        assert_eq!(terminal.input(), None, "applied alone");
        let ready = Presentation::Answered(Request::Ready(held), Ok(()));
        terminal.present(&mut parser, ready.clone());
        assert_eq!(terminal.input(), None, "applied and ready, not painted");
        terminal.painted(true);
        assert_eq!(terminal.input(), Some(held));
        assert_eq!(terminal.control(), Control::Controlling);

        let mut painted_first = restored(b"hi");
        painted_first.painted(true);
        assert_eq!(
            painted_first.input(),
            None,
            "applied and painted, not ready"
        );
        painted_first.present(&mut parser, ready);
        assert_eq!(painted_first.input(), Some(held));
    }

    /// CTL-05 (pure supplement): a Ready refused, or accepted for another
    /// restoration, never opens input, and Complete without a beginning
    /// sends no Ready.
    #[test]
    fn only_ready_accepted_for_the_applied_restoration_opens_input() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut refused = restored(b"x");
        refused.painted(true);
        refused.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Err("stale".into())),
        );
        assert_eq!(refused.input(), None);
        assert_eq!(refused.error(), Some("stale"));
        let mut other = restored(b"x");
        other.painted(true);
        other.present(
            &mut parser,
            Presentation::Answered(Request::Ready(authority(2, 4)), Ok(())),
        );
        assert_eq!(other.input(), None);
        let mut granted = Terminal::new(LIMITS);
        granted.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        assert_eq!(granted.present(&mut parser, Presentation::Complete), None);
        let mut idle = Terminal::new(LIMITS);
        idle.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        assert_eq!(idle.input(), None);
    }

    /// TRM-04 and COM-05 (pure supplement): a staged restoration never shows
    /// while it builds; the last coherent frame does, and the new one
    /// replaces it whole once applied. A superseded restoration never becomes
    /// current, and a new beginning closes input until its own Ready.
    #[test]
    fn a_restoration_replaces_the_model_whole_only_once_applied() {
        let mut parser = Parser::new();
        let mut terminal = restored(b"old\r\nrows");
        let ready = authority(2, 3);
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(ready), Ok(())),
        );
        terminal.painted(true);
        let newer = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(newer, geometry(10, 3)));
        assert_eq!(terminal.input(), None);
        terminal.present(&mut parser, Presentation::Display(b"superseded".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["old", "rows", ""]);
        let newest = authority(2, 5);
        terminal.present(&mut parser, Presentation::Begin(newest, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(b"new".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["old", "rows", ""]);
        assert_eq!(
            terminal.present(&mut parser, Presentation::Complete),
            Some(Request::Ready(newest))
        );
        assert_eq!(screen(&terminal).unwrap(), ["new", "", ""]);
    }

    /// Carried from #76 (owner decision, 2026-11-13): a Display at a
    /// geometry the model does not have marks the model stale instead of
    /// reflowing it; it keeps its frame, closes input and takes no more
    /// bytes until the next restoration replaces it. At the model's own
    /// geometry, live bytes apply.
    #[test]
    fn a_display_at_a_geometry_the_model_lacks_marks_it_stale_until_restored() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"kept");
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        terminal.painted(true);
        terminal.present(&mut parser, Presentation::Geometry(geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(b"!".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["kept!", "", ""]);
        assert_eq!(terminal.input(), Some(held));
        terminal.present(&mut parser, Presentation::Geometry(geometry(12, 3)));
        terminal.present(&mut parser, Presentation::Display(b"lost".to_vec()));
        terminal.present(&mut parser, Presentation::Geometry(geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(b"also".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["kept!", "", ""]);
        assert_eq!(terminal.input(), None);
        assert_eq!(terminal.control(), Control::Restoring);
        terminal.present(&mut parser, Presentation::Complete);
        assert_eq!(screen(&terminal).unwrap(), ["kept!", "", ""]);
        let next = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(next, geometry(12, 3)));
        terminal.present(&mut parser, Presentation::Display(b"fresh".to_vec()));
        assert_eq!(
            terminal.present(&mut parser, Presentation::Complete),
            Some(Request::Ready(next))
        );
        assert_eq!(screen(&terminal).unwrap(), ["fresh", "", ""]);
    }

    /// Enter "never exposing cached output": once control is revoked or the
    /// connection ends, the model is a cache and is not shown, and a new
    /// grant shows nothing until its own restoration is applied.
    #[test]
    fn a_cache_from_an_earlier_period_is_never_shown() {
        let mut parser = Parser::new();
        for end in [Presentation::Revoked, Presentation::Ended] {
            let mut terminal = restored(b"cached");
            assert!(terminal.view().is_some());
            terminal.present(&mut parser, end);
            assert!(terminal.view().is_none());
            assert_eq!(terminal.control(), Control::NotControlling);
            terminal.present(&mut parser, Presentation::Display(b"x".to_vec()));
            assert_eq!(terminal.present(&mut parser, Presentation::Complete), None);
            terminal.acquire();
            let again = authority(5, 6);
            terminal.present(&mut parser, Presentation::Granted(again, geometry(10, 3)));
            terminal.present(&mut parser, Presentation::Begin(again, geometry(10, 3)));
            assert!(terminal.view().is_none());
            terminal.present(&mut parser, Presentation::Display(b"live".to_vec()));
            assert_eq!(
                terminal.present(&mut parser, Presentation::Complete),
                Some(Request::Ready(again))
            );
            assert_eq!(screen(&terminal).unwrap(), ["live", "", ""]);
        }
    }

    /// #94 and the #89 notes: a finished session's view freezes at its
    /// Command final: the last output and the release that follow change
    /// nothing, a staged restoration is abandoned, input closes, and the
    /// frozen view still shows after revocation.
    #[test]
    fn the_view_freezes_at_the_command_final() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"done");
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        terminal.painted(true);
        terminal.present(&mut parser, Presentation::Final);
        assert_eq!(terminal.input(), None);
        assert_eq!(terminal.control(), Control::Ended);
        terminal.present(
            &mut parser,
            Presentation::Display(b"\x1b[2Jreleased".to_vec()),
        );
        terminal.present(
            &mut parser,
            Presentation::Begin(authority(2, 4), geometry(10, 3)),
        );
        assert_eq!(terminal.present(&mut parser, Presentation::Complete), None);
        terminal.present(&mut parser, Presentation::Revoked);
        terminal.present(&mut parser, Presentation::Final);
        assert_eq!(screen(&terminal).unwrap(), ["done", "", ""]);
        assert_eq!(terminal.control(), Control::Ended);

        let mut staged = restored(b"seen");
        staged.present(
            &mut parser,
            Presentation::Begin(authority(2, 4), geometry(10, 3)),
        );
        staged.present(&mut parser, Presentation::Display(b"unseen".to_vec()));
        staged.present(&mut parser, Presentation::Final);
        assert_eq!(staged.present(&mut parser, Presentation::Complete), None);
        assert_eq!(screen(&staged).unwrap(), ["seen", "", ""]);
    }

    /// The owner's final-view rule (2026-11-14) with terminal-integration.md
    /// ("a model from an earlier control period is never shown"): a Command
    /// final after revocation, or while the period's first restoration is
    /// still staged, freezes no view: neither the earlier period's cache nor
    /// a blank model made up for the staging is shown.
    #[test]
    fn a_command_final_without_a_live_model_shows_no_view() {
        let mut parser = Parser::new();
        let mut revoked = restored(b"cached");
        revoked.present(&mut parser, Presentation::Revoked);
        revoked.present(&mut parser, Presentation::Final);
        assert!(revoked.view().is_none());
        assert_eq!(revoked.control(), Control::Ended);

        let mut first = Terminal::new(LIMITS);
        let held = authority(2, 3);
        first.acquire();
        first.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        first.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        first.present(&mut parser, Presentation::Display(b"partial".to_vec()));
        first.present(&mut parser, Presentation::Final);
        assert_eq!(first.present(&mut parser, Presentation::Complete), None);
        assert!(first.view().is_none());

        let mut again = restored(b"cached");
        again.present(&mut parser, Presentation::Revoked);
        again.acquire();
        let next = authority(4, 5);
        again.present(&mut parser, Presentation::Granted(next, geometry(10, 3)));
        again.present(&mut parser, Presentation::Begin(next, geometry(10, 3)));
        again.present(&mut parser, Presentation::Display(b"partial".to_vec()));
        again.present(&mut parser, Presentation::Final);
        again.present(&mut parser, Presentation::Revoked);
        assert!(again.view().is_none());
    }

    /// Review of 8582801 (painted is not sticky): input needs the frame on
    /// the host to show the applied restoration now. A frame that does not
    /// (covered, too small, or another session selected) closes input until
    /// a frame shows it again.
    #[test]
    fn a_frame_that_stops_showing_the_restoration_closes_input() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        terminal.painted(true);
        assert_eq!(terminal.input(), Some(held));
        terminal.painted(false);
        assert_eq!(terminal.input(), None);
        terminal.painted(true);
        assert_eq!(terminal.input(), Some(held));
    }

    /// Owner decision (2026-11-14, follow-up): the control label follows
    /// control and restoration only, never visibility. Controlling once the
    /// period's restoration is applied and its Ready accepted, painted or
    /// not; Restoring before that, whatever the frame shows.
    #[test]
    fn the_control_label_ignores_whether_the_canvas_shows() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        terminal.painted(true);
        assert_eq!(terminal.control(), Control::Restoring, "Ready unanswered");
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        for shown in [false, true, false] {
            terminal.painted(shown);
            assert_eq!(terminal.control(), Control::Controlling, "shown {shown}");
        }
        terminal.present(
            &mut parser,
            Presentation::Begin(authority(2, 4), geometry(10, 3)),
        );
        assert_eq!(terminal.control(), Control::Restoring);
    }

    /// CTL-05 (pure supplement): a finished session is sent no Resize.
    #[test]
    fn a_finished_session_is_sent_no_resize() {
        let mut parser = Parser::new();
        let mut terminal = restored(b"x");
        terminal.present(&mut parser, Presentation::Final);
        assert_eq!(terminal.resize(TerminalSize { cols: 80, rows: 23 }), None);
    }

    /// CTL-05 (pure supplement): Resize goes once per grant and desire,
    /// under the held authority, and never without control.
    #[test]
    fn resize_follows_every_grant_and_each_new_desire_only_while_controlling() {
        let mut parser = Parser::new();
        let desire = TerminalSize { cols: 80, rows: 23 };
        let mut terminal = Terminal::new(LIMITS);
        assert_eq!(terminal.resize(desire), None);
        let first = authority(2, 3);
        terminal.present(&mut parser, Presentation::Granted(first, geometry(10, 3)));
        assert_eq!(
            terminal.resize(desire),
            Some(Request::Resize(first, desire))
        );
        assert_eq!(terminal.resize(desire), None);
        let wider = TerminalSize { cols: 81, rows: 23 };
        assert_eq!(terminal.resize(wider), Some(Request::Resize(first, wider)));
        let second = authority(4, 5);
        terminal.present(&mut parser, Presentation::Granted(second, geometry(10, 3)));
        assert_eq!(terminal.resize(wider), Some(Request::Resize(second, wider)));
        terminal.present(&mut parser, Presentation::Revoked);
        assert_eq!(terminal.resize(desire), None);
    }

    /// Acquiring lasts until the grant or Acquire's answer; a refusal is
    /// shown as the error until the next selection or applied restoration.
    #[test]
    fn acquiring_ends_at_the_grant_or_the_answer_and_errors_are_shown() {
        let mut parser = Parser::new();
        let mut refused = Terminal::new(LIMITS);
        refused.acquire();
        refused.present(
            &mut parser,
            Presentation::Answered(Request::Acquire, Err("unsupported".into())),
        );
        assert_eq!(refused.control(), Control::NotControlling);
        assert_eq!(refused.error(), Some("unsupported"));
        refused.acquire();
        assert_eq!(refused.error(), None);
        refused.present(
            &mut parser,
            Presentation::Answered(Request::Acquire, Ok(())),
        );
        assert_eq!(refused.control(), Control::NotControlling);
        let mut resized = restored(b"x");
        resized.present(
            &mut parser,
            Presentation::Answered(
                Request::Resize(authority(2, 3), TerminalSize { cols: 5, rows: 5 }),
                Err("geometry".into()),
            ),
        );
        assert_eq!(resized.error(), Some("geometry"));
        resized.present(
            &mut parser,
            Presentation::Begin(authority(2, 4), geometry(10, 3)),
        );
        resized.present(&mut parser, Presentation::Complete);
        assert_eq!(resized.error(), None);
    }

    /// CTL-05 and TRM-04 (pure supplement): Ready or Resize refused under a
    /// restoration since superseded is not an error, and the refused desire
    /// is sent again under the current authority; one refused under the
    /// current authority is shown and not resent.
    #[test]
    fn work_refused_under_a_superseded_restoration_is_not_an_error_and_resize_is_resent() {
        let mut parser = Parser::new();
        let old = authority(2, 3);
        let desire = TerminalSize { cols: 90, rows: 34 };
        let mut terminal = restored(b"x");
        assert_eq!(terminal.resize(desire), Some(Request::Resize(old, desire)));
        let new = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(new, geometry(10, 3)));
        assert_eq!(terminal.resize(desire), None);
        for refused in [Request::Ready(old), Request::Resize(old, desire)] {
            terminal.present(
                &mut parser,
                Presentation::Answered(refused, Err("stale".into())),
            );
        }
        assert_eq!(terminal.error(), None);
        assert_eq!(terminal.resize(desire), Some(Request::Resize(new, desire)));
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Resize(new, desire), Err("invalid".into())),
        );
        assert_eq!(terminal.error(), Some("invalid"));
        assert_eq!(terminal.resize(desire), None);
        terminal.present(&mut parser, Presentation::Revoked);
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Resize(new, desire), Err("late".into())),
        );
        assert_eq!(terminal.error(), Some("invalid"));
    }

    /// COM-03 queue-full and COM-04 typed cases (pure supplement): input
    /// refused under the current authority is shown with how many bytes
    /// were not sent, and that the input before them was accepted; input
    /// refused under a superseded authority is not an error.
    #[test]
    fn refused_input_shows_what_was_not_sent() {
        let mut parser = Parser::new();
        let mut terminal = restored(b"x");
        terminal.present(
            &mut parser,
            Presentation::Answered(
                Request::Input(authority(2, 2), b"old".to_vec()),
                Err("stale".into()),
            ),
        );
        assert_eq!(terminal.error(), None);
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Input(authority(2, 3), b"abc".to_vec()), Ok(())),
        );
        assert_eq!(terminal.error(), None);
        terminal.present(
            &mut parser,
            Presentation::Answered(
                Request::Input(authority(2, 3), b"def".to_vec()),
                Err("full: the request does not fit".into()),
            ),
        );
        assert_eq!(
            terminal.error(),
            Some(
                "Input not sent (3 bytes; earlier input was accepted): full: the request does not fit"
            )
        );
    }

    /// INP-03 and COM-04 (pure supplement; owner decision on #111): a
    /// refused part of a paste under the current authority shows that the
    /// paste was truncated; one refused under a superseded authority is not
    /// an error, and neither is an accepted one.
    #[test]
    fn a_refused_paste_shows_that_it_was_truncated() {
        let mut parser = Parser::new();
        let mut terminal = restored(b"x");
        let part = Paste {
            bracketed: true,
            starts: false,
            bytes: b"rest".to_vec(),
            ends: true,
        };
        for (held, answer) in [
            (authority(2, 2), Err("stale".to_owned())),
            (authority(2, 3), Ok(())),
        ] {
            let refused = Request::Paste(held, part.clone());
            terminal.present(&mut parser, Presentation::Answered(refused, answer));
            assert_eq!(terminal.error(), None);
        }
        let refused = Request::Paste(authority(2, 3), part);
        let answer = Err("full: the request does not fit".to_owned());
        terminal.present(&mut parser, Presentation::Answered(refused, answer));
        assert_eq!(terminal.error(), Some("Paste truncated"));
    }

    /// FOC-01 (pure supplement): with `focus-v1`, Focus is submitted under
    /// the ready restoration's authority for each new desire, never twice
    /// for the same one; a newly ready restoration submits the current
    /// desire once. Nothing is submitted before Ready is accepted, without
    /// control, once the command ended, or without `focus-v1`; a refusal
    /// under the current authority is shown, one under a superseded
    /// authority is not.
    #[test]
    fn focus_follows_each_new_desire_under_ready_authority_once() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        terminal.present(&mut parser, Presentation::Focusable);
        assert_eq!(terminal.focus(true), None);
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        assert_eq!(terminal.focus(true), Some(Request::Focus(held, true)));
        assert_eq!(terminal.focus(true), None);
        assert_eq!(terminal.focus(false), Some(Request::Focus(held, false)));
        assert_eq!(terminal.focus(false), None);
        let refused = |request, error: &str| Presentation::Answered(request, Err(error.into()));
        terminal.present(
            &mut parser,
            refused(Request::Focus(authority(2, 2), true), "stale"),
        );
        assert_eq!(terminal.error(), None);
        terminal.present(&mut parser, refused(Request::Focus(held, false), "refused"));
        assert_eq!(terminal.error(), Some("refused"));
        let newer = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(newer, geometry(10, 3)));
        assert_eq!(terminal.focus(false), None);
        terminal.present(&mut parser, Presentation::Complete);
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Ready(newer), Ok(())),
        );
        assert_eq!(terminal.focus(false), Some(Request::Focus(newer, false)));
        assert_eq!(terminal.focus(false), None);
        terminal.present(&mut parser, Presentation::Final);
        assert_eq!(terminal.focus(true), None);
        let mut unsupported = restored(b"x");
        unsupported.present(
            &mut parser,
            Presentation::Answered(Request::Ready(held), Ok(())),
        );
        assert_eq!(unsupported.focus(true), None);
    }

    /// Grants `held` with an applied restoration whose Ready was accepted.
    fn ready_under(terminal: &mut Terminal, parser: &mut Parser, held: Authority) {
        terminal.present(parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(parser, Presentation::Complete);
        terminal.present(parser, Presentation::Answered(Request::Ready(held), Ok(())));
    }

    /// FOC-02 (pure supplement): `focus-v1` lasts while the client does:
    /// after revocation, the next grant's ready restoration is still sent
    /// Focus; once the client ended, a later grant is not.
    #[test]
    fn focus_support_lasts_until_the_client_ends() {
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(LIMITS);
        terminal.present(&mut parser, Presentation::Focusable);
        terminal.present(&mut parser, Presentation::Revoked);
        assert_eq!(terminal.focus(true), None);
        let again = authority(3, 1);
        ready_under(&mut terminal, &mut parser, again);
        assert_eq!(terminal.focus(true), Some(Request::Focus(again, true)));
        terminal.present(&mut parser, Presentation::Ended);
        let later = authority(4, 1);
        ready_under(&mut terminal, &mut parser, later);
        assert_eq!(terminal.focus(true), None);
    }

    /// The top row names each control as #108 shows it.
    #[test]
    fn each_control_has_its_label() {
        let labels = [
            Control::Acquiring,
            Control::Restoring,
            Control::Controlling,
            Control::NotControlling,
            Control::Ended,
        ]
        .map(Control::label);
        assert_eq!(
            labels,
            [
                "Acquiring",
                "Restoring",
                "Controlling",
                "Not controlling",
                "Ended"
            ]
        );
    }

    /// LIM-03 (pure supplement): a restoration the model's limit refuses
    /// is shown as an error and sends no Ready; neither does output the
    /// limit refuses.
    #[test]
    fn a_restoration_over_the_model_limit_is_an_error_without_ready() {
        let mut parser = Parser::new();
        let tight = Limits {
            bytes: 1,
            history_rows: 1000,
        };
        let held = authority(2, 3);
        let mut terminal = Terminal::new(tight);
        terminal.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        assert!(
            terminal
                .error()
                .unwrap()
                .starts_with("terminal model over its limit")
        );
        assert_eq!(terminal.present(&mut parser, Presentation::Complete), None);
        assert!(terminal.view().is_none());
    }
}
