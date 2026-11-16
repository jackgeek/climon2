//! Mouse reports (spec #73, "Modals and mouse"; #111): pure. The host
//! reports every mouse event in SGR form (`CSI < b ; x ; y M` or `m`), which
//! the input router parses here. Each gesture belongs to the target where its
//! first press landed: the application, under the top row and inside its
//! geometry, or the monitor itself (chrome, margins and overlays). An event
//! for the application is translated to its cells, below the top row, and
//! encoded as its mouse modes ask; a gesture whose target stops taking input
//! is discarded through its release, never sent anywhere else.
use climon2_vt::view::{Modes, MouseEncoding, MouseReporting};

/// One SGR mouse report.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Mouse {
    /// The button code: the button, then modifier, motion and wheel bits.
    pub(crate) code: u16,
    /// The host cell, from 1.
    pub(crate) column: u16,
    pub(crate) row: u16,
    /// A button was released (`m`).
    pub(crate) released: bool,
}

const MOTION: u16 = 32;
const WHEEL: u16 = 64;
/// The button bits of a code; 3 is no button.
const BUTTON: u16 = 3;
/// The shift, meta and control bits.
const MODIFIERS: u16 = 4 | 8 | 16;
const ESC: u8 = 0x1b;
/// The single-byte and UTF-8 encodings add this to every value.
const OFFSET: u32 = 32;

/// What a report is.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Press,
    Release,
    /// Motion, with or without a button held.
    Motion,
    /// A wheel step: a press with no release.
    Wheel,
}

/// The SGR mouse report in the bytes after `ESC [`, if they are one.
pub(crate) fn sgr(body: &[u8]) -> Option<Mouse> {
    let (last, fields) = body.strip_prefix(b"<")?.split_last()?;
    let released = released(*last)?;
    let [code, column, row] = numbers(fields)?;
    Some(Mouse {
        code,
        column,
        row,
        released,
    })
}

/// A report's final byte: `m` for a release, `M` for anything else.
fn released(last: u8) -> Option<bool> {
    match last {
        b'M' => Some(false),
        b'm' => Some(true),
        _ => None,
    }
}

/// Exactly three decimal fields, separated by `;`.
fn numbers(fields: &[u8]) -> Option<[u16; 3]> {
    let numbers: Option<Vec<_>> = fields.split(|byte| *byte == b';').map(number).collect();
    numbers?.try_into().ok()
}

/// A decimal field: digits only.
fn number(field: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(field).ok()?;
    let digits = text.bytes().all(|byte| byte.is_ascii_digit());
    digits.then(|| text.parse().ok()).flatten()
}

impl Mouse {
    fn kind(self) -> Kind {
        if self.released {
            Kind::Release
        } else if self.code & WHEEL != 0 {
            Kind::Wheel
        } else if self.code & MOTION != 0 {
            Kind::Motion
        } else {
            Kind::Press
        }
    }

    /// The application cell under the report, from 1, when it falls inside
    /// `area` (the application's columns and rows the host shows) below
    /// the top row.
    fn inside(self, area: Area) -> Option<Cell> {
        let row = self.row.checked_sub(1).filter(|row| *row >= 1)?;
        (self.column <= area.cols && row <= area.rows).then_some(Cell {
            column: self.column,
            row,
        })
    }

    /// The application cell nearest the report, for a gesture the
    /// application owns that strays outside its area.
    fn clamped(self, area: Area) -> Cell {
        Cell {
            column: self.column.clamp(1, area.cols),
            row: self.row.saturating_sub(1).clamp(1, area.rows),
        }
    }
}

/// The application's columns and rows that the host shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Area {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// An application cell, from 1.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Cell {
    pub(crate) column: u16,
    pub(crate) row: u16,
}

/// The gesture under way, by the target its press found: `Some` while one
/// is, holding the application target that owns it, or none when the
/// monitor owns it or it was discarded.
pub(crate) struct Gestures<T> {
    open: Option<Option<T>>,
}

impl<T> Default for Gestures<T> {
    fn default() -> Self {
        Self { open: None }
    }
}

impl<T: PartialEq + Clone> Gestures<T> {
    /// Where `mouse` goes, given the target taking application input now
    /// and its shown area: the application cell to report it at, or none
    /// when the monitor keeps it. A press begins a gesture, owned by the
    /// application when it lands inside its area; until the release ends
    /// it, every report goes to that owner while it still takes input, at
    /// the cell nearest it, and is otherwise discarded, as is the rest of
    /// the gesture. Motion and the wheel outside a gesture go where they
    /// land.
    pub(crate) fn route(&mut self, mouse: Mouse, target: Option<(&T, Area)>) -> Option<Cell> {
        let kind = mouse.kind();
        let cell = match self.open.take() {
            Some(owner) => self.continued(owner, mouse, target),
            None => self.begun(kind, mouse, target),
        };
        if kind == Kind::Release {
            self.open = None;
        }
        cell
    }

    /// A report within a gesture: its owner's while it still takes input.
    fn continued(
        &mut self,
        owner: Option<T>,
        mouse: Mouse,
        target: Option<(&T, Area)>,
    ) -> Option<Cell> {
        let area = target
            .filter(|(current, _)| owner.as_ref() == Some(*current))
            .map(|(_, area)| area);
        self.open = Some(area.and(owner));
        area.map(|area| mouse.clamped(area))
    }

    /// A report outside a gesture: the application's when it lands inside
    /// its area. A press begins a gesture with that owner.
    fn begun(&mut self, kind: Kind, mouse: Mouse, target: Option<(&T, Area)>) -> Option<Cell> {
        let hit = target.and_then(|(current, area)| Some((current.clone(), mouse.inside(area)?)));
        if kind == Kind::Press {
            self.open = Some(hit.as_ref().map(|(owner, _)| owner.clone()));
        }
        hit.map(|(_, cell)| cell)
    }
}

/// What a wheel notch does over an application that does not report the
/// mouse (spec #73, "History"; owner decision on #112: one row a notch).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Wheel {
    /// Scroll the normal screen's view this many rows back into history,
    /// forward when negative.
    History(isize),
    /// Send the cursor key with this final byte (`A` up, `B` down): the
    /// alternate screen, with alternate scroll on.
    Key(u8),
    /// Nothing: the alternate screen, with alternate scroll off.
    Nothing,
}

/// A vertical wheel notch.
#[derive(Clone, Copy)]
enum Notch {
    Up,
    Down,
}

impl Notch {
    /// `mouse` as a vertical wheel notch, whatever modifiers it has.
    fn of(mouse: Mouse) -> Option<Self> {
        match (mouse.released, mouse.code & !MODIFIERS) {
            (false, WHEEL) => Some(Self::Up),
            (false, WHEEL_DOWN) => Some(Self::Down),
            _ => None,
        }
    }

    /// The rows it scrolls history: up goes back.
    fn rows(self) -> isize {
        match self {
            Self::Up => 1,
            Self::Down => -1,
        }
    }

    /// The cursor key it sends with alternate scroll.
    fn key(self) -> u8 {
        match self {
            Self::Up => b'A',
            Self::Down => b'B',
        }
    }
}

/// The wheel's down notch; up is [`WHEEL`] itself, 64.
const WHEEL_DOWN: u16 = 65;

/// What `mouse` does as a wheel notch over an application in `modes`
/// that does not report the mouse: on the normal screen it scrolls history
/// a row; on the `alternate` screen, which never shows history, it is a
/// cursor key only while the application has alternate scroll on, and
/// otherwise nothing. Anything else, or any report for an application that
/// reports the mouse, is the application's to receive ([`encode`]).
pub(crate) fn wheel(mouse: Mouse, modes: &Modes, alternate: bool) -> Option<Wheel> {
    let notch = Notch::of(mouse).filter(|_| modes.mouse_reporting == MouseReporting::Off)?;
    let wheel = match (alternate, modes.alternate_scroll) {
        (false, _) => Wheel::History(notch.rows()),
        (true, true) => Wheel::Key(notch.key()),
        (true, false) => Wheel::Nothing,
    };
    Some(wheel)
}

/// Appends `mouse`, at the application's `cell`, as an application in
/// `modes` expects it: nothing unless it asked for this kind of event, and
/// nothing when its encoding cannot hold the values.
pub(crate) fn encode(mouse: Mouse, cell: Cell, modes: &Modes, out: &mut Vec<u8>) {
    if reported(mouse, modes.mouse_reporting) {
        out.extend(encoded(mouse, cell, modes.mouse_encoding).unwrap_or_default());
    }
}

/// Whether an application with `reporting` asked for this event: clicks
/// and the wheel from 1000, motion with a button held from 1002, any
/// motion from 1003.
fn reported(mouse: Mouse, reporting: MouseReporting) -> bool {
    let held = mouse.code & BUTTON != BUTTON;
    match (mouse.kind(), reporting) {
        (_, MouseReporting::Off) => false,
        (Kind::Motion, MouseReporting::Clicks) => false,
        (Kind::Motion, MouseReporting::Drag) => held,
        _ => true,
    }
}

/// The report in `encoding`.
fn encoded(mouse: Mouse, cell: Cell, encoding: MouseEncoding) -> Option<Vec<u8>> {
    match encoding {
        MouseEncoding::Sgr => Some(sgr_report(mouse, cell)),
        legacy => legacy_report(mouse, cell, legacy),
    }
}

/// An SGR report keeps the button of a release.
fn sgr_report(mouse: Mouse, cell: Cell) -> Vec<u8> {
    let last = if mouse.released { 'm' } else { 'M' };
    format!("\x1b[<{};{};{}{last}", mouse.code, cell.column, cell.row).into_bytes()
}

/// The older encodings add 32 to every value.
fn legacy_report(mouse: Mouse, cell: Cell, encoding: MouseEncoding) -> Option<Vec<u8>> {
    let code = u8::try_from(u32::from(legacy_code(mouse)) + OFFSET).ok()?;
    let column = coordinate(cell.column, encoding)?;
    let row = coordinate(cell.row, encoding)?;
    Some([&[ESC, b'[', b'M', code][..], &column, &row].concat())
}

/// The older encodings say a release as button 3, keeping its modifiers.
fn legacy_code(mouse: Mouse) -> u16 {
    if mouse.released {
        mouse.code & MODIFIERS | BUTTON
    } else {
        mouse.code
    }
}

/// One coordinate plus 32: a single byte, at most 255, or with 1005 a UTF-8
/// character, at most 2047.
fn coordinate(value: u16, encoding: MouseEncoding) -> Option<Vec<u8>> {
    let value = u32::from(value) + OFFSET;
    if encoding == MouseEncoding::Default {
        return u8::try_from(value).ok().map(|byte| vec![byte]);
    }
    let character = char::from_u32(value).filter(|_| value < 0x800)?;
    Some(character.to_string().into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mouse(code: u16, column: u16, row: u16, released: bool) -> Mouse {
        Mouse {
            code,
            column,
            row,
            released,
        }
    }

    fn press(column: u16, row: u16) -> Mouse {
        mouse(0, column, row, false)
    }

    fn drag(column: u16, row: u16) -> Mouse {
        mouse(32, column, row, false)
    }

    fn release(column: u16, row: u16) -> Mouse {
        mouse(0, column, row, true)
    }

    fn cell(column: u16, row: u16) -> Option<Cell> {
        Some(Cell { column, row })
    }

    const AREA: Area = Area { cols: 10, rows: 5 };

    fn modes(reporting: MouseReporting, encoding: MouseEncoding) -> Modes {
        Modes {
            application_cursor_keys: false,
            application_keypad: false,
            bracketed_paste: false,
            mouse_reporting: reporting,
            mouse_encoding: encoding,
            focus_reporting: false,
            alternate_scroll: false,
        }
    }

    fn sent(mouse: Mouse, at: Cell, modes: &Modes) -> Vec<u8> {
        let mut out = Vec::new();
        encode(mouse, at, modes, &mut out);
        out
    }

    /// INP-05 (pure supplement): SGR reports parse as their button code,
    /// host cell and whether a button was released; anything else in the
    /// `<` form is not a mouse report.
    #[test]
    fn sgr_reports_parse_and_malformed_ones_do_not() {
        assert_eq!(sgr(b"<0;5;7M"), Some(press(5, 7)));
        assert_eq!(sgr(b"<35;65535;1m"), Some(mouse(35, 65535, 1, true)));
        for malformed in [
            &b"<0;5;7"[..],
            b"<0;5;7X",
            b"<0;5M",
            b"<0;5;7;1M",
            b"<0;;7M",
            b"<0;+5;7M",
            b"<0;65536;7M",
            b"<0;\xff;7M",
            b"<",
            b"0;5;7M",
        ] {
            assert_eq!(sgr(malformed), None, "{malformed:?}");
        }
    }

    /// INP-05 (pure supplement): a press inside the application's area,
    /// below the top row, is translated to its cells, at the edges and one
    /// past; the top row, margins and the area's outside stay the
    /// monitor's.
    #[test]
    fn a_click_inside_the_application_is_translated_below_the_top_row() {
        let target = Some((&1, AREA));
        let mut gestures = Gestures::default();
        for (host, expected) in [
            (press(1, 2), cell(1, 1)),
            (press(10, 6), cell(10, 5)),
            (press(11, 6), None),
            (press(10, 7), None),
            (press(1, 1), None),
            (press(1, 0), None),
        ] {
            assert_eq!(gestures.route(host, target), expected, "{host:?}");
            let released = expected.and(cell(1, 1));
            assert_eq!(gestures.route(mouse(0, 1, 2, true), target), released);
        }
    }

    /// INP-05 (pure supplement): a gesture belongs to the target its press
    /// found. The application's drag that strays onto the top row or
    /// outside its area is reported at the nearest cell; a gesture the
    /// monitor owns stays its own over the application; the release ends
    /// each.
    #[test]
    fn each_gesture_keeps_the_target_its_press_found() {
        let target = Some((&1, AREA));
        let mut gestures = Gestures::default();
        assert_eq!(gestures.route(press(3, 3), target), cell(3, 2));
        assert_eq!(gestures.route(drag(30, 1), target), cell(10, 1));
        assert_eq!(gestures.route(drag(0, 40), target), cell(1, 5));
        assert_eq!(gestures.route(release(4, 1), target), cell(4, 1));
        assert_eq!(gestures.route(drag(3, 3), target), cell(3, 2));
        assert_eq!(gestures.route(press(3, 1), target), None);
        assert_eq!(gestures.route(drag(3, 3), target), None);
        assert_eq!(gestures.route(mouse(64, 3, 3, false), target), None);
        assert_eq!(gestures.route(release(3, 3), target), None);
        assert_eq!(gestures.route(mouse(65, 3, 3, false), target), cell(3, 2));
    }

    /// INP-04 and INP-05 (pure supplement): when the gesture's owner stops
    /// taking input (a modal, another target, a new authority), the rest of
    /// the gesture is discarded through its release, even if the owner
    /// takes input again; the next press is hit-tested afresh. A press with
    /// no target taking input begins a gesture the monitor owns.
    #[test]
    fn a_gesture_whose_owner_stops_taking_input_is_discarded_through_its_release() {
        let mut gestures = Gestures::default();
        assert_eq!(gestures.route(press(3, 3), Some((&1, AREA))), cell(3, 2));
        assert_eq!(gestures.route(drag(4, 3), Some((&2, AREA))), None);
        assert_eq!(gestures.route(drag(4, 3), Some((&1, AREA))), None);
        assert_eq!(gestures.route(release(4, 3), Some((&1, AREA))), None);
        assert_eq!(gestures.route(press(3, 3), Some((&1, AREA))), cell(3, 2));
        assert_eq!(gestures.route(drag(4, 3), None), None);
        assert_eq!(gestures.route(release(4, 3), Some((&1, AREA))), None);
        assert_eq!(gestures.route(press(3, 3), None), None);
        assert_eq!(gestures.route(drag(4, 3), Some((&1, AREA))), None);
        assert_eq!(gestures.route(release(4, 3), Some((&1, AREA))), None);
        assert_eq!(gestures.route(release(4, 3), Some((&1, AREA))), cell(4, 2));
    }

    /// INP-05 (pure supplement): an application receives the events its
    /// reporting mode asks for: none while off, presses, releases and the
    /// wheel from 1000, motion with a button held from 1002, any motion
    /// from 1003.
    #[test]
    fn an_application_receives_only_the_events_its_mode_asks_for() {
        let at = Cell { column: 2, row: 3 };
        let hover = mouse(35, 2, 4, false);
        let wheel = mouse(65, 2, 4, false);
        let events = [press(2, 4), release(2, 4), wheel, drag(2, 4), hover];
        let shown = |reporting| {
            let modes = modes(reporting, MouseEncoding::Sgr);
            events.map(|event| !sent(event, at, &modes).is_empty())
        };
        assert_eq!(shown(MouseReporting::Off), [false; 5]);
        assert_eq!(
            shown(MouseReporting::Clicks),
            [true, true, true, false, false]
        );
        assert_eq!(shown(MouseReporting::Drag), [true, true, true, true, false]);
        assert_eq!(shown(MouseReporting::Motion), [true; 5]);
    }

    /// INP-05 (pure supplement): each encoding as the application set it,
    /// written by hand from the xterm control sequences: SGR keeps the
    /// button of a release; the single-byte form says button 3 for a
    /// release, keeping its modifiers, and adds 32, up to 255 and not one
    /// past; the UTF-8 form encodes coordinates as characters, up to 2047
    /// and not one past.
    #[test]
    fn each_encoding_is_written_as_the_application_set_it() {
        let at = |column, row| Cell { column, row };
        let sgr = modes(MouseReporting::Motion, MouseEncoding::Sgr);
        let single = modes(MouseReporting::Motion, MouseEncoding::Default);
        let utf8 = modes(MouseReporting::Motion, MouseEncoding::Utf8);
        assert_eq!(sent(mouse(2, 9, 9, false), at(5, 7), &sgr), b"\x1b[<2;5;7M");
        assert_eq!(
            sent(mouse(18, 9, 9, true), at(5, 7), &sgr),
            b"\x1b[<18;5;7m"
        );
        assert_eq!(
            sent(mouse(2, 9, 9, false), at(5, 7), &single),
            b"\x1b[M\x22\x25\x27"
        );
        assert_eq!(
            sent(mouse(22, 9, 9, true), at(5, 7), &single),
            b"\x1b[M\x37\x25\x27"
        );
        assert_eq!(
            sent(mouse(64, 9, 9, false), at(223, 1), &single),
            b"\x1b[M\x60\xff\x21"
        );
        assert_eq!(sent(mouse(64, 9, 9, false), at(224, 1), &single), b"");
        assert_eq!(
            sent(mouse(223, 9, 9, false), at(1, 1), &single),
            b"\x1b[M\xff\x21\x21"
        );
        assert_eq!(sent(mouse(224, 9, 9, false), at(1, 1), &single), b"");
        assert_eq!(
            sent(mouse(0, 9, 9, false), at(95, 2015), &utf8),
            b"\x1b[M\x20\x7f\xdf\xbf"
        );
        assert_eq!(sent(mouse(0, 9, 9, false), at(96, 2016), &utf8), b"");
        assert_eq!(sent(mouse(0, 9, 9, false), at(1, 2016), &utf8), b"");
    }

    /// TRM-05 (pure supplement; owner decision on #112): a vertical wheel
    /// notch over an application that does not report the mouse scrolls the
    /// normal screen's history one row, back for up and forward for down,
    /// whatever modifiers it carries. On the alternate screen it is a
    /// cursor key, up or down, only with alternate scroll on, and nothing
    /// otherwise. A horizontal notch, a press, a release, motion, or any
    /// report while the application reports the mouse is not the monitor's.
    #[test]
    fn the_wheel_scrolls_history_or_sends_cursor_keys_only_without_mouse_reporting() {
        let off = modes(MouseReporting::Off, MouseEncoding::Default);
        let up = mouse(64, 3, 3, false);
        let down = mouse(65, 3, 3, false);
        assert_eq!(wheel(up, &off, false), Some(Wheel::History(1)));
        assert_eq!(wheel(down, &off, false), Some(Wheel::History(-1)));
        assert_eq!(
            wheel(mouse(64 | 4 | 8 | 16, 3, 3, false), &off, false),
            Some(Wheel::History(1))
        );
        assert_eq!(wheel(down, &off, true), Some(Wheel::Nothing));
        let scrolling = Modes {
            alternate_scroll: true,
            ..off
        };
        assert_eq!(wheel(up, &scrolling, true), Some(Wheel::Key(b'A')));
        assert_eq!(wheel(down, &scrolling, true), Some(Wheel::Key(b'B')));
        assert_eq!(wheel(up, &scrolling, false), Some(Wheel::History(1)));
        for other in [
            mouse(66, 3, 3, false),
            mouse(67, 3, 3, false),
            mouse(64, 3, 3, true),
            press(3, 3),
            release(3, 3),
            drag(3, 3),
        ] {
            assert_eq!(wheel(other, &scrolling, true), None, "{other:?}");
        }
        for reporting in [
            MouseReporting::Clicks,
            MouseReporting::Drag,
            MouseReporting::Motion,
        ] {
            let reported = modes(reporting, MouseEncoding::Sgr);
            assert_eq!(wheel(up, &reported, false), None, "{reporting:?}");
        }
    }
}
