//! THROWAWAY: streaming client-action/paste seam, not a client or wire protocol.

pub const MAX_FEED: usize = 65536;
const PASTE_BEGIN: &[u8; 6] = b"\x1b[200~";
const PASTE_END: &[u8; 6] = b"\x1b[201~";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Viewing,
    Restoring,
    Ready,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Application(Vec<u8>),
    Detach,
    TakeControl,
    FinishPaste,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidBinding,
    ConflictingBindings,
    EscapeTakeControl,
    InputTooLarge,
    Detached,
}

#[derive(Clone, Copy)]
pub struct Bindings {
    prefix: Option<u8>,
    take_control: Option<u8>,
}

impl Bindings {
    pub fn new(prefix: Option<u8>, take_control: Option<u8>) -> Result<Self, Error> {
        if prefix.into_iter().chain(take_control).any(|byte| byte > 31) {
            return Err(Error::InvalidBinding);
        }
        if prefix.is_some() && prefix == take_control {
            return Err(Error::ConflictingBindings);
        }
        if take_control == Some(0x1b) {
            return Err(Error::EscapeTakeControl);
        }
        Ok(Self {
            prefix,
            take_control,
        })
    }
}

impl Default for Bindings {
    fn default() -> Self {
        Self {
            prefix: Some(0x1d),
            take_control: Some(0x14),
        }
    }
}

pub struct Codec {
    bindings: Bindings,
    state: State,
    pending_prefix: Option<u8>,
    detached: bool,
    pending: [u8; 6],
    pending_len: usize,
    stale_bytes: usize,
    paste: Option<bool>,
}

impl Codec {
    pub fn new(bindings: Bindings, state: State) -> Self {
        Self {
            bindings,
            state,
            pending_prefix: None,
            detached: false,
            pending: Default::default(),
            pending_len: 0,
            stale_bytes: 0,
            paste: None,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Effect>, Error> {
        if self.detached {
            return Err(Error::Detached);
        }
        if bytes.len() > MAX_FEED {
            return Err(Error::InputTooLarge);
        }
        let mut effects = Vec::new();
        for &byte in bytes {
            self.consume(byte, &mut effects);
            if self.detached {
                break;
            }
        }
        Ok(effects
            .into_iter()
            .flat_map(Self::bound_application)
            .collect())
    }

    pub fn transition(&mut self, state: State) -> Result<Vec<Effect>, Error> {
        if self.detached {
            return Err(Error::Detached);
        }
        self.state = state;
        self.pending_prefix = None;
        self.stale_bytes = self.pending_len;
        if self.paste == Some(true) {
            self.paste = Some(false);
            Ok(vec![Effect::FinishPaste])
        } else {
            Ok(Vec::new())
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.pending_len + usize::from(self.pending_prefix.is_some())
    }

    fn consume(&mut self, byte: u8, effects: &mut Vec<Effect>) {
        if self.doubled_escape(byte) {
            self.pending_len = 0;
            self.application(b"\x1b", effects);
            return;
        }
        self.pending[self.pending_len] = byte;
        self.pending_len += 1;
        self.drain_pending(effects);
    }

    fn doubled_escape(&self, byte: u8) -> bool {
        matches!(
            (self.bindings.prefix, self.paste, self.stale_bytes, byte),
            (Some(0x1b), None, 0, 0x1b)
        ) && self.pending[..self.pending_len] == *b"\x1b"
    }

    fn drain_pending(&mut self, effects: &mut Vec<Effect>) {
        while self.pending_len != 0 {
            let marker = self.expected_marker();
            if marker.starts_with(&self.pending[..self.pending_len]) {
                if self.pending_len == marker.len() {
                    let stale = self.stale_bytes != 0;
                    self.pending_len = 0;
                    self.stale_bytes = 0;
                    self.paste_marker(stale, effects);
                }
                return;
            }
            self.release_pending(effects);
            if self.detached {
                return;
            }
        }
    }

    fn expected_marker(&self) -> &'static [u8; 6] {
        if self.paste.is_some() {
            PASTE_END
        } else {
            PASTE_BEGIN
        }
    }

    fn release_pending(&mut self, effects: &mut Vec<Effect>) {
        let first = self.pending[0];
        self.pending.copy_within(1..self.pending_len, 0);
        self.pending_len -= 1;
        if self.stale_bytes == 0 {
            self.plain(first, effects);
        } else {
            self.stale_bytes -= 1;
        }
    }

    fn paste_marker(&mut self, stale: bool, effects: &mut Vec<Effect>) {
        match self.paste.take() {
            Some(true) => self.application(PASTE_END, effects),
            Some(false) => {}
            None => self.begin_paste(stale, effects),
        }
    }

    fn begin_paste(&mut self, stale: bool, effects: &mut Vec<Effect>) {
        if let Some(prefix) = self.pending_prefix.take() {
            self.application(&[prefix], effects);
        }
        self.paste = Some(self.state == State::Ready && !stale);
        if self.paste == Some(true) {
            self.application(PASTE_BEGIN, effects);
        }
    }

    fn plain(&mut self, byte: u8, effects: &mut Vec<Effect>) {
        match self.paste {
            Some(true) => self.application(&[byte], effects),
            Some(false) => {}
            None => self.ordinary(byte, effects),
        }
    }

    fn ordinary(&mut self, byte: u8, effects: &mut Vec<Effect>) {
        if let Some(prefix) = self.pending_prefix.take() {
            self.prefixed(prefix, byte, effects);
        } else if Some(byte) == self.bindings.prefix {
            self.pending_prefix = Some(byte);
        } else if self.state == State::Viewing && Some(byte) == self.bindings.take_control {
            effects.push(Effect::TakeControl);
        } else {
            self.application(&[byte], effects);
        }
    }

    fn prefixed(&mut self, prefix: u8, byte: u8, effects: &mut Vec<Effect>) {
        if byte == b'd' {
            self.detached = true;
            effects.push(Effect::Detach);
        } else if byte == prefix {
            self.application(&[prefix], effects);
        } else {
            self.application(&[prefix, byte], effects);
        }
    }

    fn application(&self, bytes: &[u8], effects: &mut Vec<Effect>) {
        if self.state != State::Ready {
            return;
        }
        if let Some(Effect::Application(pending)) = effects.last_mut() {
            pending.extend_from_slice(bytes);
        } else {
            effects.push(Effect::Application(bytes.to_vec()));
        }
    }

    fn bound_application(effect: Effect) -> Vec<Effect> {
        match effect {
            Effect::Application(bytes) => bytes
                .chunks(MAX_FEED)
                .map(|chunk| Effect::Application(chunk.to_vec()))
                .collect(),
            action => vec![action],
        }
    }
}
