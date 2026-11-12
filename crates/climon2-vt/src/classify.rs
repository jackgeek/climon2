//! The live output policy (plan section 1.9) for one complete token: what
//! reaches the host, and which token effects the adapter must track.

use crate::token::{Kind, Token};
use alacritty_terminal::vte::{Params, Parser, Perform};

/// What a token contributes to live output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Live {
    Forward,
    /// These bytes instead of the token: the C0 controls the parser executed
    /// in it, which the host must perform too, then any replacement.
    Replace(Vec<u8>),
}

/// History a token clears when the engine executes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clear {
    Nothing,
    /// `CSI 3 J`: the active screen's history only.
    History,
    /// `ESC c`: both screens, the title and every mode.
    Reset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    pub(crate) live: Live,
    /// `Some(true)` for a synchronized-update begin, `Some(false)` for an end.
    pub(crate) sync: Option<bool>,
    pub(crate) clear: Clear,
    /// How many more times the engine prints its preceding character
    /// (`CSI n b`).
    pub(crate) repeats: usize,
}

impl Policy {
    fn of(live: Live) -> Self {
        Self {
            live,
            sync: None,
            clear: Clear::Nothing,
            repeats: 0,
        }
    }
}

/// CSI requests the host must never see, by final byte and (when given) an
/// intermediate or private marker the sequence must contain.
const STRIPPED_CSI: &[(u8, Option<u8>)] = &[
    (b'c', None),
    (b'n', None),
    (b'x', None),
    (b't', None),
    (b'q', Some(b'>')),
    (b'S', Some(b'?')),
    (b'p', Some(b'$')),
    (b'u', Some(b'?')),
    (b'u', Some(b'>')),
    (b'u', Some(b'<')),
    (b'u', Some(b'=')),
    (b'm', Some(b'>')),
    (b'm', Some(b'?')),
];

/// Screen modes the engine does not implement; forwarding them would put the
/// host on a screen the engine never shows.
const UNSUPPORTED_SCREENS: &[u16] = &[47, 1047];
const SYNC_MODE: u16 = 2026;

pub(crate) fn classify(token: &Token) -> Policy {
    match token.kind {
        Kind::Text => Policy::of(Live::Forward),
        Kind::Escape => escape(&parse(&token.bytes)),
        Kind::Csi => csi(&parse(&token.bytes)),
        _ => Policy::of(strip(&parse(&token.bytes))),
    }
}

/// What vte did with one token.
#[derive(Debug, Default)]
struct Dispatch {
    params: Vec<u16>,
    intermediates: Vec<u8>,
    action: u8,
    ignored: bool,
    dispatched: bool,
    /// The C0 controls executed, in order.
    executed: Vec<u8>,
}

impl Dispatch {
    fn is(&self, action: u8, intermediates: &[u8]) -> bool {
        self.dispatched
            && !self.ignored
            && self.action == action
            && self.intermediates == intermediates
    }
}

impl Perform for Dispatch {
    fn execute(&mut self, byte: u8) {
        self.executed.push(byte);
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignored: bool, action: char) {
        self.params = params.iter().map(|param| param[0]).collect();
        self.record(intermediates, ignored, action as u8);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignored: bool, byte: u8) {
        self.record(intermediates, ignored, byte);
    }
}

impl Dispatch {
    fn record(&mut self, intermediates: &[u8], ignored: bool, action: u8) {
        self.intermediates = intermediates.to_vec();
        self.ignored = ignored;
        self.action = action;
        self.dispatched = true;
    }
}

fn parse(bytes: &[u8]) -> Dispatch {
    let mut dispatch = Dispatch::default();
    Parser::new().advance(&mut dispatch, bytes);
    dispatch
}

/// Only the controls the parser executed reach the host.
fn strip(dispatch: &Dispatch) -> Live {
    Live::Replace(dispatch.executed.clone())
}

fn escape(dispatch: &Dispatch) -> Policy {
    let live = if dispatch.is(b'Z', b"") {
        strip(dispatch)
    } else {
        Live::Forward
    };
    let clear = if dispatch.is(b'c', b"") {
        Clear::Reset
    } else {
        Clear::Nothing
    };
    Policy {
        clear,
        ..Policy::of(live)
    }
}

fn csi(dispatch: &Dispatch) -> Policy {
    Policy {
        live: csi_live(dispatch),
        sync: sync(dispatch),
        clear: csi_clear(dispatch),
        repeats: repeats(dispatch),
    }
}

/// vte prints its preceding character `n` times for `CSI n b`, once when `n`
/// is absent or zero.
fn repeats(dispatch: &Dispatch) -> usize {
    let count = dispatch.params.first().filter(|_| dispatch.is(b'b', b""));
    count.map_or(0, |&count| usize::from(count.max(1)))
}

fn csi_live(dispatch: &Dispatch) -> Live {
    if !dispatch.dispatched || dispatch.ignored || is_request(dispatch) {
        strip(dispatch)
    } else {
        screen_modes(dispatch)
    }
}

fn is_request(dispatch: &Dispatch) -> bool {
    STRIPPED_CSI.iter().any(|(action, marker)| {
        dispatch.action == *action
            && marker.is_none_or(|marker| dispatch.intermediates.contains(&marker))
    })
}

fn private_mode(dispatch: &Dispatch) -> Option<bool> {
    [(b'h', true), (b'l', false)]
        .into_iter()
        .find(|(action, _)| dispatch.is(*action, b"?"))
        .map(|(_, set)| set)
}

fn screen_modes(dispatch: &Dispatch) -> Live {
    let kept: Vec<u16> = dispatch
        .params
        .iter()
        .copied()
        .filter(|mode| !UNSUPPORTED_SCREENS.contains(mode))
        .collect();
    if private_mode(dispatch).is_none() || kept.len() == dispatch.params.len() {
        Live::Forward
    } else {
        without_screens(&kept, dispatch)
    }
}

fn without_screens(kept: &[u16], dispatch: &Dispatch) -> Live {
    if kept.is_empty() {
        strip(dispatch)
    } else {
        Live::Replace([&dispatch.executed[..], &decset(kept, dispatch.action)].concat())
    }
}

fn decset(modes: &[u16], action: u8) -> Vec<u8> {
    let modes: Vec<String> = modes.iter().map(u16::to_string).collect();
    let mut bytes = format!("\x1b[?{}", modes.join(";")).into_bytes();
    bytes.push(action);
    bytes
}

fn sync(dispatch: &Dispatch) -> Option<bool> {
    private_mode(dispatch).filter(|_| dispatch.params.contains(&SYNC_MODE))
}

/// `(set, modes)` for a DECSET (`CSI ? … h`) or DECRST (`CSI ? … l`) token.
pub(crate) fn private_modes(token: &Token) -> Option<(bool, Vec<u16>)> {
    let dispatch = (token.kind == Kind::Csi).then(|| parse(&token.bytes))?;
    private_mode(&dispatch).map(|set| (set, dispatch.params))
}

fn csi_clear(dispatch: &Dispatch) -> Clear {
    if dispatch.is(b'J', b"") && dispatch.params.first() == Some(&3) {
        Clear::History
    } else {
        Clear::Nothing
    }
}
