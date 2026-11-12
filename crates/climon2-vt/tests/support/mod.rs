#![allow(dead_code)]

use alacritty_terminal::{
    Term,
    event::VoidListener,
    grid::Dimensions,
    index::{Column, Line, Point},
    term::{Config, cell::Cell, test::TermSize},
    vte::ansi::{Processor, StdSyncHandler},
};
use climon2_vt::{Geometry, Presentation, Vt, VtConfig};
use std::time::Instant;

pub const TITLE: &str = "climon2 exec app";

pub fn vt(cols: u16, rows: u16, max_history_rows: u32) -> Vt {
    Vt::new(
        Geometry::for_host(cols, rows).unwrap(),
        VtConfig {
            max_history_rows,
            default_title: TITLE.into(),
        },
    )
}

pub fn feed(vt: &mut Vt, bytes: &[u8]) -> Vec<u8> {
    vt.feed(bytes, Instant::now()).live
}

pub fn commit(vt: &mut Vt, presentation: &mut Presentation, bytes: &[u8]) {
    let live = feed(vt, bytes);
    vt.commit_live(presentation, &live);
}

/// Grant and produce a whole restoration with a generous budget.
pub fn restore(vt: &mut Vt, presentation: &mut Presentation) -> Vec<u8> {
    let (mut bytes, mut restoration) = vt.grant(presentation);
    loop {
        let chunk = vt.produce(&mut restoration, presentation, 1 << 20);
        bytes.extend(chunk.bytes);
        if chunk.done {
            return bytes;
        }
    }
}

pub fn restored(vt: &mut Vt) -> Vec<u8> {
    let geometry = vt.geometry();
    restore(vt, &mut Presentation::new(geometry.cols(), geometry.rows()))
}

/// A fresh Alacritty terminal standing in for a host.
pub fn host(cols: usize, rows: usize, bytes: &[u8]) -> Term<VoidListener> {
    let config = Config {
        scrolling_history: 1_000_000,
        ..Config::default()
    };
    let mut term = Term::new(config, &TermSize::new(cols, rows), VoidListener);
    Processor::<StdSyncHandler>::new().advance(&mut term, bytes);
    term
}

fn text(term: &Term<VoidListener>, line: i32) -> String {
    let row = &term.grid()[Line(line)];
    let text: String = (0..term.columns())
        .map(|column| &row[Column(column)])
        .filter(|cell| {
            !cell
                .flags
                .intersects(alacritty_terminal::term::cell::Flags::WIDE_CHAR_SPACER)
        })
        .map(|cell| cell.c)
        .collect();
    text.trim_end().to_owned()
}

pub fn screen(term: &Term<VoidListener>) -> Vec<String> {
    (0..term.screen_lines() as i32)
        .map(|line| text(term, line))
        .collect()
}

pub fn history(term: &Term<VoidListener>) -> Vec<String> {
    let size = term.grid().history_size() as i32;
    (1..=size).rev().map(|age| text(term, -age)).collect()
}

pub fn cursor(term: &Term<VoidListener>) -> (i32, usize, bool) {
    let cursor = &term.grid().cursor;
    (
        cursor.point.line.0,
        cursor.point.column.0,
        cursor.input_needs_wrap,
    )
}

pub fn cell(term: &Term<VoidListener>, line: i32, column: usize) -> Cell {
    term.grid()[Point::new(Line(line), Column(column))].clone()
}

pub fn lines(numbers: std::ops::RangeInclusive<u32>) -> Vec<u8> {
    numbers
        .map(|number| format!("{number}\r\n"))
        .collect::<String>()
        .into_bytes()
}
