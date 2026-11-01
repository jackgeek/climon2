//! The live output policy (plan section 1.9) for one complete token: what
//! reaches the host, and which token effects the adapter must track.

use crate::token::{Kind, Token};
use alacritty_terminal::vte::{Params, Parser, Perform};

/// What a token contributes to live output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Live {
    Forward,
    Strip,
    Replace(Vec<u8>),
}

/// History a token clears when the engine executes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Clear {
    Nothing,
    /// `ESC c`: both screens, the title and every mode.
    Reset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Policy {
    pub(crate) live: Live,
    /// `Some(true)` for a synchronized-update begin, `Some(false)` for an end.
    pub(crate) sync: Option<bool>,
    pub(crate) clear: Clear,
}

impl Policy {
    fn of(live: Live) -> Self {
        Self {
            live,
            sync: None,
            clear: Clear::Nothing,
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
        _ => Policy::of(Live::Strip),
    }
}

#[derive(Debug, Default)]
struct Dispatch {
    params: Vec<u16>,
    intermediates: Vec<u8>,
    action: u8,
    ignored: bool,
    dispatched: bool,
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

fn escape(dispatch: &Dispatch) -> Policy {
    let live = if dispatch.is(b'Z', b"") {
        Live::Strip
    } else {
        Live::Forward
    };
    let clear = if dispatch.is(b'c', b"") {
        Clear::Reset
    } else {
        Clear::Nothing
    };
    Policy {
        live,
        sync: None,
        clear,
    }
}

fn csi(dispatch: &Dispatch) -> Policy {
    Policy {
        live: csi_live(dispatch),
        sync: sync(dispatch),
        clear: Clear::Nothing,
    }
}

fn csi_live(dispatch: &Dispatch) -> Live {
    if !dispatch.dispatched || dispatch.ignored || is_request(dispatch) {
        Live::Strip
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
        without_screens(&kept, dispatch.action)
    }
}

fn without_screens(kept: &[u16], action: u8) -> Live {
    if kept.is_empty() {
        Live::Strip
    } else {
        Live::Replace(private_modes(kept, action))
    }
}

fn private_modes(modes: &[u16], action: u8) -> Vec<u8> {
    let modes: Vec<String> = modes.iter().map(u16::to_string).collect();
    let mut bytes = format!("\x1b[?{}", modes.join(";")).into_bytes();
    bytes.push(action);
    bytes
}

fn sync(dispatch: &Dispatch) -> Option<bool> {
    private_mode(dispatch).filter(|_| dispatch.params.contains(&SYNC_MODE))
}
