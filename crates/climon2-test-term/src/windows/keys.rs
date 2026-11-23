//! Keys as a Windows host sends them to its pseudoconsole.
//!
//! ConPTY asks its host for win32-input-mode with `CSI ? 9001 h` (and drops it
//! with `CSI ? 9001 l`). A host that honours it, as Windows Terminal does,
//! sends every key as console key records, `CSI Vk ; Sc ; Uc ; Kd ; Cs ; Rc _`
//! (microsoft/terminal spec #4999): virtual-key code, scan code, UTF-16
//! character, 1 for key down or 0 for up, control-key state and repeat count.
//! Otherwise it sends plain VT, as xterm documents it. The key table is a US
//! keyboard's, so the records are the same on every machine.

/// Modifier keys held while a key is pressed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
}

impl Modifiers {
    pub const NONE: Self = Self {
        shift: false,
        ctrl: false,
        alt: false,
    };
    pub const SHIFT: Self = Self {
        shift: true,
        ..Self::NONE
    };
    pub const CTRL: Self = Self {
        ctrl: true,
        ..Self::NONE
    };
    pub const ALT: Self = Self {
        alt: true,
        ..Self::NONE
    };

    /// The xterm modifier parameter: 1 plus Shift 1, Alt 2 and Ctrl 4.
    fn parameter(self) -> u8 {
        1 + u8::from(self.shift) + 2 * u8::from(self.alt) + 4 * u8::from(self.ctrl)
    }
}

/// A key on the keyboard. A character is the key that types it, with Shift
/// added when the US layout needs it (`'A'` is Shift+A).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    /// The Break key, pressed with Ctrl: Windows reports Ctrl+Break as
    /// `VK_CANCEL` (0x03) with scan code 0x46, typing ETX (0x03).
    Break,
}

/// Control-key state bits of a console key record.
const SHIFT_PRESSED: u32 = 0x10;
const LEFT_CTRL_PRESSED: u32 = 0x08;
const LEFT_ALT_PRESSED: u32 = 0x02;
const ENHANCED_KEY: u32 = 0x100;

/// A physical key: virtual-key code, scan code and whether it is an enhanced key.
#[derive(Clone, Copy)]
struct Physical {
    vk: u16,
    scan: u16,
    enhanced: bool,
}

const fn key(vk: u16, scan: u16) -> Physical {
    Physical {
        vk,
        scan,
        enhanced: false,
    }
}

const fn enhanced(vk: u16, scan: u16) -> Physical {
    Physical {
        vk,
        scan,
        enhanced: true,
    }
}

const SHIFT: Physical = key(0x10, 0x2A);
const CTRL: Physical = key(0x11, 0x1D);
const ALT: Physical = key(0x12, 0x38);

impl Key {
    /// The key, whether the US layout adds Shift, and the character it types
    /// unmodified (0 for none).
    fn physical(self) -> Option<(Physical, bool, u16)> {
        Some(match self {
            Self::Char(c) => {
                let (physical, shift) = us_key(c)?;
                (physical, shift, utf16_unit(c)?)
            }
            Self::Enter => (key(0x0D, 0x1C), false, 0x0D),
            Self::Tab => (key(0x09, 0x0F), false, 0x09),
            Self::Backspace => (key(0x08, 0x0E), false, 0x08),
            Self::Escape => (key(0x1B, 0x01), false, 0x1B),
            Self::Up => (enhanced(0x26, 0x48), false, 0),
            Self::Down => (enhanced(0x28, 0x50), false, 0),
            Self::Left => (enhanced(0x25, 0x4B), false, 0),
            Self::Right => (enhanced(0x27, 0x4D), false, 0),
            Self::Home => (enhanced(0x24, 0x47), false, 0),
            Self::End => (enhanced(0x23, 0x4F), false, 0),
            Self::Break => (key(0x03, 0x46), false, 0),
        })
    }
}

fn utf16_unit(c: char) -> Option<u16> {
    let mut units = [0; 2];
    match c.encode_utf16(&mut units) {
        [unit] => Some(*unit),
        _ => None,
    }
}

/// The US-layout key that types `c`, and whether it needs Shift.
fn us_key(c: char) -> Option<(Physical, bool)> {
    const LETTERS: [u16; 26] = [
        0x1E, 0x30, 0x2E, 0x20, 0x12, 0x21, 0x22, 0x23, 0x17, 0x24, 0x25, 0x26, 0x32, 0x31, 0x18,
        0x19, 0x10, 0x13, 0x1F, 0x14, 0x16, 0x2F, 0x11, 0x2D, 0x15, 0x2C,
    ];
    const DIGITS: &str = "1234567890";
    const SHIFTED_DIGITS: &str = "!@#$%^&*()";
    // (unshifted, shifted, virtual key, scan code)
    const SYMBOLS: [(char, char, u16, u16); 11] = [
        ('-', '_', 0xBD, 0x0C),
        ('=', '+', 0xBB, 0x0D),
        ('[', '{', 0xDB, 0x1A),
        (']', '}', 0xDD, 0x1B),
        ('\\', '|', 0xDC, 0x2B),
        (';', ':', 0xBA, 0x27),
        ('\'', '"', 0xDE, 0x28),
        (',', '<', 0xBC, 0x33),
        ('.', '>', 0xBE, 0x34),
        ('/', '?', 0xBF, 0x35),
        ('`', '~', 0xC0, 0x29),
    ];
    if c.is_ascii_alphabetic() {
        let upper = c.to_ascii_uppercase();
        let scan = LETTERS[usize::from(upper as u8 - b'A')];
        return Some((key(u16::from(upper as u8), scan), c.is_ascii_uppercase()));
    }
    if c == ' ' {
        return Some((key(0x20, 0x39), false));
    }
    let digit = |index: usize| {
        let vk = u16::from(DIGITS.as_bytes()[index]);
        key(vk, 0x02 + index as u16)
    };
    if let Some(index) = DIGITS.find(c) {
        return Some((digit(index), false));
    }
    if let Some(index) = SHIFTED_DIGITS.find(c) {
        return Some((digit(index), true));
    }
    SYMBOLS.iter().find_map(|&(plain, shifted, vk, scan)| {
        (c == plain || c == shifted).then_some((key(vk, scan), c == shifted))
    })
}

/// The character a key types with `modifiers` held: Ctrl makes a C0 control
/// of `@` to `_` and letters, and turns Enter into line feed and Backspace
/// into DEL.
fn typed(key: Key, modifiers: Modifiers, plain: u16) -> u16 {
    if !modifiers.ctrl {
        return plain;
    }
    match key {
        Key::Enter => 0x0A,
        Key::Backspace => 0x7F,
        Key::Break => 0x03,
        Key::Char(' ') => 0,
        Key::Char(c) if c.is_ascii_alphabetic() || ('@'..='_').contains(&c) => {
            u16::from(c.to_ascii_uppercase() as u8 & 0x1F)
        }
        _ => plain,
    }
}

fn record(out: &mut Vec<u8>, physical: Physical, character: u16, down: bool, state: u32) {
    let state = state | if physical.enhanced { ENHANCED_KEY } else { 0 };
    out.extend_from_slice(
        format!(
            "\x1b[{};{};{character};{};{state};1_",
            physical.vk,
            physical.scan,
            u8::from(down)
        )
        .as_bytes(),
    );
}

/// The win32-input-mode records for pressing and releasing `key` with
/// `modifiers` held, as a keyboard makes them: each modifier goes down in
/// turn, then the key goes down and up, then the modifiers go up in reverse.
/// A character outside the table (one with no US key) is sent as Windows
/// Terminal sends composed text: a key-down and key-up with no key, per UTF-16
/// unit.
pub(crate) fn win32(key: Key, modifiers: Modifiers) -> Vec<u8> {
    let mut out = Vec::new();
    let Some((physical, layout_shift, plain)) = key.physical() else {
        if let Key::Char(c) = key {
            let mut units = [0; 2];
            for unit in c.encode_utf16(&mut units) {
                record(&mut out, key_none(), *unit, true, 0);
                record(&mut out, key_none(), *unit, false, 0);
            }
        }
        return out;
    };
    let held = Modifiers {
        shift: modifiers.shift || layout_shift,
        ..modifiers
    };
    let pressed = [
        (held.ctrl, CTRL, LEFT_CTRL_PRESSED),
        (held.alt, ALT, LEFT_ALT_PRESSED),
        (held.shift, SHIFT, SHIFT_PRESSED),
    ];
    let mut state = 0;
    for &(_, modifier, bit) in pressed.iter().filter(|(on, ..)| *on) {
        state |= bit;
        record(&mut out, modifier, 0, true, state);
    }
    let character = typed(key, modifiers, plain);
    record(&mut out, physical, character, true, state);
    record(&mut out, physical, character, false, state);
    for &(_, modifier, bit) in pressed.iter().rev().filter(|(on, ..)| *on) {
        state &= !bit;
        record(&mut out, modifier, 0, false, state);
    }
    out
}

fn key_none() -> Physical {
    key(0, 0)
}

/// The plain VT a host sends for `key` with `modifiers` held, as xterm does.
/// `application_cursor` is DECCKM (`CSI ? 1 h`): unmodified arrows, Home and
/// End are sent as SS3 instead of CSI.
pub(crate) fn vt(key: Key, modifiers: Modifiers, application_cursor: bool) -> Vec<u8> {
    let cursor = |last: u8| {
        if modifiers.parameter() > 1 {
            format!("\x1b[1;{}{}", modifiers.parameter(), last as char).into_bytes()
        } else if application_cursor {
            vec![0x1b, b'O', last]
        } else {
            vec![0x1b, b'[', last]
        }
    };
    let plain: Vec<u8> = match key {
        Key::Up => return cursor(b'A'),
        Key::Down => return cursor(b'B'),
        Key::Right => return cursor(b'C'),
        Key::Left => return cursor(b'D'),
        Key::Home => return cursor(b'H'),
        Key::End => return cursor(b'F'),
        // VT has no Break: a plain-VT host such as the VS Code terminal
        // sends nothing for it.
        Key::Break => return Vec::new(),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Enter => b"\r".to_vec(),
        Key::Escape => b"\x1b".to_vec(),
        Key::Backspace if modifiers.ctrl => b"\x08".to_vec(),
        Key::Backspace => b"\x7f".to_vec(),
        Key::Char(c) if modifiers.ctrl => match typed(key, modifiers, 0) {
            0 if c != ' ' && c != '@' => c.to_string().into_bytes(),
            control => vec![control as u8],
        },
        Key::Char(c) => c.to_string().into_bytes(),
    };
    if modifiers.alt {
        [b"\x1b".as_slice(), &plain].concat()
    } else {
        plain
    }
}

/// Follows a pseudoconsole's output for `CSI ? 9001 h` and `CSI ? 9001 l`,
/// across reads that split a sequence.
#[derive(Debug, Default)]
pub(crate) struct Win32InputMode {
    on: bool,
    state: Scan,
    /// Parameter and intermediate bytes of the CSI sequence being read.
    pending: Vec<u8>,
}

#[derive(Debug, Default, PartialEq, Eq)]
enum Scan {
    #[default]
    Ground,
    Escape,
    Csi,
}

impl Win32InputMode {
    /// Whether the last mode set or reset seen asked for win32-input-mode.
    pub(crate) fn on(&self) -> bool {
        self.on
    }

    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state = match (&self.state, byte) {
                (_, 0x1B) => Scan::Escape,
                (Scan::Escape, b'[') => {
                    self.pending.clear();
                    Scan::Csi
                }
                (Scan::Csi, 0x20..=0x3F) => {
                    if self.pending.len() < 64 {
                        self.pending.push(byte);
                    }
                    Scan::Csi
                }
                (Scan::Csi, b'h' | b'l') => {
                    self.finish(byte == b'h');
                    Scan::Ground
                }
                (Scan::Csi, 0x00..=0x1F) => Scan::Csi,
                _ => Scan::Ground,
            };
        }
    }

    fn finish(&mut self, set: bool) {
        if let Some(parameters) = self.pending.strip_prefix(b"?")
            && parameters
                .split(|&byte| byte == b';')
                .any(|mode| mode == b"9001")
        {
            self.on = set;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn a_key_is_a_down_and_an_up_record() {
        // Spec #4999's example: 'a' is VK 0x41, scan 0x1E, character 97.
        assert_eq!(
            text(win32(Key::Char('a'), Modifiers::NONE)),
            "\x1b[65;30;97;1;0;1_\x1b[65;30;97;0;0;1_"
        );
        assert_eq!(
            text(win32(Key::Enter, Modifiers::NONE)),
            "\x1b[13;28;13;1;0;1_\x1b[13;28;13;0;0;1_"
        );
    }

    #[test]
    fn modifiers_go_down_around_the_key_and_set_its_state() {
        assert_eq!(
            text(win32(Key::Enter, Modifiers::SHIFT)),
            "\x1b[16;42;0;1;16;1_\x1b[13;28;13;1;16;1_\x1b[13;28;13;0;16;1_\x1b[16;42;0;0;0;1_"
        );
        // Ctrl+C types ETX, 0x03.
        assert_eq!(
            text(win32(Key::Char('c'), Modifiers::CTRL)),
            "\x1b[17;29;0;1;8;1_\x1b[67;46;3;1;8;1_\x1b[67;46;3;0;8;1_\x1b[17;29;0;0;0;1_"
        );
        // A capital needs Shift on a US keyboard.
        assert_eq!(
            text(win32(Key::Char('?'), Modifiers::NONE)),
            "\x1b[16;42;0;1;16;1_\x1b[191;53;63;1;16;1_\x1b[191;53;63;0;16;1_\x1b[16;42;0;0;0;1_"
        );
    }

    #[test]
    fn ctrl_break_is_vk_cancel_typing_etx_and_has_no_vt() {
        assert_eq!(
            text(win32(Key::Break, Modifiers::CTRL)),
            "\x1b[17;29;0;1;8;1_\x1b[3;70;3;1;8;1_\x1b[3;70;3;0;8;1_\x1b[17;29;0;0;0;1_"
        );
        assert_eq!(vt(Key::Break, Modifiers::CTRL, false), b"");
    }

    #[test]
    fn cursor_keys_are_enhanced_and_type_nothing() {
        assert_eq!(
            text(win32(Key::Up, Modifiers::NONE)),
            "\x1b[38;72;0;1;256;1_\x1b[38;72;0;0;256;1_"
        );
    }

    #[test]
    fn a_character_without_a_key_is_sent_per_utf16_unit() {
        assert_eq!(
            text(win32(Key::Char('é'), Modifiers::NONE)),
            "\x1b[0;0;233;1;0;1_\x1b[0;0;233;0;0;1_"
        );
        assert_eq!(
            text(win32(Key::Char('😀'), Modifiers::NONE)),
            "\x1b[0;0;55357;1;0;1_\x1b[0;0;55357;0;0;1_\x1b[0;0;56832;1;0;1_\x1b[0;0;56832;0;0;1_"
        );
    }

    #[test]
    fn plain_vt_follows_xterm() {
        assert_eq!(vt(Key::Up, Modifiers::NONE, false), b"\x1b[A");
        assert_eq!(vt(Key::Up, Modifiers::NONE, true), b"\x1bOA");
        assert_eq!(vt(Key::Right, Modifiers::CTRL, true), b"\x1b[1;5C");
        assert_eq!(vt(Key::Enter, Modifiers::SHIFT, false), b"\r");
        assert_eq!(vt(Key::Char('c'), Modifiers::CTRL, false), b"\x03");
        assert_eq!(vt(Key::Char('x'), Modifiers::ALT, false), b"\x1bx");
        assert_eq!(vt(Key::Tab, Modifiers::SHIFT, false), b"\x1b[Z");
        assert_eq!(vt(Key::Backspace, Modifiers::NONE, false), b"\x7f");
        assert_eq!(vt(Key::Char('é'), Modifiers::NONE, false), "é".as_bytes());
    }

    #[test]
    fn the_mode_follows_9001_sets_and_resets_split_across_reads() {
        let mut mode = Win32InputMode::default();
        mode.feed(b"\x1b[1t\x1b[c\x1b[?1004h\x1b[?90");
        assert!(!mode.on());
        mode.feed(b"01h");
        assert!(mode.on());
        mode.feed(b"\x1b[?1004l\x1b[9001l");
        assert!(mode.on(), "a non-private 9001 is another mode");
        mode.feed(b"\x1b[?1;9001l");
        assert!(!mode.on());
        mode.feed(b"\x1b[?9001\x1bh");
        assert!(!mode.on(), "an interrupted sequence sets nothing");
    }
}
