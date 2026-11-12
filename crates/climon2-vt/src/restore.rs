//! Three-phase restoration (plan section 1.8): phase A is committed at the
//! grant; phase B (history) and phase C (the screen suffix captured at the
//! snapshot point) are produced incrementally.

use crate::{
    history::Ledger,
    presentation::{Mode, Presentation},
    render::{cells, content, goto, render, wrapped},
};
use alacritty_terminal::{Grid, term::cell::Cell};

pub(crate) const NOTICE_TEXT: &[u8] = b"[climon2: older session scrollback was discarded]";
pub(crate) const NOTICE: &[u8] = b"\x1b[0m[climon2: older session scrollback was discarded]\r\n";

/// A1: leave nothing an earlier stream may have set that history relies on.
pub(crate) const PROLOGUE: &[u8] = b"\x1b[?2026l\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\x1b[0m";

/// The next part of a restoration prefix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chunk {
    pub bytes: Vec<u8>,
    /// The restoration is complete; `RestoreEnd` may follow these bytes.
    pub done: bool,
}

/// A restoration in progress: the moving view of the snapshot's history and
/// the screen suffix captured at the snapshot point.
#[derive(Debug)]
pub struct Restoration {
    view: View,
    direct: bool,
    host_rows: u16,
    suffix: Option<Snapshot>,
}

#[derive(Debug)]
pub(crate) struct Snapshot {
    pub bytes: Vec<u8>,
    pub alternate: bool,
    pub normal_cursor: (usize, usize),
}

/// Phase B's position among the `h0` retained rows at the snapshot point,
/// located from push counts so a row is never read from the wrong line.
#[derive(Debug)]
struct View {
    epoch: u64,
    pushed: u64,
    rows: usize,
    next: usize,
    /// The last line emitted was the truncation notice.
    noticed: bool,
    /// Some line was emitted after phase A's preservation scroll.
    lines: bool,
    /// The last line emitted was a soft-wrapped row without CR LF.
    soft: bool,
}

enum Next {
    End,
    Row(u64),
    Gap(usize),
}

impl View {
    fn next(&self, ledger: &Ledger) -> Next {
        if !self.current(ledger) {
            return Next::End;
        }
        let pushed = ledger.pushed - self.pushed;
        let age = (self.rows - 1 - self.next) as u64 + pushed;
        if age < ledger.max_rows as u64 {
            Next::Row(age)
        } else {
            Next::Gap(self.rows + pushed as usize - ledger.max_rows)
        }
    }

    fn current(&self, ledger: &Ledger) -> bool {
        ledger.epoch == self.epoch && self.next < self.rows
    }

    /// Emit the next line; false once phase B has ended.
    fn step(
        &mut self,
        grid: &Grid<Cell>,
        ledger: &Ledger,
        direct: bool,
        out: &mut Vec<u8>,
    ) -> bool {
        match self.next(ledger) {
            Next::End => return false,
            Next::Row(age) => self.row(cells(grid, -1 - age as i32), direct, out),
            Next::Gap(next) => self.gap(next, out),
        }
        true
    }

    fn row(&mut self, cells: &[Cell], direct: bool, out: &mut Vec<u8>) {
        self.soft = history_row(cells, direct, out);
        self.next += 1;
        self.noticed = false;
        self.lines = true;
    }

    fn gap(&mut self, next: usize, out: &mut Vec<u8>) {
        if !self.noticed {
            out.extend_from_slice(NOTICE);
        }
        self.next = next;
        self.noticed = true;
        self.lines = true;
        self.soft = false;
    }
}

/// A history row: in Direct presentation a soft-wrapped row is printed full
/// width so the host wraps it; every other row is trimmed and ends in CR LF.
/// True for a soft-wrapped row.
pub(crate) fn history_row(cells: &[Cell], direct: bool, out: &mut Vec<u8>) -> bool {
    let soft = direct && wrapped(cells);
    render(if soft { cells } else { content(cells) }, out);
    if !soft {
        out.extend_from_slice(b"\r\n");
    }
    soft
}

/// Move to the host's bottom row and scroll `lines` rows into native
/// scrollback.
pub(crate) fn scroll_off(rows: u16, lines: u16, out: &mut Vec<u8>) {
    goto(out, usize::from(rows), 1);
    out.extend(std::iter::repeat_n(b'\n', usize::from(lines)));
}

/// Phase A: prologue, preservation of visible host content, and the
/// truncation notice. Committed with the grant.
pub(crate) fn lead(presentation: &mut Presentation, notice: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if presentation.alternate {
        out.extend_from_slice(b"\x1b[?1049l");
    }
    out.extend_from_slice(PROLOGUE);
    scroll_off(presentation.rows, presentation.rows, &mut out);
    if notice {
        out.extend_from_slice(NOTICE);
    }
    presentation.alternate = false;
    presentation.sync_open = false;
    presentation.release_cursor = None;
    out
}

impl Restoration {
    pub(crate) fn new(
        ledger: &Ledger,
        rows: usize,
        notice: bool,
        presentation: &Presentation,
        suffix: Snapshot,
    ) -> Self {
        Self {
            view: View {
                epoch: ledger.epoch,
                pushed: ledger.pushed,
                rows,
                next: 0,
                noticed: notice,
                lines: notice,
                soft: false,
            },
            direct: presentation.mode == Mode::Direct,
            host_rows: presentation.rows,
            suffix: Some(suffix),
        }
    }

    /// Phase B rows (whole rows only) until about `budget` bytes; true when
    /// phase B has ended.
    pub(crate) fn history(
        &mut self,
        grid: &Grid<Cell>,
        ledger: &Ledger,
        budget: usize,
        out: &mut Vec<u8>,
    ) -> bool {
        let mut more = true;
        while more && out.len() < budget {
            more = self.view.step(grid, ledger, self.direct, out);
        }
        !more
    }

    /// After phase B: C1 when lines were emitted, then the captured suffix.
    /// True once the suffix has been produced.
    pub(crate) fn finish(
        &mut self,
        ended: bool,
        presentation: &mut Presentation,
        out: &mut Vec<u8>,
    ) -> bool {
        if let Some(suffix) = self.suffix.take_if(|_| ended) {
            if self.view.lines {
                self.after_history(out);
            }
            out.extend(suffix.bytes);
            presentation.alternate = suffix.alternate;
            presentation.release_cursor = Some(suffix.normal_cursor);
        }
        self.suffix.is_none()
    }

    /// C1: the cursor is on the host's bottom row, blank unless the last row
    /// was soft-wrapped; scroll every row above it into native scrollback.
    fn after_history(&self, out: &mut Vec<u8>) {
        if self.view.soft {
            out.extend_from_slice(b"\r\n");
        }
        scroll_off(self.host_rows, self.host_rows - 1, out);
    }
}
