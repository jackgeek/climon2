//! The application modes the monitor's input router and painter honour.

use alacritty_terminal::term::TermMode;

/// What the application asked of the keyboard, paste, mouse, focus and
/// wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    /// DECCKM (`CSI ? 1 h` and `l`): the cursor keys send application
    /// sequences.
    pub application_cursor_keys: bool,
    /// DECKPAM (`ESC =`) and DECKPNM (`ESC >`): the keypad sends application
    /// sequences.
    pub application_keypad: bool,
    /// `CSI ? 2004 h` and `l`: pastes are bracketed.
    pub bracketed_paste: bool,
    pub mouse_reporting: MouseReporting,
    pub mouse_encoding: MouseEncoding,
    /// `CSI ? 1004 h` and `l`: focus in and out are reported.
    pub focus_reporting: bool,
    /// `CSI ? 1007 h` and `l`: on the alternate screen, the wheel sends
    /// cursor keys.
    pub alternate_scroll: bool,
}

impl Modes {
    pub(crate) fn of(mode: TermMode) -> Self {
        Self {
            application_cursor_keys: mode.contains(TermMode::APP_CURSOR),
            application_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse_reporting: first(mode, &REPORTING).unwrap_or(MouseReporting::Off),
            mouse_encoding: first(mode, &ENCODINGS).unwrap_or(MouseEncoding::Default),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
        }
    }
}

/// The mouse events the application asked to be reported; setting one
/// replaces another, and resetting it stops reporting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseReporting {
    /// No mouse event is reported.
    Off,
    /// `CSI ? 1000 h`: button presses and releases.
    Clicks,
    /// `CSI ? 1002 h`: also motion while a button is held.
    Drag,
    /// `CSI ? 1003 h`: also all other motion.
    Motion,
}

/// How mouse reports are encoded; setting one replaces the other, and
/// resetting it returns to the default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEncoding {
    /// Coordinates as single bytes (`CSI M`).
    Default,
    /// `CSI ? 1005 h`: coordinates as UTF-8 characters.
    Utf8,
    /// `CSI ? 1006 h`: SGR reports (`CSI <`).
    Sgr,
}

const REPORTING: [(TermMode, MouseReporting); 3] = [
    (TermMode::MOUSE_REPORT_CLICK, MouseReporting::Clicks),
    (TermMode::MOUSE_DRAG, MouseReporting::Drag),
    (TermMode::MOUSE_MOTION, MouseReporting::Motion),
];

const ENCODINGS: [(TermMode, MouseEncoding); 2] = [
    (TermMode::UTF8_MOUSE, MouseEncoding::Utf8),
    (TermMode::SGR_MOUSE, MouseEncoding::Sgr),
];

/// The value of the first of `table`'s modes that is set; the engine sets
/// at most one of each table's.
fn first<T: Copy>(mode: TermMode, table: &[(TermMode, T)]) -> Option<T> {
    table
        .iter()
        .find(|(flag, _)| mode.contains(*flag))
        .map(|(_, value)| *value)
}
