//! Host-bound signals the emulated host acts on: OSC 7501 program-status
//! reports and the bell, found by the host's own parser (vte) in the order it
//! acts on them.

use alacritty_terminal::vte::{Parser, Perform};
use std::{fmt, iter, mem};

const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const ESC: u8 = 0x1b;
const PROGRAM_STATUS: &[u8] = b"7501";

/// One host-bound signal, recorded in the order the host's parser acted on it.
#[derive(Clone, PartialEq, Eq)]
pub enum HostSignal {
    /// An OSC 7501 program-status report, recorded where the parser ended its
    /// OSC string. Its exact bytes run from the ESC that began it through the
    /// terminator the host parsed: BEL, CAN, SUB, or an ESC whose escape
    /// sequence ends in `\` (ST). Bytes the host executed or ignored inside
    /// those escape sequences are kept; when another escape sequence
    /// interrupted the report, it has no terminator.
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
/// in the stream however the host's reads split it. The parser rings the bell,
/// ends an OSC string or changes a report's bytes only at BEL, CAN, SUB or ESC,
/// or at a byte in its Escape state, so those bytes go one at a time and the
/// runs between them go in one call.
#[derive(Default)]
pub(crate) struct Signals {
    parser: Parser,
    place: Place,
    recorded: Vec<HostSignal>,
}

impl Signals {
    pub(crate) fn push(&mut self, bytes: &[u8]) {
        let mut rest = bytes;
        while !rest.is_empty() {
            let (run, later) = rest.split_at(self.place.run(rest));
            let mut acts = Acts::default();
            self.parser.advance(&mut acts, run);
            self.place = mem::take(&mut self.place).after(run, acts, &mut self.recorded);
            rest = later;
        }
    }

    pub(crate) fn recorded(&self) -> &[HostSignal] {
        &self.recorded
    }
}

/// What the parser did with one run.
#[derive(Default)]
struct Acts {
    /// It executed BEL.
    bell: bool,
    /// Whether it ended an OSC string, and which.
    osc: OscEnd,
}

/// Whether the parser ended an OSC string in a run, and which.
#[derive(Default)]
enum OscEnd {
    #[default]
    NotEnded,
    /// An OSC string that is not an OSC 7501 report.
    Other,
    /// An OSC 7501 program-status report.
    ProgramStatus,
}

impl Perform for Acts {
    fn execute(&mut self, byte: u8) {
        self.bell |= byte == BEL;
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        self.osc = if params.first() == Some(&PROGRAM_STATUS) {
            OscEnd::ProgramStatus
        } else {
            OscEnd::Other
        };
    }
}

/// Where the parser is, as far as a report's bytes depend on it. vte 0.15
/// enters its Escape state at every ESC, from any state, and stays there
/// through C0 controls other than CAN and SUB, ESC, DEL and bytes 0x80-0xFF.
/// Only `]` in that state begins an OSC string, and only BEL, CAN, SUB or ESC
/// ends one.
#[derive(Default)]
enum Place {
    /// Neither in the Escape state nor in an OSC string.
    #[default]
    Other,
    /// In the Escape state, with its bytes from the ESC that entered it.
    Escape {
        bytes: Vec<u8>,
        ended: Option<Ended>,
    },
    /// In an OSC string, with its bytes from the ESC that began it.
    Osc(Vec<u8>),
}

/// A report the parser ended at the ESC that entered its Escape state: that
/// escape sequence decides whether the ESC begins its ST.
struct Ended {
    report: Vec<u8>,
    /// Bells rung since, which the parser acted on after the report.
    bells: usize,
}

impl Place {
    /// The length of the next run to parse from `bytes`, which is not empty.
    fn run(&self, bytes: &[u8]) -> usize {
        if matches!(self, Self::Escape { .. }) {
            return 1;
        }
        bytes
            .iter()
            .position(|&byte| matches!(byte, BEL | CAN | SUB | ESC))
            .map_or(bytes.len(), |special| special.max(1))
    }

    /// Where the parser is after `run`, which did `acts`; records the signals
    /// it completed.
    fn after(self, run: &[u8], acts: Acts, recorded: &mut Vec<HostSignal>) -> Self {
        match self {
            Self::Other => {
                if acts.bell {
                    recorded.push(HostSignal::Bell);
                }
                Self::at(run[0])
            }
            Self::Osc(bytes) => Self::osc(bytes, run, acts.osc, recorded),
            Self::Escape { bytes, ended } => {
                Self::escape(bytes, ended, run[0], acts.bell, recorded)
            }
        }
    }

    /// In an OSC string with `bytes` so far, after `run`, in which the parser
    /// ended it or not (`end`).
    fn osc(mut bytes: Vec<u8>, run: &[u8], end: OscEnd, recorded: &mut Vec<HostSignal>) -> Self {
        bytes.extend_from_slice(run);
        match end {
            OscEnd::NotEnded => Self::Osc(bytes),
            OscEnd::Other => Self::at(run[0]),
            OscEnd::ProgramStatus if run[0] == ESC => {
                bytes.pop();
                Self::Escape {
                    bytes: vec![ESC],
                    ended: Some(Ended {
                        report: bytes,
                        bells: 0,
                    }),
                }
            }
            OscEnd::ProgramStatus => {
                recorded.push(HostSignal::Osc7501(bytes));
                Self::Other
            }
        }
    }

    /// In the Escape state with `bytes` so far, after `byte`, which rang the
    /// bell if `bell`.
    fn escape(
        mut bytes: Vec<u8>,
        mut ended: Option<Ended>,
        byte: u8,
        bell: bool,
        recorded: &mut Vec<HostSignal>,
    ) -> Self {
        bytes.push(byte);
        if bell {
            match &mut ended {
                Some(ended) => ended.bells += 1,
                None => recorded.push(HostSignal::Bell),
            }
        }
        if !matches!(byte, CAN | SUB | 0x20..=0x7e) {
            return Self::Escape { bytes, ended };
        }
        if let Some(Ended { mut report, bells }) = ended {
            if byte == b'\\' {
                report.extend_from_slice(&bytes);
            }
            recorded.push(HostSignal::Osc7501(report));
            recorded.extend(iter::repeat_n(HostSignal::Bell, bells));
        }
        if byte == b']' {
            Self::Osc(bytes)
        } else {
            Self::Other
        }
    }

    /// Where `byte` leaves the parser when it is outside the Escape state and
    /// ends no report: in the Escape state after ESC, elsewhere otherwise.
    fn at(byte: u8) -> Self {
        if byte == ESC {
            Self::Escape {
                bytes: vec![ESC],
                ended: None,
            }
        } else {
            Self::Other
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HostSignal, Signals};

    fn report(bytes: &[u8]) -> HostSignal {
        HostSignal::Osc7501(bytes.to_vec())
    }

    /// Streams and the signals vte 0.15's transition table gives for them,
    /// written out by hand: every ESC enters its Escape state, which executes C0
    /// controls other than CAN and SUB, ignores ESC, DEL and 0x80-0xFF, and is
    /// left by any other byte; only `]` there begins an OSC string, which only
    /// BEL, CAN, SUB or ESC ends.
    fn cases() -> Vec<(&'static [u8], Vec<HostSignal>)> {
        vec![
            // A bell rung inside the escape sequence that begins a report.
            (
                b"\x1b\x07]7501;state=working\x07",
                vec![HostSignal::Bell, report(b"\x1b\x07]7501;state=working\x07")],
            ),
            // Bytes the Escape state executes or ignores, before `]` and inside ST.
            (
                b"\x1b\x00\x7f\x80\xff\x1b]7501;state=clear\x1b\x00\x7f\x9c\x1b\\",
                vec![report(
                    b"\x1b\x00\x7f\x80\xff\x1b]7501;state=clear\x1b\x00\x7f\x9c\x1b\\",
                )],
            ),
            // The parser ends a report at ESC, before ringing a bell inside its ST
            // or inside the escape sequence that interrupts it.
            (
                b"\x1b]7501;state=idle\x1b\x07\\",
                vec![report(b"\x1b]7501;state=idle\x1b\x07\\"), HostSignal::Bell],
            ),
            (
                b"\x1b]7501;state=idle\x1b\x07[0m",
                vec![report(b"\x1b]7501;state=idle"), HostSignal::Bell],
            ),
            // CAN cancels the escape sequence, so `]` is printed and BEL rings.
            (
                b"\x1b]7501;state=idle\x1b\x18]7501;state=clear\x07",
                vec![report(b"\x1b]7501;state=idle"), HostSignal::Bell],
            ),
            // An intermediate byte turns `]` into the final byte of another escape.
            (b"\x1b ]7501;state=clear\x07", vec![HostSignal::Bell]),
            // A C1 OSC byte is executed, not an OSC string.
            (b"\x9d7501;state=clear\x07", vec![HostSignal::Bell]),
            // CAN and SUB end a report; a C0 control inside it keeps its bytes.
            (
                b"\x1b]7501;state=working\x18\x1b]75\x0001;state=clear\x1a",
                vec![
                    report(b"\x1b]7501;state=working\x18"),
                    report(b"\x1b]75\x0001;state=clear\x1a"),
                ],
            ),
            // A report interrupted by another keeps no terminator.
            (
                b"\x1b]7501;state=working\x1b]7501;state=clear\x1b\\",
                vec![
                    report(b"\x1b]7501;state=working"),
                    report(b"\x1b]7501;state=clear\x1b\\"),
                ],
            ),
            // BEL ends other OSC strings and sits inside DCS, SOS, PM and APC
            // strings without ringing.
            (
                b"\x1b]75010;x\x07\x1b]750;x\x1b\\\x1b]0;t\x07\x1bP1$r\x07\x1b\\\
                  \x1bXs\x07\x1b\\\x1b^p\x07\x1b\\\x1b_a\x07\x1b\\",
                vec![],
            ),
        ]
    }

    fn signals(reads: &[&[u8]]) -> Vec<HostSignal> {
        let mut signals = Signals::default();
        for read in reads {
            signals.push(read);
        }
        signals.recorded().to_vec()
    }

    /// #79 (Terminal oracle): each stream gives the signals the host's parser
    /// acts on, in that order and with exact bytes, however reads split it:
    /// at any one point, or between every byte.
    #[test]
    fn signals_follow_the_host_parser_at_every_read_split() {
        for (stream, expected) in cases() {
            let name = stream.escape_ascii();
            let bytes: Vec<&[u8]> = stream.chunks(1).collect();
            assert_eq!(signals(&bytes), expected, "{name} byte by byte");
            for split in 0..=stream.len() {
                let (first, second) = stream.split_at(split);
                assert_eq!(
                    signals(&[first, second]),
                    expected,
                    "{name} split at {split}"
                );
            }
        }
    }
}
