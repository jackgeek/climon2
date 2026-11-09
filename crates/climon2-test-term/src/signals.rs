//! Host-bound signals the emulated host acts on: OSC 7501 program-status
//! reports and the bell, found by the host's own parser (vte) in stream order.

use alacritty_terminal::vte::{Parser, Perform};
use std::fmt;

const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;
const PROGRAM_STATUS: &[u8] = b"7501";

/// One host-bound signal, recorded in the order the host received it.
#[derive(Clone, PartialEq, Eq)]
pub enum HostSignal {
    /// An OSC 7501 program-status report: its exact bytes from `ESC ]` through
    /// the terminator the host parsed (`ESC \`, BEL, CAN or SUB), or without
    /// one when another escape sequence interrupted it.
    Osc7501(Vec<u8>),
    /// A BEL the host executed, ringing its bell. A BEL that ends an OSC
    /// string, or sits inside a DCS, SOS, PM or APC string, is not one.
    Bell,
}

impl fmt::Debug for HostSignal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Osc7501(bytes) => write!(formatter, "Osc7501(b\"{}\")", bytes.escape_ascii()),
            Self::Bell => formatter.write_str("Bell"),
        }
    }
}

/// Feeds the host's parser so each signal keeps its exact bytes and its place
/// in the stream however the host's reads split it: a byte that can end an
/// OSC string or ring the bell, and the byte after ESC, go one at a time, and
/// the runs between them, which can do neither, go in one call.
#[derive(Default)]
pub(crate) struct Signals {
    parser: Parser,
    scan: Scan,
}

impl Signals {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while let Some(length) = self.scan.run(rest) {
            let (run, later) = rest.split_at(length);
            self.scan.before(run);
            self.parser.advance(&mut self.scan, run);
            self.scan.previous = run[length - 1];
            rest = later;
        }
    }

    pub(crate) fn recorded(&self) -> &[HostSignal] {
        &self.scan.recorded
    }
}

#[derive(Default)]
struct Scan {
    previous: u8,
    /// Raw bytes of the OSC string the parser is in; vte enters one only at `ESC ]`.
    osc: Option<Vec<u8>>,
    /// A report the parser ended at ESC: the next byte shows whether `\`
    /// completes its ST or the ESC began another sequence.
    ended_at_escape: Option<Vec<u8>>,
    recorded: Vec<HostSignal>,
}

impl Scan {
    /// The length of the next run of `bytes` to parse, if any remain.
    fn run(&self, bytes: &[u8]) -> Option<usize> {
        let (&first, later) = bytes.split_first()?;
        if self.previous == ESC || ends_osc(first) {
            return Some(1);
        }
        Some(
            1 + later
                .iter()
                .position(|&byte| ends_osc(byte))
                .unwrap_or(later.len()),
        )
    }

    fn before(&mut self, run: &[u8]) {
        if let Some(mut report) = self.ended_at_escape.take() {
            if run[0] == b'\\' {
                report.push(run[0]);
            } else {
                report.pop();
            }
            self.recorded.push(HostSignal::Osc7501(report));
        }
        if self.previous == ESC && run[0] == b']' {
            self.osc = Some(vec![ESC]);
        }
        if let Some(osc) = &mut self.osc {
            osc.extend_from_slice(run);
        }
    }
}

/// Whether vte ends an OSC string at `byte`; BEL may instead ring the bell.
fn ends_osc(byte: u8) -> bool {
    matches!(byte, BEL | CAN | SUB | ESC)
}

impl Perform for Scan {
    fn execute(&mut self, byte: u8) {
        if byte == BEL {
            self.recorded.push(HostSignal::Bell);
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        let osc = self
            .osc
            .take()
            .expect("vte enters an OSC string only at ESC ]");
        if params.first() != Some(&PROGRAM_STATUS) {
            return;
        }
        if osc.last() == Some(&ESC) {
            self.ended_at_escape = Some(osc);
        } else {
            self.recorded.push(HostSignal::Osc7501(osc));
        }
    }
}
