//! Derived presentation (plan section 3.2): for a host larger than the
//! session, live output is rendered from engine state inside the session
//! rectangle instead of forwarding tokens that would wrap and scroll at the
//! host's size.

use crate::{
    engine::Engine,
    history::Ledger,
    modes::{SHARED, cursor_style, transitions},
    presentation::Presentation,
    render::{cells, content, goto, render},
    restore::NOTICE_TEXT,
    title,
};
use alacritty_terminal::{
    Grid,
    grid::Dimensions,
    term::{TermMode, cell::Cell},
};

/// Every derived line starts from the default rendition on an erased line,
/// so host cells outside the session rectangle stay blank.
const ERASE_LINE: &[u8] = b"\x1b[0m\x1b[2K";

/// What a Derived presentation last put on its host. An empty `rows` means
/// the host's screen is unknown and must be repainted completely.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cache {
    rows: Vec<Vec<u8>>,
    pushed: u64,
    epoch: u64,
    modes: Option<TermMode>,
    style: Vec<u8>,
    title: Option<String>,
    bells: u64,
    cursor: Option<(usize, usize)>,
}

impl Cache {
    /// The host's state after a restoration (or Direct output) that showed
    /// the engine's current state.
    pub(crate) fn current(engine: &Engine, ledger: &Ledger, title: &str) -> Self {
        let grid = engine.active();
        Self {
            rows: (0..grid.screen_lines())
                .map(|line| row(grid, line))
                .collect(),
            pushed: ledger.pushed,
            epoch: ledger.epoch,
            modes: Some(engine.mode()),
            style: cursor_style(engine.cursor_style()),
            title: Some(title.to_owned()),
            bells: engine.bells(),
            cursor: Some(position(grid)),
        }
    }

    pub(crate) fn repaint(&mut self) {
        self.rows.clear();
    }

    /// Repaint rows that changed; when the host screen is unknown, every row,
    /// and blank the host below the session rectangle.
    fn paint(&mut self, grid: &Grid<Cell>, host_rows: u16, out: &mut Vec<u8>) {
        let rows = grid.screen_lines();
        let unknown = self.rows.is_empty();
        self.rows.resize(rows, Vec::new());
        for line in 0..rows {
            let bytes = row(grid, line);
            if bytes != self.rows[line] {
                goto(out, line + 1, 1);
                out.extend_from_slice(&bytes);
                self.cursor = None;
            }
            self.rows[line] = bytes;
        }
        if unknown && usize::from(host_rows) > rows {
            goto(out, rows + 1, 1);
            out.extend_from_slice(b"\x1b[0m\x1b[J");
        }
    }

    fn place(&mut self, grid: &Grid<Cell>, out: &mut Vec<u8>) {
        let cursor = position(grid);
        if self.cursor != Some(cursor) {
            goto(out, cursor.0, cursor.1);
            self.cursor = Some(cursor);
        }
    }

    /// Input and display modes and the cursor style that changed. Derived
    /// presentation never changes origin, autowrap, insert or new-line mode.
    fn modes(&mut self, engine: &Engine, out: &mut Vec<u8>) {
        transitions(self.modes, engine.mode(), SHARED, out);
        self.modes = Some(engine.mode());
        let style = cursor_style(engine.cursor_style());
        if style != self.style {
            out.extend_from_slice(&style);
            self.style = style;
        }
    }

    fn retitle(&mut self, title: &str, out: &mut Vec<u8>) {
        if self.title.as_deref() != Some(title) {
            out.extend(title::osc(title));
            self.title = Some(title.to_owned());
        }
    }

    fn ring(&mut self, bells: u64, out: &mut Vec<u8>) {
        if bells > self.bells {
            out.push(0x07);
            self.bells = bells;
        }
    }
}

fn row(grid: &Grid<Cell>, line: usize) -> Vec<u8> {
    line_bytes(cells(grid, line as i32))
}

fn line_bytes(cells: &[Cell]) -> Vec<u8> {
    let mut out = ERASE_LINE.to_vec();
    render(content(cells), &mut out);
    out
}

fn position(grid: &Grid<Cell>) -> (usize, usize) {
    let point = grid.cursor.point;
    (point.line.0 as usize + 1, point.column.0 + 1)
}

/// One derived update; empty when nothing changed. Never call while a
/// synchronized update is pending.
pub(crate) fn derive(
    engine: &mut Engine,
    ledger: &Ledger,
    title: &str,
    presentation: &mut Presentation,
) -> Vec<u8> {
    let mut out = Vec::new();
    retire(ledger, &mut presentation.cache);
    let pushed = ledger.pushed - presentation.cache.pushed;
    leave(engine.alternate(), pushed, presentation, &mut out);
    if pushed > 0 {
        push(engine, ledger, pushed, presentation, &mut out);
        presentation.cache.pushed = ledger.pushed;
        presentation.cache.repaint();
    }
    enter(engine.alternate(), presentation, &mut out);
    let cache = &mut presentation.cache;
    cache.paint(engine.active(), presentation.rows, &mut out);
    cache.place(engine.active(), &mut out);
    cache.modes(engine, &mut out);
    cache.retitle(title, &mut out);
    cache.ring(engine.bells(), &mut out);
    out
}

/// A cleared or resized history cannot be continued: repaint everything and
/// skip rows the host never saw pushed.
fn retire(ledger: &Ledger, cache: &mut Cache) {
    if ledger.epoch != cache.epoch {
        cache.repaint();
        cache.pushed = ledger.pushed;
        cache.epoch = ledger.epoch;
    }
}

/// Leave this host's alternate screen when the engine left it, or when
/// pushed rows must reach the host's normal screen and native scrollback.
fn leave(alternate: bool, pushed: u64, presentation: &mut Presentation, out: &mut Vec<u8>) {
    if presentation.alternate && (!alternate || pushed > 0) {
        out.extend_from_slice(b"\x1b[?1049l");
        presentation.alternate = false;
        presentation.cache.repaint();
    }
}

fn enter(alternate: bool, presentation: &mut Presentation, out: &mut Vec<u8>) {
    if alternate && !presentation.alternate {
        out.extend_from_slice(b"\x1b[?1049h");
        presentation.alternate = true;
        presentation.cache.repaint();
    }
}

/// Send rows pushed into retained history, oldest first, into the host's
/// native scrollback: paint up to a host screenful at the top, then scroll it
/// off from the bottom row. A run longer than retention starts with the
/// notice, cut to the host's width so it never wraps.
fn push(engine: &mut Engine, ledger: &Ledger, pushed: u64, host: &Presentation, out: &mut Vec<u8>) {
    let host_rows = host.rows;
    let retained = pushed.min(ledger.max_rows as u64) as usize;
    let mut lines: Vec<Vec<u8>> = Vec::new();
    if pushed > retained as u64 && retained > 0 {
        let width = NOTICE_TEXT.len().min(usize::from(host.cols));
        lines.push([ERASE_LINE, &NOTICE_TEXT[..width]].concat());
    }
    engine.normal(|grid| {
        lines.extend(
            (0..retained)
                .rev()
                .map(|age| line_bytes(cells(grid, -1 - age as i32))),
        )
    });
    for chunk in lines.chunks(usize::from(host_rows)) {
        for (index, line) in chunk.iter().enumerate() {
            goto(out, index + 1, 1);
            out.extend_from_slice(line);
        }
        goto(out, usize::from(host_rows), 1);
        out.extend(std::iter::repeat_n(b'\n', chunk.len()));
    }
}
