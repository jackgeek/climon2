//! The front filter's tokenizer. It mirrors the state machine of vte 0.15.0
//! (the parser inside `alacritty_terminal` 0.26.0) so that every token it
//! completes is exactly a unit the engine would act on, and holds partial
//! tokens across reads.

/// Largest control-string or CSI body kept; a longer token is dropped.
pub const MAX_BODY: usize = 4096;
/// Introducer bytes held before a body (`ESC [`, `ESC ]`, `ESC P`, ...).
const LIMIT: usize = MAX_BODY + 2;
const ESC: u8 = 0x1B;
const ST: &[u8] = b"\x1b\\";

/// The kind of a complete token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Printable text and C0 controls, never ending inside a UTF-8 sequence.
    Text,
    /// `ESC` with intermediates and a final byte.
    Escape,
    /// `ESC [` with parameters, intermediates and a final byte.
    Csi,
    /// `ESC ]`, always ending in BEL or ST.
    Osc,
    /// `ESC P`, always ending in ST (or 0x9C).
    Dcs,
    /// `ESC X`, `ESC ^` or `ESC _`, always ending in ST.
    Sos,
    /// A CSI or control string whose body exceeded [`MAX_BODY`]; it carries
    /// no bytes and must reach neither the engine nor a host.
    Oversized,
}

/// One complete token. `bytes` are what the engine is fed: escape sequences
/// lose bytes the parser ignores, and a control string the parser ended
/// without ST (by CAN, SUB or another escape) gains a synthetic ST.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub kind: Kind,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    EscInter,
    Csi,
    DcsEntry,
    DcsParam,
    DcsInter,
    DcsIgnore,
    DcsPass,
    Osc,
    Sos,
    StrEsc,
}

impl State {
    const fn bit(self) -> u16 {
        1 << self as u16
    }
}

const GROUND: u16 = State::Ground.bit();
const ESCAPE: u16 = State::Escape.bit();
const ESC_INTER: u16 = State::EscInter.bit();
const CSI: u16 = State::Csi.bit();
const DCS_ENTRY: u16 = State::DcsEntry.bit();
const DCS_PARAM: u16 = State::DcsParam.bit();
const DCS_INTER: u16 = State::DcsInter.bit();
const DCS_PASS: u16 = State::DcsPass.bit();
const OSC: u16 = State::Osc.bit();
const STR_ESC: u16 = State::StrEsc.bit();
const SEQUENCE: u16 = ESCAPE | ESC_INTER | CSI;
const STRING: u16 =
    DCS_ENTRY | DCS_PARAM | DCS_INTER | State::DcsIgnore.bit() | DCS_PASS | OSC | State::Sos.bit();

type Action = fn(&mut Tokenizer, u8, &mut Vec<Token>);

struct Rule {
    states: u16,
    lo: u8,
    hi: u8,
    action: Action,
    next: Option<State>,
}

const fn rule(states: u16, lo: u8, hi: u8, action: Action, next: Option<State>) -> Rule {
    Rule {
        states,
        lo,
        hi,
        action,
        next,
    }
}

/// Transitions in priority order; the first rule covering the state and byte
/// applies. Every state ends with a catch-all, so a rule always matches.
const RULES: &[Rule] = &[
    rule(
        GROUND,
        ESC,
        ESC,
        Tokenizer::begin_escape,
        Some(State::Escape),
    ),
    rule(GROUND, 0x00, 0xFF, Tokenizer::text, None),
    rule(SEQUENCE, 0x18, 0x18, Tokenizer::abort, Some(State::Ground)),
    rule(SEQUENCE, 0x1A, 0x1A, Tokenizer::abort, Some(State::Ground)),
    rule(SEQUENCE, ESC, ESC, Tokenizer::restart, Some(State::Escape)),
    rule(SEQUENCE, 0x00, 0x1F, Tokenizer::execute, None),
    rule(
        ESCAPE | ESC_INTER,
        0x20,
        0x2F,
        Tokenizer::hold,
        Some(State::EscInter),
    ),
    rule(ESCAPE, b'P', b'P', Tokenizer::hold, Some(State::DcsEntry)),
    rule(ESCAPE, b'X', b'X', Tokenizer::hold, Some(State::Sos)),
    rule(ESCAPE, b'[', b'[', Tokenizer::hold, Some(State::Csi)),
    rule(ESCAPE, b']', b']', Tokenizer::hold, Some(State::Osc)),
    rule(ESCAPE, b'^', b'_', Tokenizer::hold, Some(State::Sos)),
    rule(
        ESCAPE | ESC_INTER,
        0x30,
        0x7E,
        Tokenizer::finish,
        Some(State::Ground),
    ),
    rule(CSI, 0x20, 0x3F, Tokenizer::hold, None),
    rule(CSI, 0x40, 0x7E, Tokenizer::finish, Some(State::Ground)),
    rule(SEQUENCE, 0x00, 0xFF, Tokenizer::ignore, None),
    rule(
        STRING,
        0x18,
        0x18,
        Tokenizer::cancel_string,
        Some(State::Ground),
    ),
    rule(
        STRING,
        0x1A,
        0x1A,
        Tokenizer::cancel_string,
        Some(State::Ground),
    ),
    rule(STRING, ESC, ESC, Tokenizer::ignore, Some(State::StrEsc)),
    rule(OSC, 0x07, 0x07, Tokenizer::finish, Some(State::Ground)),
    rule(DCS_PASS, 0x9C, 0x9C, Tokenizer::finish, Some(State::Ground)),
    rule(
        DCS_ENTRY | DCS_PARAM | DCS_INTER,
        0x40,
        0x7E,
        Tokenizer::hold,
        Some(State::DcsPass),
    ),
    rule(
        DCS_ENTRY | DCS_PARAM,
        0x20,
        0x2F,
        Tokenizer::hold,
        Some(State::DcsInter),
    ),
    rule(
        DCS_ENTRY,
        0x30,
        0x3F,
        Tokenizer::hold,
        Some(State::DcsParam),
    ),
    rule(
        DCS_PARAM | DCS_INTER,
        0x3C,
        0x3F,
        Tokenizer::hold,
        Some(State::DcsIgnore),
    ),
    rule(
        DCS_INTER,
        0x30,
        0x3B,
        Tokenizer::hold,
        Some(State::DcsIgnore),
    ),
    rule(STRING, 0x00, 0xFF, Tokenizer::hold, None),
    rule(
        STR_ESC,
        b'\\',
        b'\\',
        Tokenizer::terminate_string,
        Some(State::Ground),
    ),
    rule(STR_ESC, 0x00, 0xFF, Tokenizer::interrupt_string, None),
];

/// Introducer byte after `ESC` that names a token's kind; anything else is an
/// escape sequence.
const KINDS: &[(u8, Kind)] = &[
    (b'[', Kind::Csi),
    (b']', Kind::Osc),
    (b'P', Kind::Dcs),
    (b'X', Kind::Sos),
    (b'^', Kind::Sos),
    (b'_', Kind::Sos),
];

/// Splits PTY output into complete tokens, holding a partial token (at most
/// [`MAX_BODY`] plus its introducer, or an unfinished UTF-8 sequence) until
/// a later read completes it.
#[derive(Debug, Default)]
pub struct Tokenizer {
    state: State,
    held: Vec<u8>,
    oversized: bool,
}

impl Tokenizer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one read and return every token it completed, in order.
    pub fn push(&mut self, input: &[u8]) -> Vec<Token> {
        let mut out = Vec::new();
        input.iter().for_each(|&byte| self.byte(byte, &mut out));
        self.emit_text(incomplete_utf8(&self.held), &mut out);
        out
    }

    fn byte(&mut self, byte: u8, out: &mut Vec<Token>) {
        let bit = self.state.bit();
        let rule = RULES
            .iter()
            .find(|rule| rule.states & bit != 0 && rule.lo <= byte && byte <= rule.hi)
            .expect("every state has a catch-all rule");
        (rule.action)(self, byte, out);
        self.state = rule.next.unwrap_or(self.state);
    }

    fn emit_text(&mut self, keep: usize, out: &mut Vec<Token>) {
        let ready = self.held.len().saturating_sub(keep);
        if self.state == State::Ground && ready > 0 {
            out.push(Token {
                kind: Kind::Text,
                bytes: self.held.drain(..ready).collect(),
            });
        }
    }

    fn text(&mut self, byte: u8, _: &mut Vec<Token>) {
        self.held.push(byte);
    }

    fn begin_escape(&mut self, byte: u8, out: &mut Vec<Token>) {
        self.emit_text(0, out);
        self.held.push(byte);
    }

    fn execute(&mut self, byte: u8, out: &mut Vec<Token>) {
        out.push(Token {
            kind: Kind::Text,
            bytes: vec![byte],
        });
    }

    fn abort(&mut self, byte: u8, out: &mut Vec<Token>) {
        self.reset();
        self.execute(byte, out);
    }

    fn restart(&mut self, byte: u8, _: &mut Vec<Token>) {
        self.reset();
        self.held.push(byte);
    }

    fn ignore(&mut self, _: u8, _: &mut Vec<Token>) {}

    fn hold(&mut self, byte: u8, _: &mut Vec<Token>) {
        self.oversized |= self.held.len() >= LIMIT;
        if !self.oversized {
            self.held.push(byte);
        }
    }

    fn finish(&mut self, byte: u8, out: &mut Vec<Token>) {
        self.complete(&[byte], out);
    }

    fn terminate_string(&mut self, _: u8, out: &mut Vec<Token>) {
        self.complete(ST, out);
    }

    fn cancel_string(&mut self, byte: u8, out: &mut Vec<Token>) {
        self.complete(ST, out);
        self.execute(byte, out);
    }

    /// An ESC inside a control string ends it; the ESC begins a new escape
    /// sequence that consumes this byte.
    fn interrupt_string(&mut self, byte: u8, out: &mut Vec<Token>) {
        self.complete(ST, out);
        self.held.push(ESC);
        self.state = State::Escape;
        self.byte(byte, out);
    }

    fn complete(&mut self, tail: &[u8], out: &mut Vec<Token>) {
        let token = if self.oversized {
            Token {
                kind: Kind::Oversized,
                bytes: Vec::new(),
            }
        } else {
            self.held.extend_from_slice(tail);
            Token {
                kind: kind_of(&self.held),
                bytes: std::mem::take(&mut self.held),
            }
        };
        out.push(token);
        self.reset();
    }

    fn reset(&mut self) {
        self.held.clear();
        self.oversized = false;
    }
}

fn kind_of(bytes: &[u8]) -> Kind {
    KINDS
        .iter()
        .find(|(introducer, _)| bytes.get(1) == Some(introducer))
        .map_or(Kind::Escape, |(_, kind)| *kind)
}

/// Length of an unfinished but so far valid UTF-8 sequence ending the text.
fn incomplete_utf8(text: &[u8]) -> usize {
    (text.len().saturating_sub(3)..text.len())
        .find(|&start| is_incomplete(&text[start..]))
        .map_or(0, |start| text.len() - start)
}

fn is_incomplete(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes)
        .err()
        .is_some_and(|error| error.valid_up_to() == 0 && error.error_len().is_none())
}
