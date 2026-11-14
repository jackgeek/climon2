//! One session's terminal as the monitor keeps it (spec #73, "Terminal
//! models"; #108): its allocation-bounded `climon2-vt` model, kept across
//! generations, and the control the monitor holds over the session. Pure:
//! the client's presentation records are its inputs, and the requests the
//! monitor must submit (Ready after each applied restoration, Resize for the
//! selected controller) are its outputs. The model never answers queries
//! and nothing here writes to the host.
use climon2_core::protocol::TerminalSize;
use climon2_vt::{Geometry, Limits, Model, Rejected, view::View};

/// A control period's authority: the connection generation, period and
/// restoration ID the daemon judges each request by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Authority {
    pub(crate) generation: u64,
    pub(crate) period: u64,
    pub(crate) restoration: u64,
}

/// A request the monitor submits through the session's client.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Request {
    /// Take control: explicit selection.
    Acquire,
    /// The restoration under this authority is applied.
    Ready(Authority),
    /// The selected controller's desired application area.
    Resize(Authority, TerminalSize),
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
    /// The client's connection ended.
    Ended,
}

/// The control the monitor holds, as the top row names it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Control {
    /// Acquire is submitted and not yet answered or granted.
    Acquiring,
    /// Control is held, but input would not reach a painted, ready frame.
    Restoring,
    /// Control is held with a ready restoration painted: input may go.
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

/// One session's terminal: kept per session ID for the monitor's whole run.
pub(crate) struct Terminal {
    limits: Limits,
    model: Option<Model>,
    held: Option<Held>,
    acquiring: bool,
    /// The command ended: the model changes no more.
    frozen: bool,
    /// The latest failure to show, until the next applied restoration.
    error: Option<String>,
}

impl Terminal {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            model: None,
            held: None,
            acquiring: false,
            frozen: false,
            error: None,
        }
    }

    /// Explicit selection asks for control.
    pub(crate) fn acquire(&mut self) -> Request {
        self.acquiring = true;
        self.error = None;
        Request::Acquire
    }

    /// Applies one report; returns Ready once a restoration is applied.
    pub(crate) fn present(&mut self, presentation: Presentation) -> Option<Request> {
        match presentation {
            Presentation::Complete => return self.complete(),
            Presentation::Display(bytes) => self.display(&bytes),
            Presentation::Answered(request, answer) => self.answered(request, answer),
            other => self.control_changed(other),
        }
        None
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
        let Some(held) = self.held.as_mut().filter(|_| !self.frozen) else {
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

    fn display(&mut self, bytes: &[u8]) {
        let Some(held) = self.held.as_mut().filter(|_| !self.frozen) else {
            return;
        };
        let Some(model) = self.model.as_mut() else {
            return;
        };
        if held.phase != Phase::Restoring && model.geometry() != held.effective {
            held.phase = Phase::Stale;
        }
        if held.phase != Phase::Stale {
            let fed = model.feed(bytes);
            self.rejected(fed);
        }
    }

    fn complete(&mut self) -> Option<Request> {
        let held = self.held.as_mut().filter(|_| !self.frozen)?;
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
        match (answer, self.current(request)) {
            (Ok(()), _) => self.accepted(request),
            (Err(error), true) => self.error = Some(error),
            (Err(_), false) => self.superseded(request),
        }
    }

    /// Whether `request`'s authority is the one held; Acquire has none.
    fn current(&self, request: Request) -> bool {
        match request {
            Request::Acquire => true,
            Request::Ready(authority) | Request::Resize(authority, _) => {
                self.held.is_some_and(|held| held.authority == authority)
            }
        }
    }

    fn superseded(&mut self, request: Request) {
        if let (Request::Resize(..), Some(held)) = (request, self.held.as_mut()) {
            held.resized = None;
        }
    }

    /// Ready accepted for the restoration still applied.
    fn accepted(&mut self, request: Request) {
        let Some(held) = self.held.as_mut() else {
            return;
        };
        if let (Request::Ready(authority), Phase::Applied { painted, .. }) = (request, held.phase)
            && authority == held.authority
        {
            held.phase = Phase::Applied {
                ready: true,
                painted,
            };
        }
    }

    /// Revocation, the end of the connection or the command's final result:
    /// control, if held, is over, and a staged restoration is abandoned.
    fn ended(&mut self, presentation: &Presentation) {
        if *presentation == Presentation::Final {
            self.frozen = true;
        } else {
            self.held = None;
            self.acquiring = false;
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

    /// The frame just painted showed the applied restoration.
    pub(crate) fn painted(&mut self) {
        if let Some(held) = self.held.as_mut()
            && let Phase::Applied { ready, .. } = held.phase
        {
            held.phase = Phase::Applied {
                ready,
                painted: true,
            };
        }
    }

    /// The Resize to submit for `desire`, while this is the selected
    /// controller and the host has room: once per period and desire, so a
    /// grant (a new period) always gets one.
    pub(crate) fn resize(&mut self, desire: TerminalSize) -> Option<Request> {
        let held = self.held.as_mut()?;
        (held.resized != Some(desire)).then(|| {
            held.resized = Some(desire);
            Request::Resize(held.authority, desire)
        })
    }

    /// The authority input may use: control held, its restoration applied,
    /// Ready accepted and painted, and the model current.
    pub(crate) fn input(&self) -> Option<Authority> {
        let held = self.held.filter(|_| !self.frozen)?;
        let ready = Phase::Applied {
            ready: true,
            painted: true,
        };
        (held.phase == ready).then_some(held.authority)
    }

    pub(crate) fn control(&self) -> Control {
        if self.frozen {
            return Control::Ended;
        }
        match self.held {
            Some(_) => self
                .input()
                .map_or(Control::Restoring, |_| Control::Controlling),
            None if self.acquiring => Control::Acquiring,
            None => Control::NotControlling,
        }
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// What to paint: the live model while control is held and a restoration
    /// was applied in this period (the last coherent frame while a newer one
    /// is staged), or the view frozen at the command's end. A cache from an
    /// earlier period is never shown.
    pub(crate) fn view(&self) -> Option<View<'_>> {
        let live = self.held.is_some_and(|held| held.live);
        self.model
            .as_ref()
            .filter(|_| live || self.frozen)
            .map(Model::view)
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
        let mut terminal = Terminal::new(LIMITS);
        let held = authority(2, 3);
        assert_eq!(terminal.acquire(), Request::Acquire);
        terminal.present(Presentation::Granted(held, geometry(10, 3)));
        terminal.present(Presentation::Begin(held, geometry(10, 3)));
        terminal.present(Presentation::Display(text.to_vec()));
        assert_eq!(
            terminal.present(Presentation::Complete),
            Some(Request::Ready(held))
        );
        terminal
    }

    /// CTL-05 and COM-05 (pure supplement): Ready follows only the applied
    /// restoration; input additionally needs Ready accepted and a painted
    /// frame, in either order, and each of the three alone is not enough.
    #[test]
    fn ready_applied_and_painted_stay_distinct_and_input_needs_all_three() {
        let mut terminal = Terminal::new(LIMITS);
        let held = authority(2, 3);
        terminal.acquire();
        assert_eq!(terminal.control(), Control::Acquiring);
        terminal.present(Presentation::Granted(held, geometry(10, 3)));
        assert_eq!(terminal.control(), Control::Restoring);
        terminal.present(Presentation::Begin(held, geometry(10, 3)));
        terminal.present(Presentation::Display(b"hi".to_vec()));
        terminal.painted();
        assert_eq!(terminal.input(), None);
        assert_eq!(
            terminal.present(Presentation::Complete),
            Some(Request::Ready(held))
        );
        assert_eq!(terminal.input(), None, "applied alone");
        let ready = Presentation::Answered(Request::Ready(held), Ok(()));
        terminal.present(ready.clone());
        assert_eq!(terminal.input(), None, "applied and ready, not painted");
        terminal.painted();
        assert_eq!(terminal.input(), Some(held));
        assert_eq!(terminal.control(), Control::Controlling);

        let mut painted_first = restored(b"hi");
        painted_first.painted();
        assert_eq!(
            painted_first.input(),
            None,
            "applied and painted, not ready"
        );
        painted_first.present(ready);
        assert_eq!(painted_first.input(), Some(held));
    }

    /// CTL-05 (pure supplement): a Ready refused, or accepted for another
    /// restoration, never opens input, and Complete without a beginning
    /// sends no Ready.
    #[test]
    fn only_ready_accepted_for_the_applied_restoration_opens_input() {
        let held = authority(2, 3);
        let mut refused = restored(b"x");
        refused.painted();
        refused.present(Presentation::Answered(
            Request::Ready(held),
            Err("stale".into()),
        ));
        assert_eq!(refused.input(), None);
        assert_eq!(refused.error(), Some("stale"));
        let mut other = restored(b"x");
        other.painted();
        other.present(Presentation::Answered(
            Request::Ready(authority(2, 4)),
            Ok(()),
        ));
        assert_eq!(other.input(), None);
        let mut granted = Terminal::new(LIMITS);
        granted.present(Presentation::Granted(held, geometry(10, 3)));
        assert_eq!(granted.present(Presentation::Complete), None);
        let mut idle = Terminal::new(LIMITS);
        idle.present(Presentation::Answered(Request::Ready(held), Ok(())));
        assert_eq!(idle.input(), None);
    }

    /// TRM-04 and COM-05 (pure supplement): a staged restoration never shows
    /// while it builds; the last coherent frame does, and the new one
    /// replaces it whole once applied. A superseded restoration never becomes
    /// current, and a new beginning closes input until its own Ready.
    #[test]
    fn a_restoration_replaces_the_model_whole_only_once_applied() {
        let mut terminal = restored(b"old\r\nrows");
        let ready = authority(2, 3);
        terminal.present(Presentation::Answered(Request::Ready(ready), Ok(())));
        terminal.painted();
        let newer = authority(2, 4);
        terminal.present(Presentation::Begin(newer, geometry(10, 3)));
        assert_eq!(terminal.input(), None);
        terminal.present(Presentation::Display(b"superseded".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["old", "rows", ""]);
        let newest = authority(2, 5);
        terminal.present(Presentation::Begin(newest, geometry(10, 3)));
        terminal.present(Presentation::Display(b"new".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["old", "rows", ""]);
        assert_eq!(
            terminal.present(Presentation::Complete),
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
        let held = authority(2, 3);
        let mut terminal = restored(b"kept");
        terminal.present(Presentation::Answered(Request::Ready(held), Ok(())));
        terminal.painted();
        terminal.present(Presentation::Geometry(geometry(10, 3)));
        terminal.present(Presentation::Display(b"!".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["kept!", "", ""]);
        assert_eq!(terminal.input(), Some(held));
        terminal.present(Presentation::Geometry(geometry(12, 3)));
        terminal.present(Presentation::Display(b"lost".to_vec()));
        terminal.present(Presentation::Geometry(geometry(10, 3)));
        terminal.present(Presentation::Display(b"also".to_vec()));
        assert_eq!(screen(&terminal).unwrap(), ["kept!", "", ""]);
        assert_eq!(terminal.input(), None);
        assert_eq!(terminal.control(), Control::Restoring);
        terminal.present(Presentation::Complete);
        assert_eq!(screen(&terminal).unwrap(), ["kept!", "", ""]);
        let next = authority(2, 4);
        terminal.present(Presentation::Begin(next, geometry(12, 3)));
        terminal.present(Presentation::Display(b"fresh".to_vec()));
        assert_eq!(
            terminal.present(Presentation::Complete),
            Some(Request::Ready(next))
        );
        assert_eq!(screen(&terminal).unwrap(), ["fresh", "", ""]);
    }

    /// Enter "never exposing cached output": once control is revoked or the
    /// connection ends, the model is a cache and is not shown, and a new
    /// grant shows nothing until its own restoration is applied.
    #[test]
    fn a_cache_from_an_earlier_period_is_never_shown() {
        for end in [Presentation::Revoked, Presentation::Ended] {
            let mut terminal = restored(b"cached");
            assert!(terminal.view().is_some());
            terminal.present(end);
            assert!(terminal.view().is_none());
            assert_eq!(terminal.control(), Control::NotControlling);
            terminal.present(Presentation::Display(b"x".to_vec()));
            assert_eq!(terminal.present(Presentation::Complete), None);
            terminal.acquire();
            let again = authority(5, 6);
            terminal.present(Presentation::Granted(again, geometry(10, 3)));
            terminal.present(Presentation::Begin(again, geometry(10, 3)));
            assert!(terminal.view().is_none());
            terminal.present(Presentation::Display(b"live".to_vec()));
            assert_eq!(
                terminal.present(Presentation::Complete),
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
        let held = authority(2, 3);
        let mut terminal = restored(b"done");
        terminal.present(Presentation::Answered(Request::Ready(held), Ok(())));
        terminal.painted();
        terminal.present(Presentation::Final);
        assert_eq!(terminal.input(), None);
        assert_eq!(terminal.control(), Control::Ended);
        terminal.present(Presentation::Display(b"\x1b[2Jreleased".to_vec()));
        terminal.present(Presentation::Begin(authority(2, 4), geometry(10, 3)));
        assert_eq!(terminal.present(Presentation::Complete), None);
        terminal.present(Presentation::Revoked);
        assert_eq!(screen(&terminal).unwrap(), ["done", "", ""]);
        assert_eq!(terminal.control(), Control::Ended);

        let mut staged = restored(b"seen");
        staged.present(Presentation::Begin(authority(2, 4), geometry(10, 3)));
        staged.present(Presentation::Display(b"unseen".to_vec()));
        staged.present(Presentation::Final);
        assert_eq!(staged.present(Presentation::Complete), None);
        assert_eq!(screen(&staged).unwrap(), ["seen", "", ""]);
    }

    /// CTL-05 (pure supplement): Resize goes once per grant and desire,
    /// under the held authority, and never without control.
    #[test]
    fn resize_follows_every_grant_and_each_new_desire_only_while_controlling() {
        let desire = TerminalSize { cols: 80, rows: 23 };
        let mut terminal = Terminal::new(LIMITS);
        assert_eq!(terminal.resize(desire), None);
        let first = authority(2, 3);
        terminal.present(Presentation::Granted(first, geometry(10, 3)));
        assert_eq!(
            terminal.resize(desire),
            Some(Request::Resize(first, desire))
        );
        assert_eq!(terminal.resize(desire), None);
        let wider = TerminalSize { cols: 81, rows: 23 };
        assert_eq!(terminal.resize(wider), Some(Request::Resize(first, wider)));
        let second = authority(4, 5);
        terminal.present(Presentation::Granted(second, geometry(10, 3)));
        assert_eq!(terminal.resize(wider), Some(Request::Resize(second, wider)));
        terminal.present(Presentation::Revoked);
        assert_eq!(terminal.resize(desire), None);
    }

    /// Acquiring lasts until the grant or Acquire's answer; a refusal is
    /// shown as the error until the next selection or applied restoration.
    #[test]
    fn acquiring_ends_at_the_grant_or_the_answer_and_errors_are_shown() {
        let mut refused = Terminal::new(LIMITS);
        refused.acquire();
        refused.present(Presentation::Answered(
            Request::Acquire,
            Err("unsupported".into()),
        ));
        assert_eq!(refused.control(), Control::NotControlling);
        assert_eq!(refused.error(), Some("unsupported"));
        refused.acquire();
        assert_eq!(refused.error(), None);
        refused.present(Presentation::Answered(Request::Acquire, Ok(())));
        assert_eq!(refused.control(), Control::NotControlling);
        let mut resized = restored(b"x");
        resized.present(Presentation::Answered(
            Request::Resize(authority(2, 3), TerminalSize { cols: 5, rows: 5 }),
            Err("geometry".into()),
        ));
        assert_eq!(resized.error(), Some("geometry"));
        resized.present(Presentation::Begin(authority(2, 4), geometry(10, 3)));
        resized.present(Presentation::Complete);
        assert_eq!(resized.error(), None);
    }

    /// CTL-05 and TRM-04 (pure supplement): Ready or Resize refused under a
    /// restoration since superseded is not an error, and the refused desire
    /// is sent again under the current authority; one refused under the
    /// current authority is shown and not resent.
    #[test]
    fn work_refused_under_a_superseded_restoration_is_not_an_error_and_resize_is_resent() {
        let old = authority(2, 3);
        let desire = TerminalSize { cols: 90, rows: 34 };
        let mut terminal = restored(b"x");
        assert_eq!(terminal.resize(desire), Some(Request::Resize(old, desire)));
        let new = authority(2, 4);
        terminal.present(Presentation::Begin(new, geometry(10, 3)));
        assert_eq!(terminal.resize(desire), None);
        for refused in [Request::Ready(old), Request::Resize(old, desire)] {
            terminal.present(Presentation::Answered(refused, Err("stale".into())));
        }
        assert_eq!(terminal.error(), None);
        assert_eq!(terminal.resize(desire), Some(Request::Resize(new, desire)));
        terminal.present(Presentation::Answered(
            Request::Resize(new, desire),
            Err("invalid".into()),
        ));
        assert_eq!(terminal.error(), Some("invalid"));
        assert_eq!(terminal.resize(desire), None);
        terminal.present(Presentation::Revoked);
        terminal.present(Presentation::Answered(
            Request::Resize(new, desire),
            Err("late".into()),
        ));
        assert_eq!(terminal.error(), Some("invalid"));
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
        let tight = Limits {
            bytes: 1,
            history_rows: 1000,
        };
        let held = authority(2, 3);
        let mut terminal = Terminal::new(tight);
        terminal.present(Presentation::Granted(held, geometry(10, 3)));
        terminal.present(Presentation::Begin(held, geometry(10, 3)));
        assert!(
            terminal
                .error()
                .unwrap()
                .starts_with("terminal model over its limit")
        );
        assert_eq!(terminal.present(Presentation::Complete), None);
        assert!(terminal.view().is_none());
    }
}
