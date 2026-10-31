//! THROWAWAY #48/#43: OSC admission ahead of the unchanged engine.
//!
//! vte 0.15.0 (std) accumulates OSC bodies in an unbounded Vec (lib.rs:64,
//! 551) and every way out of the OSC state dispatches it (lib.rs:407-433), so
//! an oversized string can only be refused before the engine sees it. This
//! tracks just enough of vte's state to find `ESC ]` (lib.rs:340-391; ESC
//! enters Escape from every state, lib.rs:597-606, 455-466), withholds the
//! body until it terminates, and forwards it only if it fits the cap.
//! DCS/SOS/PM/APC bodies are not retained by vte (lib.rs:317-338, 182) or
//! Alacritty (ansi.rs:1311-1326) and pass through untouched.

const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    /// Ground, CSI, DCS or SOS/PM/APC: only ESC matters here.
    Elsewhere,
    Escape,
    EscapeIntermediate,
    /// The engine is in Escape; `]` and the body are withheld.
    Osc,
    /// Oversized: the engine is in Escape; discard until the terminator.
    Refusing,
}

pub struct OscAdmission {
    cap: usize,
    state: State,
    held: Vec<u8>,
    breaches: u64,
}

impl OscAdmission {
    pub fn new(cap: usize) -> Self {
        Self {
            cap,
            state: State::Elsewhere,
            held: Vec::new(),
            breaches: 0,
        }
    }

    /// Appends the bytes the engine may parse now to `out`.
    pub fn admit(&mut self, input: &[u8], out: &mut Vec<u8>) {
        for &byte in input {
            self.state = match self.state {
                State::Osc => self.osc(byte, out),
                State::Refusing => Self::refusing(byte, out),
                state => {
                    let next = Self::outside(state, byte);
                    if next == State::Osc {
                        self.held.clear();
                    } else {
                        out.push(byte);
                    }
                    next
                }
            };
        }
    }

    fn outside(state: State, byte: u8) -> State {
        match (state, byte) {
            (_, ESC) => State::Escape,
            (State::Escape, b']') => State::Osc,
            (State::Escape, 0x20..=0x2f) => State::EscapeIntermediate,
            (State::Escape | State::EscapeIntermediate, 0x30..=0x7e | CAN | SUB) => {
                State::Elsewhere
            }
            (state, _) => state,
        }
    }

    fn osc(&mut self, byte: u8, out: &mut Vec<u8>) -> State {
        match byte {
            BEL | CAN | SUB | ESC => {
                out.push(b']');
                out.append(&mut self.held);
                out.push(byte);
                if byte == ESC {
                    State::Escape
                } else {
                    State::Elsewhere
                }
            }
            _ if self.held.len() >= self.cap => {
                self.breaches += 1;
                self.held = Vec::new();
                State::Refusing
            }
            _ => {
                if self.held.capacity() == 0 {
                    self.held.reserve_exact(self.cap);
                }
                self.held.push(byte);
                State::Osc
            }
        }
    }

    /// The engine sits in Escape; CAN/SUB return it to ground with no
    /// Alacritty effect beyond what the real terminator would have had.
    fn refusing(byte: u8, out: &mut Vec<u8>) -> State {
        match byte {
            BEL => out.push(CAN),
            CAN | SUB => out.push(byte),
            ESC => {
                out.extend([CAN, ESC]);
                return State::Escape;
            }
            _ => return State::Refusing,
        }
        State::Elsewhere
    }

    /// Bytes withheld from the engine because their OSC has not ended.
    pub fn pending(&self) -> Vec<u8> {
        match self.state {
            State::Osc => [b"]", self.held.as_slice()].concat(),
            _ => Vec::new(),
        }
    }

    pub fn retained_capacity(&self) -> usize {
        self.held.capacity()
    }

    /// Oversized strings refused so far, for the session log.
    pub fn breaches(&self) -> u64 {
        self.breaches
    }
}
