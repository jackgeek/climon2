//! Terminal modes a host must match, as DECSET/DECRST and friends.

use alacritty_terminal::{
    term::TermMode,
    vte::ansi::{CursorShape, CursorStyle},
};

pub(crate) type Entry = (TermMode, &'static [u8], &'static [u8]);

/// Input and display modes every presentation carries to its host.
pub(crate) const SHARED: &[Entry] = &[
    (TermMode::SHOW_CURSOR, b"\x1b[?25h", b"\x1b[?25l"),
    (TermMode::APP_CURSOR, b"\x1b[?1h", b"\x1b[?1l"),
    (TermMode::APP_KEYPAD, b"\x1b=", b"\x1b>"),
    (TermMode::BRACKETED_PASTE, b"\x1b[?2004h", b"\x1b[?2004l"),
    (TermMode::MOUSE_REPORT_CLICK, b"\x1b[?1000h", b"\x1b[?1000l"),
    (TermMode::MOUSE_DRAG, b"\x1b[?1002h", b"\x1b[?1002l"),
    (TermMode::MOUSE_MOTION, b"\x1b[?1003h", b"\x1b[?1003l"),
    (TermMode::SGR_MOUSE, b"\x1b[?1006h", b"\x1b[?1006l"),
    (TermMode::UTF8_MOUSE, b"\x1b[?1005h", b"\x1b[?1005l"),
    (TermMode::FOCUS_IN_OUT, b"\x1b[?1004h", b"\x1b[?1004l"),
    (TermMode::ALTERNATE_SCROLL, b"\x1b[?1007h", b"\x1b[?1007l"),
];

/// Modes only Direct presentation carries; Derived output relies on the
/// prologue's values for them.
pub(crate) const DIRECT: &[Entry] = &[
    (TermMode::LINE_WRAP, b"\x1b[?7h", b"\x1b[?7l"),
    (TermMode::ORIGIN, b"\x1b[?6h", b"\x1b[?6l"),
    (TermMode::INSERT, b"\x1b[4h", b"\x1b[4l"),
    (TermMode::LINE_FEED_NEW_LINE, b"\x1b[20h", b"\x1b[20l"),
];

/// Emit every mode of `table` as the engine has it; resets precede sets so
/// exclusive groups such as mouse protocols end in the engine's state.
pub(crate) fn transitions(to: TermMode, table: &[Entry], out: &mut Vec<u8>) {
    let (sets, resets): (Vec<&Entry>, Vec<&Entry>) =
        table.iter().partition(|(mode, _, _)| to.contains(*mode));
    resets
        .iter()
        .for_each(|(_, _, reset)| out.extend_from_slice(reset));
    sets.iter()
        .for_each(|(_, set, _)| out.extend_from_slice(set));
}

/// Steady DECSCUSR codes; the blinking variant is one less.
const SHAPES: &[(CursorShape, u8)] = &[
    (CursorShape::Block, 2),
    (CursorShape::Underline, 4),
    (CursorShape::Beam, 6),
];

/// DECSCUSR for a cursor style; the engine's default style selects the
/// host's default (`CSI 0 SP q`).
pub(crate) fn cursor_style(style: CursorStyle) -> Vec<u8> {
    let code = SHAPES
        .iter()
        .find(|(shape, _)| *shape == style.shape)
        .filter(|_| style != CursorStyle::default())
        .map_or(0, |(_, steady)| steady - u8::from(style.blinking));
    format!("\x1b[{code} q").into_bytes()
}
