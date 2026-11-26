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
    /// Whether the monitor views the session (`status-v1`; #114): true
    /// only under ready authority.
    Viewing(Authority, bool),
    /// Acknowledges pending attention, after an explicit action, once the
    /// session is viewed (`status-v1`; #114).
    Acknowledge(Authority),
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

impl Paste {
    /// The part that closes a bracketed paste cut short: its end marker
    /// alone.
    pub(crate) fn end() -> Self {
        Self {
            bracketed: true,
            starts: false,
            bytes: Vec::new(),
            ends: true,
        }
    }
}

impl Authority {
    /// Whether `other` belongs to the same control period: its
    /// restoration may differ.
    fn same_period(self, other: Self) -> bool {
        (self.generation, self.period) == (other.generation, other.period)
    }
}

/// Why the daemon revoked this monitor's control.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Revocation {
    /// Another client took control.
    Takeover,
    /// The session is ending: its command's final result follows.
    Ending,
}

/// What the session's client reported about control and display. The
/// client library checks that every record names the authority the role
/// holds, so only grants and beginnings carry theirs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Presentation {
    Granted(Authority, Geometry),
    Revoked(Revocation),
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
    /// The client opened with `status-v1` selected: viewing and
    /// acknowledgement can be reported (#114).
    Viewable,
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

/// A state the monitor reports to the daemon (application focus, viewing),
/// coalesced (owner decision on #114, 2026-11-17): while one report is
/// unanswered no other is submitted, so however often the desire changes
/// the library's operation queue holds at most one of each kind, and the
/// latest desire goes once that answer arrives.
#[derive(Clone, Copy, Debug, Default)]
struct Report {
    /// The value last submitted, with the authority it was submitted under.
    sent: Option<(Authority, bool)>,
    /// The last submitted report is not yet answered.
    unanswered: bool,
}

impl Report {
    /// Submits `wanted` when it is `due` and no report is unanswered.
    fn submit(&mut self, due: bool, wanted: (Authority, bool)) -> bool {
        let submitted = due && !self.unanswered;
        if submitted {
            *self = Self {
                sent: Some(wanted),
                unanswered: true,
            };
        }
        submitted
    }
}

/// One session's terminal: kept per session ID for the monitor's whole run.
pub(crate) struct Terminal {
    limits: Limits,
    model: Option<Model>,
    held: Option<Held>,
    /// Acquire is submitted and not yet answered. It counts only while no
    /// control is held: a grant shows as held control instead, and every
    /// end of held control clears it.
    acquiring: bool,
    /// Once the command ended (the model changes no more): whether its
    /// frozen view shows, which it does only if the control period held then
    /// had applied a restoration.
    finished: Option<bool>,
    /// The latest failure to show, until the next applied restoration.
    error: Option<String>,
    /// The client opened with `focus-v1`, until it ends.
    focusable: bool,
    /// The application focus reported.
    focused: Report,
    /// The client opened with `status-v1`, until it ends.
    viewable: bool,
    /// The viewing reported.
    viewed: Report,
    /// An Acknowledge was refused because a newer restoration of its
    /// control period superseded it: it is owed again.
    unacknowledged: bool,
    /// A bracketed paste cut short under this authority: its application is
    /// owed the end marker.
    cut: Option<Authority>,
    /// A restoration was ever applied: the model holds terminal content,
    /// kept as a read-only cache once control or the connection ends.
    cached: bool,
    /// Why the model refused an update since the monitor last asked: the
    /// session's client is dropped, its cache kept read-only (#119).
    rejection: Option<String>,
    /// This connection's client was dropped for a refusal: what it still
    /// delivers before its end is refused too, and drops nothing more.
    dropped: bool,
    /// Another client took the control this monitor last held on this
    /// connection: it was revoked, and not granted again since (#400).
    taken: bool,
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
            focused: Report::default(),
            viewable: false,
            viewed: Report::default(),
            unacknowledged: false,
            cut: None,
            cached: false,
            rejection: None,
            dropped: false,
            taken: false,
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
            Presentation::Viewable => self.viewable = true,
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
        self.taken = false;
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
            self.cached = true;
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
        self.settled(&request);
        match (answer, self.current(&request)) {
            (Ok(()), _) => self.accepted(&request),
            (Err(error), true) => self.error = Some(refusal(&request, error)),
            (Err(_), false) => self.superseded(&request),
        }
    }

    /// An answered Acquire is no longer awaited, and an answered Focus or
    /// Set viewing lets the next of its kind go.
    fn settled(&mut self, request: &Request) {
        match request {
            Request::Acquire => self.acquiring = false,
            Request::Focus(..) => self.focused.unanswered = false,
            Request::Viewing(..) => self.viewed.unanswered = false,
            _ => {}
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
            | Request::Focus(authority, _)
            | Request::Viewing(authority, _)
            | Request::Acknowledge(authority) => {
                self.held.is_some_and(|held| held.authority == authority)
            }
        }
    }

    /// A superseded Resize is sent again under the current authority, a
    /// superseded viewing report again if the current viewing differs from
    /// the daemon's, and an Acknowledge a newer restoration of its control
    /// period superseded (a resize brings one) is owed again.
    fn superseded(&mut self, request: &Request) {
        match (request, self.held.as_mut()) {
            (Request::Resize(..), Some(held)) => held.resized = None,
            (Request::Viewing(..), _) => self.viewed.sent = None,
            (Request::Acknowledge(authority), Some(held)) => {
                self.unacknowledged = held.authority.same_period(*authority);
            }
            _ => {}
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
    /// The end of the connection also ends its errors: work refused as it
    /// closed is the loss's symptom, and the row's diagnostic says why
    /// (#116, #119). Only a takeover's revocation says another client took
    /// control: one for the session's end does not, and a new connection
    /// knows nothing of who controls the session (#400).
    fn ended(&mut self, presentation: &Presentation) {
        if *presentation == Presentation::Final {
            let live = self.held.is_some_and(|held| held.live);
            self.finished.get_or_insert(live);
        } else {
            self.held = None;
            self.acquiring = false;
            self.taken = *presentation == Presentation::Revoked(Revocation::Takeover);
            let open = *presentation != Presentation::Ended;
            self.focusable &= open;
            self.viewable &= open;
            self.error = self.error.take().filter(|_| open);
            self.dropped &= open;
            // An ended client's answers never come.
            self.focused.unanswered &= open;
            self.viewed.unanswered &= open;
        }
        if let Some(model) = self.model.as_mut() {
            model.discard();
        }
    }

    /// An update the model refused is shown as the error, and the
    /// session's client is to be dropped for it.
    fn rejected(&mut self, fed: Result<(), Rejected>) {
        if let Err(rejected) = fed {
            let why = format!(
                "terminal model over its limit: {} bytes needed, {} allowed",
                rejected.needed, rejected.limit
            );
            if !self.dropped {
                self.rejection = Some(why.clone());
            }
            self.dropped = true;
            self.error = Some(why);
        }
    }

    /// Why the model refused an update since the last call, if it did: the
    /// monitor drops the session's client, which is retried passively, and
    /// keeps the model's last safe content read-only (#119).
    pub(crate) fn take_rejection(&mut self) -> Option<String> {
        self.rejection.take()
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
    /// authority and desire, and while no Focus is unanswered ([`Report`]).
    /// A newly ready restoration so submits the current desire once, never
    /// an earlier one.
    pub(crate) fn focus(&mut self, desired: bool) -> Option<Request> {
        let held = self
            .held
            .filter(|_| self.focusable && self.finished.is_none())?;
        let wanted = (held.authority, desired);
        let ready = matches!(held.phase, Phase::Applied { ready: true, .. });
        let due = ready && self.focused.sent != Some(wanted);
        self.focused
            .submit(due, wanted)
            .then_some(Request::Focus(held.authority, desired))
    }

    /// The Set viewing to submit so the daemon's viewing for this control
    /// period is `desired` (spec #73, "Viewing"): only with `status-v1`,
    /// under the current authority, on each change, while no Set viewing is
    /// unanswered ([`Report`]). A control period starts not viewing, as the
    /// daemon's does; a new restoration in it keeps the value reported.
    pub(crate) fn viewing(&mut self, desired: bool) -> Option<Request> {
        let held = self
            .held
            .filter(|_| self.viewable && self.finished.is_none())?;
        let due = self.viewed_in(held) != desired;
        self.viewed
            .submit(due, (held.authority, desired))
            .then_some(Request::Viewing(held.authority, desired))
    }

    /// The viewing last submitted in `held`'s control period: false until
    /// one was, as the daemon's starts.
    fn viewed_in(&self, held: Held) -> bool {
        self.viewed
            .sent
            .is_some_and(|(authority, viewing)| viewing && authority.same_period(held.authority))
    }

    /// Whether an Acknowledge may follow (STA-08: it needs the latest
    /// viewing true): Set viewing true was the last submitted in the held
    /// control period, answered or not, since the daemon serves requests in
    /// order. A client without `status-v1` reports no viewing, so nothing
    /// holds its Acknowledge back.
    pub(crate) fn viewing_reported(&self) -> bool {
        !self.viewable || self.held.is_some_and(|held| self.viewed_in(held))
    }

    /// Whether an Acknowledge is owed again since the last call
    /// ([`Terminal::superseded`]).
    pub(crate) fn take_unacknowledged(&mut self) -> bool {
        std::mem::take(&mut self.unacknowledged)
    }

    /// A bracketed paste sent under `authority` was cut short by a target
    /// change: its application is owed the end marker (owner decision on
    /// #111).
    pub(crate) fn cut_paste(&mut self, authority: Authority) {
        self.cut = Some(authority);
    }

    /// The end marker a cut paste owes, submitted once under the current
    /// authority as soon as its restoration is ready, while the control
    /// period that received the paste lasts. Once that period ends nothing
    /// is owed: the daemon ends an open paste when admission ends.
    pub(crate) fn paste_end(&mut self) -> Option<Request> {
        let cut = self.cut?;
        let held = self.held.filter(|held| held.authority.same_period(cut));
        self.cut = held.and(self.cut);
        let held = held.filter(|held| matches!(held.phase, Phase::Applied { ready: true, .. }))?;
        self.cut = None;
        Some(Request::Paste(held.authority, Paste::end()))
    }

    /// The authority of the control period and restoration held, if any:
    /// it changes with every grant, restoration, revocation and end.
    pub(crate) fn authority(&self) -> Option<Authority> {
        self.held.map(|held| held.authority)
    }

    /// Scrolls the model's normal-screen view `rows` back into its history,
    /// forward when negative (#112).
    pub(crate) fn scroll(&mut self, rows: isize) {
        if let Some(model) = self.model.as_mut() {
            model.scroll(rows);
        }
    }

    /// Returns the model's view to the live screen: application input
    /// went to the session (#112).
    pub(crate) fn return_live(&mut self) {
        if let Some(model) = self.model.as_mut() {
            model.return_live();
        }
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

    /// Whether another client took control from this monitor (#400): only
    /// then does its placeholder say the session is viewed elsewhere.
    pub(crate) fn taken_over(&self) -> bool {
        self.taken
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

    /// The allocation the model holds, any staged restoration included
    /// (#119).
    pub(crate) fn footprint(&self) -> usize {
        self.model.as_ref().map_or(0, Model::footprint)
    }

    /// What the model holds beyond its current model's mandatory allocation:
    /// its history, with any staged restoration and cell extras, which the
    /// history budgets bound (#119).
    pub(crate) fn history_bytes(&self) -> usize {
        self.model.as_ref().map_or(0, |model| {
            let rows = self.limits.history_rows;
            let base = Model::mandatory(model.geometry(), rows).current_model;
            model.footprint().saturating_sub(base)
        })
    }

    /// Gives up the model's oldest history chunk, never the one holding its
    /// newest page: false once nothing more can go (#119).
    pub(crate) fn shorten(&mut self) -> bool {
        self.model.as_mut().is_some_and(Model::shorten_history)
    }

    /// The retained cache, for read-only inspection once the session is no
    /// longer connected (#116): what the last applied restoration and the
    /// output after it left, from any generation; none before one applied.
    pub(crate) fn retained(&self) -> Option<View<'_>> {
        self.model.as_ref().filter(|_| self.cached).map(Model::view)
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
        for end in [
            Presentation::Revoked(Revocation::Takeover),
            Presentation::Ended,
        ] {
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

    /// #400: only a takeover's revocation says another client took control;
    /// a session never controlled, one whose connection ended, one revoked
    /// because it is ending, and one granted again was not taken over, and
    /// the command's end changes nothing.
    #[test]
    fn only_a_takeover_takes_control_over() {
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(LIMITS);
        assert!(!terminal.taken_over());
        terminal.present(&mut parser, Presentation::Ended);
        assert!(!terminal.taken_over());
        let mut ending = restored(b"held");
        ending.present(&mut parser, Presentation::Revoked(Revocation::Ending));
        assert_eq!(ending.control(), Control::NotControlling);
        assert!(!ending.taken_over());
        let mut terminal = restored(b"held");
        assert!(!terminal.taken_over());
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert!(terminal.taken_over());
        terminal.present(&mut parser, Presentation::Final);
        assert!(terminal.taken_over());
        let mut terminal = restored(b"held");
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        terminal.acquire();
        assert!(terminal.taken_over());
        let again = authority(5, 6);
        terminal.present(&mut parser, Presentation::Granted(again, geometry(10, 3)));
        assert!(!terminal.taken_over());
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert!(terminal.taken_over());
        terminal.present(&mut parser, Presentation::Ended);
        assert!(!terminal.taken_over());
    }

    /// REC-02 (pure supplement, #116): a terminal keeps its last applied
    /// restoration, and the output after it, as a read-only cache once its
    /// control or connection ends, from one generation to the next; a
    /// terminal that never applied one, staged or not, has no cache.
    #[test]
    fn the_last_applied_content_is_retained_read_only() {
        let mut parser = Parser::new();
        let retained = |terminal: &Terminal| {
            let view = terminal.retained()?;
            let climon2_vt::view::Screen::Normal { rows, .. } = view.screen() else {
                panic!("the normal screen");
            };
            let first = rows.iter().next()?;
            let text: String = first
                .cells()
                .filter_map(|cell| match cell.content {
                    climon2_vt::view::Content::Narrow(glyph) => Some(glyph.character),
                    _ => None,
                })
                .collect();
            Some(text.trim_end().to_owned())
        };
        let mut never = Terminal::new(LIMITS);
        assert_eq!(retained(&never), None);
        let held = authority(2, 3);
        never.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        never.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        never.present(&mut parser, Presentation::Display(b"staged".to_vec()));
        never.present(&mut parser, Presentation::Ended);
        assert_eq!(retained(&never), None);
        let mut terminal = restored(b"cache");
        terminal.present(&mut parser, Presentation::Display(b" more".to_vec()));
        terminal.present(&mut parser, Presentation::Ended);
        assert!(terminal.view().is_none());
        assert_eq!(retained(&terminal).as_deref(), Some("cache more"));
        let fresh = Authority {
            generation: 2,
            period: 1,
            restoration: 1,
        };
        terminal.present(&mut parser, Presentation::Granted(fresh, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Begin(fresh, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(b"next".to_vec()));
        assert_eq!(retained(&terminal).as_deref(), Some("cache more"));
        terminal.present(&mut parser, Presentation::Complete);
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert_eq!(retained(&terminal).as_deref(), Some("next"));
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
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
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
        revoked.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
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
        again.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        again.acquire();
        let next = authority(4, 5);
        again.present(&mut parser, Presentation::Granted(next, geometry(10, 3)));
        again.present(&mut parser, Presentation::Begin(next, geometry(10, 3)));
        again.present(&mut parser, Presentation::Display(b"partial".to_vec()));
        again.present(&mut parser, Presentation::Final);
        again.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
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
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
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

    /// REC-02 (pure supplement): work refused as the connection closed is
    /// shown while control lasts; a revocation keeps it, the end of the
    /// connection drops it, leaving the row's diagnostic to say why.
    #[test]
    fn the_end_of_the_connection_ends_its_errors() {
        let mut parser = Parser::new();
        let mut terminal = restored(b"x");
        let closed = "closed: the instance accepts no more requests";
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Viewing(authority(2, 3), true), Err(closed.into())),
        );
        assert_eq!(terminal.error(), Some(closed));
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert_eq!(terminal.error(), Some(closed));
        terminal.present(&mut parser, Presentation::Ended);
        assert_eq!(terminal.error(), None);
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
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
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
    /// the ready restoration's authority for each new desire once the
    /// previous Focus is answered, never twice for the same one; a newly
    /// ready restoration submits the current desire once. Nothing is
    /// submitted before Ready is accepted, without control, once the command
    /// ended, or without `focus-v1`; a refusal under the current authority
    /// is shown, one under a superseded authority is not.
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
        terminal.present(
            &mut parser,
            Presentation::Answered(Request::Focus(held, true), Ok(())),
        );
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

    /// FOC-01 and STA-08 (pure supplement, #114): with `status-v1`, Set
    /// viewing is submitted under the current authority on each change
    /// once the previous one is answered, never twice for the same value. A
    /// control period starts not viewing, so nothing is sent for false until
    /// true was; a new restoration keeps the value reported, a refusal under
    /// a superseded authority sends the current value again, and a refusal
    /// under the current one is shown. Nothing is submitted without
    /// `status-v1` (which ends with the client), without control or once the
    /// command ended. An Acknowledge's refusal shows as any other.
    #[test]
    fn viewing_is_reported_on_each_change_within_a_control_period() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        let ready = |held| Presentation::Answered(Request::Ready(held), Ok(()));
        terminal.present(&mut parser, ready(held));
        assert_eq!(terminal.viewing(true), None);
        terminal.present(&mut parser, Presentation::Viewable);
        assert_eq!(terminal.viewing(false), None);
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(held, true)));
        assert_eq!(terminal.viewing(true), None);
        let accepted = |request| Presentation::Answered(request, Ok(()));
        terminal.present(&mut parser, accepted(Request::Viewing(held, true)));
        assert_eq!(terminal.viewing(true), None);
        let newer = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(newer, geometry(10, 3)));
        assert_eq!(terminal.viewing(true), None);
        assert_eq!(
            terminal.viewing(false),
            Some(Request::Viewing(newer, false))
        );
        assert_eq!(terminal.viewing(false), None);
        terminal.present(&mut parser, accepted(Request::Viewing(newer, false)));
        terminal.present(&mut parser, Presentation::Complete);
        terminal.present(&mut parser, ready(newer));
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(newer, true)));
        let refused = |request, error: &str| Presentation::Answered(request, Err(error.into()));
        terminal.present(&mut parser, refused(Request::Viewing(held, true), "stale"));
        assert_eq!(terminal.error(), None);
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(newer, true)));
        assert_eq!(terminal.viewing(true), None);
        terminal.present(
            &mut parser,
            refused(Request::Acknowledge(newer), "not viewing"),
        );
        assert_eq!(terminal.error(), Some("not viewing"));
        assert!(!terminal.take_unacknowledged());
        terminal.present(&mut parser, accepted(Request::Viewing(newer, true)));
        let next = authority(3, 1);
        terminal.present(&mut parser, Presentation::Granted(next, geometry(10, 3)));
        assert_eq!(terminal.viewing(false), None);
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(next, true)));
        terminal.present(&mut parser, Presentation::Final);
        assert_eq!(terminal.viewing(false), None);
        let mut ended = restored(b"x");
        ended.present(&mut parser, Presentation::Viewable);
        ended.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert_eq!(ended.viewing(true), None);
        ended.present(&mut parser, Presentation::Granted(next, geometry(10, 3)));
        assert_eq!(ended.viewing(true), Some(Request::Viewing(next, true)));
        ended.present(&mut parser, Presentation::Ended);
        ended.present(&mut parser, Presentation::Granted(next, geometry(10, 3)));
        assert_eq!(ended.viewing(true), None);
    }

    /// STA-08 (pure supplement, #114): an Acknowledge refused because a newer
    /// restoration of its control period superseded it, as a resize brings,
    /// is owed again, once; one refused under an earlier control period, or
    /// once control ended, or accepted, is not.
    #[test]
    fn an_acknowledge_a_newer_restoration_superseded_is_owed_again() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        let answered = |held, answer| Presentation::Answered(Request::Acknowledge(held), answer);
        terminal.present(&mut parser, answered(held, Ok(())));
        assert!(!terminal.take_unacknowledged());
        let newer = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(newer, geometry(10, 3)));
        terminal.present(&mut parser, answered(held, Err("stale".into())));
        assert_eq!(terminal.error(), None);
        assert!(terminal.take_unacknowledged());
        assert!(!terminal.take_unacknowledged());
        terminal.present(&mut parser, answered(authority(1, 9), Err("stale".into())));
        assert!(!terminal.take_unacknowledged());
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        terminal.present(&mut parser, answered(newer, Err("stale".into())));
        assert!(!terminal.take_unacknowledged());
    }

    /// INP-04 (pure supplement; owner decision on #111): a bracketed paste
    /// cut short by a target change that keeps admission owes its
    /// application the end marker, once: at once while its restoration is
    /// still ready; after a new restoration of the same control period,
    /// under that restoration once it is ready; never once the period
    /// ended, revoked, even if control comes back.
    #[test]
    fn a_cut_paste_owes_its_end_marker_under_the_current_authority() {
        let mut parser = Parser::new();
        let end = |held| Some(Request::Paste(held, Paste::end()));
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        let ready = |held| Presentation::Answered(Request::Ready(held), Ok(()));
        terminal.present(&mut parser, ready(held));
        assert_eq!(terminal.paste_end(), None);
        terminal.cut_paste(held);
        assert_eq!(terminal.paste_end(), end(held));
        assert_eq!(terminal.paste_end(), None);
        terminal.cut_paste(held);
        let newer = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(newer, geometry(10, 3)));
        assert_eq!(terminal.paste_end(), None);
        terminal.present(&mut parser, Presentation::Complete);
        assert_eq!(terminal.paste_end(), None);
        terminal.present(&mut parser, ready(newer));
        assert_eq!(terminal.paste_end(), end(newer));
        assert_eq!(terminal.paste_end(), None);
        terminal.cut_paste(newer);
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert_eq!(terminal.paste_end(), None);
        ready_under(&mut terminal, &mut parser, authority(3, 1));
        assert_eq!(terminal.paste_end(), None);
    }

    /// Grants `held` with an applied restoration whose Ready was accepted.
    fn ready_under(terminal: &mut Terminal, parser: &mut Parser, held: Authority) {
        terminal.present(parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(parser, Presentation::Complete);
        terminal.present(parser, Presentation::Answered(Request::Ready(held), Ok(())));
    }

    /// FOC-01 and STA-08 (pure supplement; owner decision on #114, 2026-11-17):
    /// while a Focus or Set viewing is unanswered, no other of its kind is
    /// submitted, however often the desire changes; its answer, accepted or
    /// refused, lets the latest desire go once, and nothing when that is
    /// what was last submitted. Each kind waits only for its own answer.
    #[test]
    fn focus_and_viewing_wait_for_their_answers_and_then_send_the_latest_desire() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        terminal.present(&mut parser, Presentation::Focusable);
        terminal.present(&mut parser, Presentation::Viewable);
        let answered = |request, answer| Presentation::Answered(request, answer);
        terminal.present(&mut parser, answered(Request::Ready(held), Ok(())));
        assert_eq!(terminal.focus(true), Some(Request::Focus(held, true)));
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(held, true)));
        for desired in [false, true, false, true, false] {
            assert_eq!(terminal.focus(desired), None, "focus {desired} waits");
            assert_eq!(terminal.viewing(desired), None, "viewing {desired} waits");
        }
        terminal.present(&mut parser, answered(Request::Focus(held, true), Ok(())));
        assert_eq!(
            terminal.viewing(false),
            None,
            "Focus's answer is not viewing's"
        );
        assert_eq!(terminal.focus(false), Some(Request::Focus(held, false)));
        assert_eq!(terminal.focus(true), None);
        terminal.present(&mut parser, answered(Request::Viewing(held, true), Ok(())));
        assert_eq!(
            terminal.focus(true),
            None,
            "viewing's answer is not Focus's"
        );
        assert_eq!(terminal.viewing(true), None, "true was last submitted");
        assert_eq!(terminal.viewing(false), Some(Request::Viewing(held, false)));
        let refused = Err("queue_full".to_owned());
        terminal.present(&mut parser, answered(Request::Focus(held, false), refused));
        assert_eq!(terminal.error(), Some("queue_full"));
        assert_eq!(terminal.focus(false), None, "false was last submitted");
        assert_eq!(terminal.focus(true), Some(Request::Focus(held, true)));
    }

    /// STA-08 (pure supplement; owner decision on #114, 2026-11-17): an
    /// Acknowledge may follow only once Set viewing true was submitted in
    /// the held control period, answered or not: not while the latest
    /// submitted is false or waits behind an unanswered report, nor after
    /// another period's or without control. A client without `status-v1`
    /// reports no viewing, so nothing holds its Acknowledge back.
    #[test]
    fn an_acknowledge_follows_only_viewing_true_submitted_in_the_period() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        let answered = |request| Presentation::Answered(request, Ok(()));
        terminal.present(&mut parser, answered(Request::Ready(held)));
        assert!(terminal.viewing_reported(), "no status-v1");
        terminal.present(&mut parser, Presentation::Viewable);
        assert!(!terminal.viewing_reported(), "nothing submitted yet");
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(held, true)));
        assert!(terminal.viewing_reported(), "submitted, not yet answered");
        terminal.present(&mut parser, answered(Request::Viewing(held, true)));
        assert_eq!(terminal.viewing(false), Some(Request::Viewing(held, false)));
        assert!(!terminal.viewing_reported());
        assert_eq!(terminal.viewing(true), None, "waits for false's answer");
        assert!(!terminal.viewing_reported(), "true not yet submitted");
        terminal.present(&mut parser, answered(Request::Viewing(held, false)));
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(held, true)));
        assert!(terminal.viewing_reported());
        let newer = authority(2, 4);
        terminal.present(&mut parser, Presentation::Begin(newer, geometry(10, 3)));
        assert!(terminal.viewing_reported(), "a new restoration keeps it");
        terminal.present(
            &mut parser,
            Presentation::Granted(authority(3, 1), geometry(10, 3)),
        );
        assert!(!terminal.viewing_reported(), "another period's");
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        assert!(!terminal.viewing_reported(), "no control");
    }

    /// FOC-01 and STA-08 (pure supplement; owner decision on #114,
    /// 2026-11-17): a revocation leaves the connection, so an unanswered
    /// Focus or Set viewing is still awaited under the next grant; once the
    /// client ended its answers never come, and the next client's are not
    /// held back by them.
    #[test]
    fn only_the_end_of_the_client_forgets_unanswered_focus_and_viewing() {
        let mut parser = Parser::new();
        let held = authority(2, 3);
        let mut terminal = restored(b"x");
        terminal.present(&mut parser, Presentation::Focusable);
        terminal.present(&mut parser, Presentation::Viewable);
        let answered = |request, answer| Presentation::Answered(request, answer);
        terminal.present(&mut parser, answered(Request::Ready(held), Ok(())));
        assert_eq!(terminal.focus(true), Some(Request::Focus(held, true)));
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(held, true)));
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
        let again = authority(3, 1);
        ready_under(&mut terminal, &mut parser, again);
        assert_eq!(terminal.focus(true), None);
        assert_eq!(terminal.viewing(true), None);
        let stale = || Err("stale".to_owned());
        terminal.present(&mut parser, answered(Request::Focus(held, true), stale()));
        terminal.present(&mut parser, answered(Request::Viewing(held, true), stale()));
        assert_eq!(terminal.error(), None);
        assert_eq!(terminal.focus(true), Some(Request::Focus(again, true)));
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(again, true)));
        terminal.present(&mut parser, Presentation::Ended);
        terminal.present(&mut parser, Presentation::Focusable);
        terminal.present(&mut parser, Presentation::Viewable);
        let later = authority(4, 1);
        ready_under(&mut terminal, &mut parser, later);
        assert_eq!(terminal.focus(true), Some(Request::Focus(later, true)));
        assert_eq!(terminal.viewing(true), Some(Request::Viewing(later, true)));
    }

    /// FOC-02 (pure supplement): `focus-v1` lasts while the client does:
    /// after revocation, the next grant's ready restoration is still sent
    /// Focus; once the client ended, a later grant is not.
    #[test]
    fn focus_support_lasts_until_the_client_ends() {
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(LIMITS);
        terminal.present(&mut parser, Presentation::Focusable);
        terminal.present(&mut parser, Presentation::Revoked(Revocation::Takeover));
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
        assert_eq!(
            terminal.take_rejection().as_deref(),
            terminal.error(),
            "the refused restoration drops the client"
        );
        assert_eq!(terminal.take_rejection(), None);
    }

    /// The text of `view`'s normal-screen row `index`, narrow characters
    /// only, trailing blanks trimmed.
    fn row_text(view: &View<'_>, index: usize) -> String {
        let climon2_vt::view::Screen::Normal { rows, .. } = view.screen() else {
            panic!("the normal screen");
        };
        let text: String = rows
            .get(index)
            .unwrap()
            .cells()
            .filter_map(|cell| match cell.content {
                climon2_vt::view::Content::Narrow(glyph) => Some(glyph.character),
                _ => None,
            })
            .collect();
        text.trim_end().to_owned()
    }

    /// LIM-03 (pure supplement, rejected updates): output the model's limit
    /// refuses drops the session's client once, with the error it shows, and
    /// keeps the last safe content as the read-only cache once control ends;
    /// nothing after the refusal reaches it. Output within the limit drops
    /// nothing.
    #[test]
    fn refused_output_drops_the_client_once_and_keeps_the_last_safe_cache() {
        let mut parser = Parser::new();
        let mandatory = Model::mandatory(geometry(10, 3), 1000);
        // Room for the blank model and the restoration staged beside it,
        // and 40,000 bytes more; once applied, the restored model alone
        // leaves under 80,000 bytes free: one combining mark fits, 10,000
        // (16 bytes each kept, and 72 at most while each is added) do not.
        let limits = Limits {
            bytes: 2 * mandatory.current_model + 40_000,
            history_rows: 1000,
        };
        let held = authority(2, 3);
        let mut terminal = Terminal::new(limits);
        terminal.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(b"safe".to_vec()));
        terminal.present(&mut parser, Presentation::Complete);
        terminal.present(&mut parser, Presentation::Display("e\u{301}".into()));
        assert_eq!(terminal.take_rejection(), None);
        assert_eq!(terminal.error(), None);
        let marks = "\u{301}".repeat(10_000);
        terminal.present(&mut parser, Presentation::Display(marks.into()));
        let why = terminal.take_rejection().expect("the client is dropped");
        assert!(why.starts_with("terminal model over its limit: "), "{why}");
        assert_eq!(terminal.error(), Some(why.as_str()));
        assert_eq!(terminal.take_rejection(), None);
        // What the dropped connection still delivers is refused, and drops
        // its session's next client no more.
        terminal.present(&mut parser, Presentation::Display(b"\r\nlater".to_vec()));
        assert_eq!(terminal.take_rejection(), None);
        terminal.present(&mut parser, Presentation::Ended);
        let cached = terminal.retained().expect("the cache is retained");
        assert_eq!(row_text(&cached, 0), "safee");
        assert_eq!(row_text(&cached, 1), "", "nothing after the refusal");
        // The next connection's refusal, here of its restoration beside the
        // full cache, drops its client again.
        let held = authority(3, 4);
        terminal.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Complete);
        let marks = "\u{301}".repeat(10_000);
        terminal.present(
            &mut parser,
            Presentation::Display(format!("e{marks}").into()),
        );
        assert!(terminal.take_rejection().is_some());
    }

    /// LIM-03 (pure supplement, bounded history): a terminal's footprint is
    /// its model's, and its history bytes what the model holds beyond its
    /// current model; shortening gives up history a chunk at a time, oldest
    /// first, never the chunk holding the newest page, and the view shows
    /// the truncation. A terminal without a model holds nothing and has
    /// nothing to give up.
    #[test]
    fn history_is_shortened_a_chunk_at_a_time_down_to_its_newest_page() {
        let mut blank = Terminal::new(LIMITS);
        assert_eq!((blank.footprint(), blank.history_bytes()), (0, 0));
        assert!(!blank.shorten());
        let lines: String = (1..=2500).map(|line| format!("{line}\r\n")).collect();
        let mut parser = Parser::new();
        let mut terminal = Terminal::new(Limits {
            bytes: 64 << 20,
            history_rows: 3000,
        });
        let held = authority(2, 3);
        terminal.present(&mut parser, Presentation::Granted(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Begin(held, geometry(10, 3)));
        terminal.present(&mut parser, Presentation::Display(lines.into()));
        terminal.present(&mut parser, Presentation::Complete);
        let mandatory = Model::mandatory(geometry(10, 3), 3000);
        let full = terminal.history_bytes();
        assert_eq!(full, terminal.footprint() - mandatory.current_model);
        assert!(full > 2 * mandatory.recent_page, "{full}");
        assert!(terminal.shorten());
        assert!(terminal.history_bytes() < full);
        while terminal.shorten() {}
        assert!(terminal.history_bytes() >= mandatory.recent_page);
        let view = terminal.view().unwrap();
        assert_eq!(row_text(&view, 1), "2500");
    }

    /// TRM-05 (pure supplement): the wheel scrolls the model's history and
    /// application input returns it live; a terminal with no model yet has
    /// nothing to scroll.
    #[test]
    fn the_view_scrolls_into_history_and_input_returns_it_live() {
        let mut terminal = restored(b"a\r\nb\r\nc\r\nd\r\ne");
        let scrolled = |terminal: &Terminal| terminal.view().map(|view| view.scrolled());
        assert_eq!(scrolled(&terminal), Some(0));
        terminal.scroll(2);
        assert_eq!(scrolled(&terminal), Some(2));
        terminal.scroll(-1);
        assert_eq!(scrolled(&terminal), Some(1));
        terminal.return_live();
        assert_eq!(scrolled(&terminal), Some(0));

        let mut blank = Terminal::new(LIMITS);
        blank.scroll(1);
        blank.return_live();
        assert_eq!(blank.geometry(), None);
    }
}
