//! The input router (spec #73, "Input router"; #109): pure, with no I/O. It
//! classifies raw host bytes into keys, prefix actions and the terminal
//! events that bypass them, and never translates a key into a lossy event:
//! a key that reaches an application is its host bytes, re-encoded only for
//! the application's cursor and keypad modes.
//!
//! Classification never depends on how the host splits its reads: a
//! sequence waits for its next byte whatever read brings it. Only a lone
//! Escape is ambiguous, because it is also how every sequence begins; it is
//! a bare Escape once the host has been quiet for [`ESCAPE_TIME`], and the
//! byte after it decides otherwise.
use crate::keys::{Action, action};
use climon2_core::keys::ControlKey;
use climon2_vt::view::Modes;
use std::time::Duration;

/// How long the host stays quiet after a lone Escape before it is a bare
/// Escape rather than the start of a sequence.
pub(crate) const ESCAPE_TIME: Duration = Duration::from_millis(50);

/// A key, as the host sent it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    /// A bare Escape.
    Escape,
    /// An unmodified cursor key, by its final byte: A, B, C or D, or H or F
    /// (Home and End). It is sent as each application's cursor mode expects.
    Cursor(u8),
    /// An application keypad key (`ESC O` and its final byte). It is sent as
    /// each application's keypad mode expects.
    Keypad(u8),
    /// Any other key, exactly as the host sent it.
    Bytes(Vec<u8>),
}

/// What host input means.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Routed {
    /// The prefix and the key that completes a global action.
    Action(Action),
    /// A key outside a prefix, or the one literal prefix a doubled prefix is.
    Key(Key),
    /// The prefix byte and a key that completes no action: an application
    /// receives both, a modal neither.
    Unknown(u8, Key),
}

/// Classifies host input, however the host splits its reads.
pub(crate) struct Router {
    prefix: u8,
    /// A lone prefix waits for the key after it.
    pending: bool,
    scan: Scan,
    /// The latest pasted bytes, to find the paste's end marker.
    recent: [u8; MARKER],
}

/// Where the host's bytes stand.
#[derive(Debug, Default)]
enum Scan {
    #[default]
    Ground,
    /// A lone Escape: the next byte, or the escape time, decides.
    Escape,
    /// A control sequence after `ESC` and its introducer (`[` or `O`): its
    /// bytes so far, or none while an overlong one is skipped.
    Sequence(u8, Option<Vec<u8>>),
    /// An OSC, DCS or APC string, which only a host reply sends; true just
    /// after an Escape inside it.
    Text(bool),
    /// A bracketed paste, until its end marker.
    Paste,
}

/// What the scan finds.
#[derive(Debug, Eq, PartialEq)]
enum Token {
    Key(Key),
    /// A bracketed paste began.
    Paste,
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// No key's control sequence is longer; a longer one is skipped through its
/// final byte, never forwarded.
const SEQUENCE_BYTES: usize = 64;
const PASTE_START: &[u8] = b"200~";
const PASTE_END: &[u8; MARKER] = b"\x1b[201~";
const MARKER: usize = 6;
/// The final bytes of the cursor keys that DECCKM changes.
const CURSOR: &[u8] = b"ABCDHF";
/// Each application keypad key's final byte, and what the key sends in
/// numeric keypad mode.
const KEYPAD: [(u8, u8); 18] = [
    (b'p', b'0'),
    (b'q', b'1'),
    (b'r', b'2'),
    (b's', b'3'),
    (b't', b'4'),
    (b'u', b'5'),
    (b'v', b'6'),
    (b'w', b'7'),
    (b'x', b'8'),
    (b'y', b'9'),
    (b'j', b'*'),
    (b'k', b'+'),
    (b'l', b','),
    (b'm', b'-'),
    (b'n', b'.'),
    (b'o', b'/'),
    (b'M', b'\r'),
    (b'X', b'='),
];
/// Control sequences that are never keys, by their bytes after `ESC [`: SGR
/// mouse reports and replies with a private marker, replies with an
/// intermediate byte or a reply's final byte (device attributes, status and
/// window reports), focus reports and a paste end without its start.
const BYPASS: [fn(&[u8]) -> bool; 4] = [
    |body| b"<=>?".contains(&body[0]),
    |body| body.iter().any(|byte| (0x20..=0x2f).contains(byte)),
    |body| body.last().is_some_and(|last| b"cnty".contains(last)),
    |body| [&b"I"[..], b"O", b"201~"].contains(&body),
];

impl Router {
    pub(crate) fn new(prefix: ControlKey) -> Self {
        Self {
            prefix: prefix.byte(),
            pending: false,
            scan: Scan::Ground,
            recent: [0; MARKER],
        }
    }

    /// What one host read means, in order.
    pub(crate) fn read(&mut self, bytes: &[u8]) -> Vec<Routed> {
        let mut tokens = Vec::new();
        for &byte in bytes {
            self.scan(byte, &mut tokens);
        }
        self.route(tokens)
    }

    /// What the host's silence for `quiet` since its last read means: a
    /// lone Escape is bare once `quiet` reaches the escape time.
    pub(crate) fn idle(&mut self, quiet: Duration) -> Vec<Routed> {
        if !matches!(self.scan, Scan::Escape) || quiet < ESCAPE_TIME {
            return Vec::new();
        }
        self.scan = Scan::Ground;
        self.route(vec![Token::Key(Key::Escape)])
    }

    fn route(&mut self, tokens: Vec<Token>) -> Vec<Routed> {
        tokens
            .into_iter()
            .filter_map(|token| self.token(token))
            .collect()
    }

    /// Applies the prefix: a paste discards a pending one.
    fn token(&mut self, token: Token) -> Option<Routed> {
        let Token::Key(key) = token else {
            self.pending = false;
            return None;
        };
        let pending = std::mem::take(&mut self.pending);
        match (pending, self.is_prefix(&key)) {
            (false, true) => {
                self.pending = true;
                None
            }
            (false, false) => Some(Routed::Key(key)),
            (true, true) => Some(Routed::Key(Key::Bytes(vec![self.prefix]))),
            (true, false) => Some(self.suffix(key)),
        }
    }

    /// Whether `key` is the prefix: with Ctrl+[ as the prefix, a bare
    /// Escape is.
    fn is_prefix(&self, key: &Key) -> bool {
        match key {
            Key::Escape => self.prefix == ESC,
            Key::Bytes(bytes) => bytes[..] == [self.prefix],
            _ => false,
        }
    }

    /// The action `key` completes after the prefix, or the prefix and it.
    fn suffix(&self, key: Key) -> Routed {
        let action = match &key {
            Key::Bytes(bytes) if bytes.len() == 1 => action(bytes[0]),
            _ => None,
        };
        action.map_or(Routed::Unknown(self.prefix, key), Routed::Action)
    }

    fn scan(&mut self, byte: u8, tokens: &mut Vec<Token>) {
        match std::mem::take(&mut self.scan) {
            Scan::Ground => self.ground(byte, tokens),
            Scan::Escape => self.escaped(byte, tokens),
            Scan::Sequence(introducer, body) => self.sequence(introducer, body, byte, tokens),
            data => self.data(data, byte, tokens),
        }
    }

    /// A byte of a host reply's string or of a paste.
    fn data(&mut self, data: Scan, byte: u8, tokens: &mut Vec<Token>) {
        if let Scan::Text(escaped) = data {
            self.text(escaped, byte, tokens);
        } else {
            self.pasted(byte);
        }
    }

    fn ground(&mut self, byte: u8, tokens: &mut Vec<Token>) {
        if byte == ESC {
            self.scan = Scan::Escape;
        } else {
            tokens.push(Token::Key(Key::Bytes(vec![byte])));
        }
    }

    /// The byte after a lone Escape: a sequence's introducer, or the next
    /// key after a bare Escape.
    fn escaped(&mut self, byte: u8, tokens: &mut Vec<Token>) {
        match byte {
            b'[' | b'O' => self.scan = Scan::Sequence(byte, Some(Vec::new())),
            b']' | b'P' | b'_' => self.scan = Scan::Text(false),
            _ => {
                tokens.push(Token::Key(Key::Escape));
                self.ground(byte, tokens);
            }
        }
    }

    /// A control or non-ASCII byte breaks a sequence: its bytes so far are a
    /// key, and the byte is read afresh.
    fn sequence(
        &mut self,
        introducer: u8,
        body: Option<Vec<u8>>,
        byte: u8,
        tokens: &mut Vec<Token>,
    ) {
        if !(0x20..=0x7e).contains(&byte) {
            tokens.extend(body.map(|body| literal(introducer, &body)));
            return self.ground(byte, tokens);
        }
        let ends = introducer == b'O' || (0x40..=0x7e).contains(&byte);
        match body {
            Some(mut body) => {
                body.push(byte);
                self.grown(introducer, body, ends, tokens);
            }
            None => self.skipped(introducer, ends),
        }
    }

    /// An overlong sequence is skipped through its final byte.
    fn skipped(&mut self, introducer: u8, ends: bool) {
        if !ends {
            self.scan = Scan::Sequence(introducer, None);
        }
    }

    /// A sequence one byte longer: complete at its final byte, otherwise
    /// waiting for more unless it is now too long for any key.
    fn grown(&mut self, introducer: u8, body: Vec<u8>, ends: bool, tokens: &mut Vec<Token>) {
        if ends {
            return self.complete(introducer, &body, tokens);
        }
        let kept = (body.len() < SEQUENCE_BYTES).then_some(body);
        self.scan = Scan::Sequence(introducer, kept);
    }

    fn complete(&mut self, introducer: u8, body: &[u8], tokens: &mut Vec<Token>) {
        let token = if introducer == b'O' {
            Some(Token::Key(ss3(body[0])))
        } else {
            csi(body)
        };
        if token == Some(Token::Paste) {
            self.scan = Scan::Paste;
            self.recent = [0; MARKER];
        }
        tokens.extend(token);
    }

    /// A host reply's string ends at BEL or ST; an Escape followed by
    /// anything else begins a new sequence.
    fn text(&mut self, escaped: bool, byte: u8, tokens: &mut Vec<Token>) {
        match (escaped, byte) {
            (_, BEL) | (true, b'\\') => {}
            (true, _) => self.escaped(byte, tokens),
            (false, _) => self.scan = Scan::Text(byte == ESC),
        }
    }

    /// Pasted bytes are data until the end marker.
    fn pasted(&mut self, byte: u8) {
        self.recent.rotate_left(1);
        self.recent[MARKER - 1] = byte;
        if self.recent != *PASTE_END {
            self.scan = Scan::Paste;
        }
    }
}

/// A complete `ESC [` sequence: a paste's start, nothing for a sequence
/// that bypasses keys, a cursor key, or any other key as its bytes.
fn csi(body: &[u8]) -> Option<Token> {
    if body == PASTE_START {
        return Some(Token::Paste);
    }
    if BYPASS.iter().any(|bypasses| bypasses(body)) {
        return None;
    }
    let key = match body {
        [last] if CURSOR.contains(last) => Key::Cursor(*last),
        _ => Key::Bytes([&[ESC, b'['], body].concat()),
    };
    Some(Token::Key(key))
}

/// A complete `ESC O` key.
fn ss3(last: u8) -> Key {
    if CURSOR.contains(&last) {
        Key::Cursor(last)
    } else if KEYPAD.iter().any(|(key, _)| *key == last) {
        Key::Keypad(last)
    } else {
        Key::Bytes(vec![ESC, b'O', last])
    }
}

/// A broken sequence's bytes, as one key.
fn literal(introducer: u8, body: &[u8]) -> Token {
    Token::Key(Key::Bytes([&[ESC, introducer], body].concat()))
}

impl Key {
    /// Appends the key as an application in `modes` expects it.
    pub(crate) fn encode(&self, modes: &Modes, out: &mut Vec<u8>) {
        match self {
            Self::Escape => out.push(ESC),
            Self::Cursor(last) => cursor(*last, modes.application_cursor_keys, out),
            Self::Keypad(last) => keypad(*last, modes.application_keypad, out),
            Self::Bytes(bytes) => out.extend_from_slice(bytes),
        }
    }
}

/// A cursor key as an application expects it: `ESC O` in application cursor
/// mode, otherwise `ESC [`.
fn cursor(last: u8, application: bool, out: &mut Vec<u8>) {
    let introducer = if application { b'O' } else { b'[' };
    out.extend_from_slice(&[ESC, introducer, last]);
}

/// An application keypad key as an application expects it: unchanged in
/// application keypad mode, otherwise what the key sends in numeric mode.
fn keypad(last: u8, application: bool, out: &mut Vec<u8>) {
    let numeric = KEYPAD
        .iter()
        .find(|(key, _)| *key == last)
        .map(|(_, numeric)| *numeric);
    match numeric.filter(|_| !application) {
        Some(numeric) => out.push(numeric),
        None => out.extend_from_slice(&[ESC, b'O', last]),
    }
}

impl Routed {
    /// Appends what an application in `modes` receives for this, if
    /// anything: every key, and the prefix with a key that completes no
    /// action.
    pub(crate) fn encode(&self, modes: &Modes, out: &mut Vec<u8>) {
        match self {
            Self::Action(_) => {}
            Self::Key(key) => key.encode(modes, out),
            Self::Unknown(prefix, key) => {
                out.push(*prefix);
                key.encode(modes, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ESCAPE_TIME, Key, Routed, Router};
    use crate::keys::Action;
    use climon2_core::keys::ControlKey;
    use climon2_vt::view::{Modes, MouseEncoding, MouseReporting};
    use std::time::Duration;

    const EXIT: Routed = Routed::Action(Action::Exit);
    const JUMP: Routed = Routed::Action(Action::Jump);
    const LIST: Routed = Routed::Action(Action::SessionList);
    const HELP: Routed = Routed::Action(Action::KeyIntroduction);
    const ESCAPE: Routed = Routed::Key(Key::Escape);

    fn router(prefix: &str) -> Router {
        Router::new(ControlKey::from_name(prefix).unwrap())
    }

    fn bytes(bytes: &[u8]) -> Routed {
        Routed::Key(Key::Bytes(bytes.to_vec()))
    }

    fn typed(text: &[u8]) -> Vec<Routed> {
        text.iter().map(|byte| bytes(&[*byte])).collect()
    }

    fn modes(cursor: bool, keypad: bool) -> Modes {
        Modes {
            application_cursor_keys: cursor,
            application_keypad: keypad,
            bracketed_paste: false,
            mouse_reporting: MouseReporting::Off,
            mouse_encoding: MouseEncoding::Default,
            focus_reporting: false,
            alternate_scroll: false,
        }
    }

    /// What an application in `modes` receives for `routed`.
    fn sent(routed: &[Routed], modes: &Modes) -> Vec<u8> {
        let mut out = Vec::new();
        for routed in routed {
            routed.encode(modes, &mut out);
        }
        out
    }

    impl Router {
        /// Everything `reads` mean, read one after another.
        fn reads(&mut self, reads: &[&[u8]]) -> Vec<Routed> {
            reads.iter().flat_map(|read| self.read(read)).collect()
        }
    }

    /// Every way to split `input` into two reads, and one read per byte.
    fn splits(input: &[u8]) -> Vec<Vec<&[u8]>> {
        let mut splits: Vec<Vec<&[u8]>> = (0..=input.len())
            .map(|at| vec![&input[..at], &input[at..]])
            .collect();
        splits.push(input.chunks(1).collect());
        splits
    }

    /// INP-01 (pure input-router supplement): prefix+s, x, j and ? are the
    /// global actions, with the default prefix and a custom one; only the
    /// configured prefix begins one.
    #[test]
    fn the_prefix_and_lowercase_s_x_j_or_question_mark_are_the_global_actions() {
        assert_eq!(
            router("Ctrl+]").read(b"\x1ds\x1dx\x1dj\x1d?"),
            [LIST, EXIT, JUMP, HELP]
        );
        assert_eq!(router("Ctrl+B").read(b"\x02s\x02x"), [LIST, EXIT]);
        assert_eq!(router("Ctrl+B").read(b"\x1dx"), typed(b"\x1dx"));
    }

    /// INP-01 (pure input-router supplement): a doubled prefix is one literal
    /// prefix, and the key after it is an ordinary key again.
    #[test]
    fn a_doubled_prefix_is_one_literal_prefix() {
        assert_eq!(
            router("Ctrl+]").read(b"\x1d\x1dx\x1d\x1d\x1d\x1d"),
            typed(b"\x1dx\x1d\x1d")
        );
    }

    /// INP-01 (pure input-router supplement): an unknown suffix, including
    /// d, h, r, uppercase J, S, X, the take-control key (Ctrl+T), Escape
    /// and a whole cursor key, is the prefix and that key, never an action;
    /// the key after it is ordinary again.
    #[test]
    fn an_unknown_suffix_is_the_prefix_and_that_key() {
        let mut router = router("Ctrl+]");
        let unknown: Vec<_> = b"dhrJSX\x14 \r"
            .iter()
            .map(|byte| Routed::Unknown(0x1d, Key::Bytes(vec![*byte])))
            .collect();
        let read: Vec<u8> = b"dhrJSX\x14 \r"
            .iter()
            .flat_map(|byte| [0x1d, *byte])
            .collect();
        assert_eq!(router.read(&read), unknown);
        assert_eq!(
            router.read(b"\x1d\x1b[Ax"),
            [Routed::Unknown(0x1d, Key::Cursor(b'A')), bytes(b"x")]
        );
        assert_eq!(
            router.read(b"\x1d\x1bx"),
            [Routed::Unknown(0x1d, Key::Escape), bytes(b"x")]
        );
    }

    /// INP-01 (pure input-router supplement): a lone prefix waits, through
    /// any silence, for the key after it.
    #[test]
    fn a_lone_prefix_waits_indefinitely() {
        let mut router = router("Ctrl+]");
        assert_eq!(router.read(b"\x1d"), []);
        assert_eq!(router.idle(Duration::from_secs(3600)), []);
        assert_eq!(router.read(b"x"), [EXIT]);
    }

    /// INP-02 (pure input-router supplement): prefixes, actions, keys,
    /// sequences, reports and pastes mean the same however the host splits
    /// its reads, down to one byte per read.
    #[test]
    fn every_split_of_the_reads_routes_identically() {
        let input: &[u8] = b"a\x1d\x1d\x1ds\x1b[A\x1bOB\x1b[1;5C\x1b[3~\x1bOP\x1bOp\x03\
            \x1b[<0;5;5M\x1b[I\x1b[?1;2c\x1b]11;rgb:0/0/0\x07\x1bP1$r0m\x1b\\\
            \x1b[200~\x1dx\x1b[201~\x1b\x1bx\x1dJ\xff\x00";
        let whole = router("Ctrl+]").read(input);
        assert_eq!(
            whole,
            [
                bytes(b"a"),
                bytes(b"\x1d"),
                LIST,
                Routed::Key(Key::Cursor(b'A')),
                Routed::Key(Key::Cursor(b'B')),
                bytes(b"\x1b[1;5C"),
                bytes(b"\x1b[3~"),
                bytes(b"\x1bOP"),
                Routed::Key(Key::Keypad(b'p')),
                bytes(b"\x03"),
                ESCAPE,
                ESCAPE,
                bytes(b"x"),
                Routed::Unknown(0x1d, Key::Bytes(b"J".to_vec())),
                bytes(b"\xff"),
                bytes(b"\x00"),
            ]
        );
        for reads in splits(input) {
            assert_eq!(router("Ctrl+]").reads(&reads), whole, "{reads:?}");
        }
    }

    /// INP-02 (pure input-router supplement): a cursor key split across two
    /// reads is still one cursor key, never an Escape, however long the
    /// host is quiet between its bytes once its introducer has arrived.
    #[test]
    fn a_split_cursor_key_is_never_an_escape() {
        let mut router = router("Ctrl+]");
        assert_eq!(router.read(b"\x1b["), []);
        assert_eq!(router.idle(Duration::from_secs(3600)), []);
        assert_eq!(router.read(b"B"), [Routed::Key(Key::Cursor(b'B'))]);
    }

    /// INP-02 (pure input-router supplement): Escape and a key in one read are
    /// Escape, then that key: identical to Alt and that key.
    #[test]
    fn escape_and_a_key_in_one_read_are_escape_then_that_key() {
        assert_eq!(router("Ctrl+]").read(b"\x1bx"), [ESCAPE, bytes(b"x")]);
        assert_eq!(router("Ctrl+]").read(b"\x1b\x1dx"), [ESCAPE, EXIT]);
    }

    /// INP-02 (pure input-router supplement): a lone Escape is bare once the
    /// host has been quiet for exactly the escape time, and not a moment
    /// before.
    #[test]
    fn a_lone_escape_is_bare_after_the_escape_time() {
        let mut router = router("Ctrl+]");
        assert_eq!(router.read(b"\x1b"), []);
        let before = ESCAPE_TIME - Duration::from_millis(1);
        assert_eq!(router.idle(before), []);
        assert_eq!(router.idle(ESCAPE_TIME), [ESCAPE]);
        assert_eq!(router.idle(ESCAPE_TIME), []);
        assert_eq!(router.read(b"[A"), typed(b"[A"));
    }

    /// INP-02 (pure input-router supplement), with Ctrl+[ as the prefix: a
    /// bare Escape waits as the prefix, the legacy Alt+s, Alt+x, Alt+j and
    /// Alt+? encodings are the actions, a doubled Escape is one literal
    /// Escape (never a bare one), and cursor keys stay keys.
    #[test]
    fn with_escape_as_the_prefix_bare_escape_waits_and_alt_keys_are_actions() {
        let mut router = router("Ctrl+[");
        assert_eq!(
            router.read(b"\x1bs\x1bx\x1bj\x1b?"),
            [LIST, EXIT, JUMP, HELP]
        );
        assert_eq!(router.read(b"\x1b"), []);
        assert_eq!(router.idle(Duration::from_secs(3600)), []);
        assert_eq!(router.read(b"x"), [EXIT]);
        assert_eq!(router.read(b"\x1b\x1b"), []);
        assert_eq!(router.idle(ESCAPE_TIME), [bytes(b"\x1b")]);
        assert_eq!(
            router.read(b"\x1b[A\x1bd"),
            [
                Routed::Key(Key::Cursor(b'A')),
                Routed::Unknown(0x1b, Key::Bytes(b"d".to_vec()))
            ]
        );
    }

    /// TRM-02 and INP-05 (pure input-router supplement): host replies
    /// (device attributes, status, mode and window reports, OSC, DCS and APC
    /// strings), SGR mouse reports, focus reports and a stray paste end never
    /// reach shortcuts or applications, and never complete or clear a
    /// pending prefix.
    #[test]
    fn replies_reports_and_pastes_are_never_keys() {
        let mut router = router("Ctrl+]");
        let bypassing: &[&[u8]] = &[
            b"\x1b[?62;22c",
            b"\x1b[>1;10;0c",
            b"\x1b[0n",
            b"\x1b[?1;2$y",
            b"\x1b[8;24;80t",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\",
            b"\x1b]10;?\x07",
            b"\x1bP1$r0m\x1b\\",
            b"\x1b_Gi=1;OK\x1b\\",
            b"\x1b[<0;6;6M",
            b"\x1b[<0;6;6m",
            b"\x1b[I",
            b"\x1b[O",
            b"\x1b[201~",
        ];
        for reply in bypassing {
            assert_eq!(router.read(reply), [], "{reply:?}");
            assert_eq!(router.read(b"\x1d"), []);
            assert_eq!(router.read(reply), [], "{reply:?}");
            assert_eq!(router.read(b"x"), [EXIT], "{reply:?}");
        }
    }

    /// INP-03 (pure input-router supplement): a paste is never keys; it
    /// discards a pending prefix and leaves none pending, and only its end
    /// marker ends it.
    #[test]
    fn a_paste_discards_a_pending_prefix_and_leaves_none() {
        let mut router = router("Ctrl+]");
        assert_eq!(router.read(b"\x1d\x1b[200~x\x1b[201~x"), typed(b"x"));
        assert_eq!(router.read(b"\x1b[200~\x1d\x1b[201~x"), typed(b"x"));
        assert_eq!(
            router.read(b"\x1b[200~\x1b[200~\x1b[20\x1b]x\x1dx\x1b[A\r\x1b[201~y"),
            typed(b"y")
        );
    }

    /// INP-02 (pure input-router supplement): a sequence that a control
    /// byte breaks is its bytes so far, then that control; an Escape
    /// breaking a string starts a new sequence.
    #[test]
    fn a_broken_sequence_is_its_bytes() {
        assert_eq!(
            router("Ctrl+]").read(b"\x1b[1\x03\x1bO\x1dx"),
            [bytes(b"\x1b[1"), bytes(b"\x03"), bytes(b"\x1bO"), EXIT]
        );
        assert_eq!(
            router("Ctrl+]").read(b"\x1b]0;t\x1b[Bq"),
            [Routed::Key(Key::Cursor(b'B')), bytes(b"q")]
        );
    }

    /// INP-02 (pure input-router supplement): a control sequence longer than
    /// any key's is discarded at its final byte, at the limit and one past.
    #[test]
    fn an_overlong_sequence_is_discarded() {
        let at_limit = [&b"\x1b["[..], &[b'1'; 63], b"~"].concat();
        assert_eq!(router("Ctrl+]").read(&at_limit), [bytes(&at_limit)]);
        let past = [&b"\x1b["[..], &[b'1'; 64], b"~x"].concat();
        assert_eq!(router("Ctrl+]").read(&past), typed(b"x"));
    }

    /// INP-02 (pure input-router supplement): cursor keys follow the
    /// application's cursor mode, whichever form the host sent; modified
    /// cursor keys, function keys and everything else pass unchanged.
    #[test]
    fn cursor_keys_follow_the_application_cursor_mode() {
        let input = b"\x1b[A\x1bOB\x1b[C\x1bOD\x1b[H\x1bOF\x1b[1;2A\x1bOP\x1b[5~";
        let routed = router("Ctrl+]").read(input);
        assert_eq!(
            sent(&routed, &modes(false, false)),
            b"\x1b[A\x1b[B\x1b[C\x1b[D\x1b[H\x1b[F\x1b[1;2A\x1bOP\x1b[5~"
        );
        assert_eq!(
            sent(&routed, &modes(true, false)),
            b"\x1bOA\x1bOB\x1bOC\x1bOD\x1bOH\x1bOF\x1b[1;2A\x1bOP\x1b[5~"
        );
    }

    /// INP-02 (pure input-router supplement): application keypad keys follow
    /// the application's keypad mode: as sent in application mode, and as
    /// the digit, operator or Enter in numeric mode.
    #[test]
    fn keypad_keys_follow_the_application_keypad_mode() {
        let input = b"\x1bOp\x1bOq\x1bOr\x1bOs\x1bOt\x1bOu\x1bOv\x1bOw\x1bOx\x1bOy\
            \x1bOj\x1bOk\x1bOl\x1bOm\x1bOn\x1bOo\x1bOM\x1bOX";
        let routed = router("Ctrl+]").read(input);
        assert_eq!(sent(&routed, &modes(false, true)), input);
        assert_eq!(sent(&routed, &modes(false, false)), b"0123456789*+,-./\r=");
    }

    /// COM-04 and EXT-03 (pure input-router supplement): typed bytes reach an
    /// application exactly, NUL, Ctrl+C, invalid UTF-8 and all, with an
    /// unknown suffix as both its bytes and a doubled prefix as one.
    #[test]
    fn typed_bytes_reach_the_application_exactly() {
        let input = b"\x00\xff\x41\x03\xc3\x28\x1b\x1bx\x1dd\x1d\x1d\t\x7f\r";
        let routed = router("Ctrl+]").read(input);
        assert_eq!(
            sent(&routed, &modes(true, true)),
            b"\x00\xff\x41\x03\xc3\x28\x1b\x1bx\x1dd\x1d\t\x7f\r"
        );
        let actions = router("Ctrl+]").read(b"\x1ds\x1dj\x1d?");
        assert_eq!(sent(&actions, &modes(false, false)), b"");
    }
}
