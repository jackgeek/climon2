use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::{Config, cell::Flags},
    vte::ansi::{Color, NamedColor, Processor},
};
use native_scrollback_prototype::{Cell, Row, restore, row, sample};
use std::{cell::RefCell, rc::Rc};

struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

#[derive(Clone)]
struct Events(Rc<RefCell<Vec<Event>>>);

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

struct Oracle {
    term: Term<Events>,
    parser: Processor,
    events: Rc<RefCell<Vec<Event>>>,
}

impl Oracle {
    fn new(history: usize) -> Self {
        let events = Rc::new(RefCell::new(Vec::new()));
        Self {
            term: Term::new(
                Config {
                    scrolling_history: history,
                    ..Config::default()
                },
                &Size { rows: 6, cols: 24 },
                Events(events.clone()),
            ),
            parser: Processor::new(),
            events,
        }
    }

    fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    fn lines(&self) -> Vec<String> {
        let grid = self.term.grid();
        (-(grid.history_size() as i32)..grid.screen_lines() as i32)
            .map(|n| {
                (0..grid.columns())
                    .filter_map(|c| {
                        let cell = &grid[Line(n)][Column(c)];
                        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                            return None;
                        }
                        let mut text = cell.c.to_string();
                        if let Some(zero) = cell.zerowidth() {
                            text.extend(zero);
                        }
                        Some(text)
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    fn prime(&mut self) {
        self.feed(b"SAVED-SENTINEL\r\n");
        for n in 0..6 {
            self.feed(format!("VISIBLE-{n}\r\n").as_bytes());
        }
        self.events.borrow_mut().clear();
    }
}

#[test]
fn preserves_saved_and_visible_content_and_seeds_full_history() {
    let mut oracle = Oracle::new(100);
    oracle.prime();
    oracle.feed(&restore(&sample()).unwrap());
    let lines = oracle.lines();
    for sentinel in [
        "SAVED-SENTINEL",
        "VISIBLE-0",
        "VISIBLE-5",
        "HARD-A",
        "HARD-Z",
    ] {
        assert!(
            lines.iter().any(|line| line == sentinel),
            "{sentinel}: {lines:?}"
        );
    }
    assert_eq!(
        &lines[lines.len() - 6..],
        [
            "NORMAL-0", "NORMAL-1", "NORMAL-2", "NORMAL-3", "NORMAL-4", "NORMAL-5"
        ]
    );
    assert!(
        oracle.events.borrow().is_empty(),
        "unexpected terminal side effects: {:?}",
        oracle.events.borrow()
    );
}

#[test]
fn full_resend_on_regain_keeps_duplicates_without_explicit_clear() {
    let mut oracle = Oracle::new(100);
    oracle.prime();
    let bytes = restore(&sample()).unwrap();
    oracle.feed(&bytes);
    oracle.feed(&bytes);
    let lines = oracle.lines();
    assert_eq!(lines.iter().filter(|line| *line == "HARD-A").count(), 2);
    assert!(lines.iter().any(|line| line == "SAVED-SENTINEL"));
}

#[test]
fn finite_host_capacity_evicts_oldest_not_the_latest_screen() {
    let mut oracle = Oracle::new(3);
    oracle.prime();
    oracle.feed(&restore(&sample()).unwrap());
    assert_eq!(oracle.term.grid().history_size(), 3);
    let lines = oracle.lines();
    assert!(!lines.iter().any(|line| line == "SAVED-SENTINEL"));
    assert!(lines.iter().any(|line| line == "HARD-Z"));
    assert_eq!(lines.last().unwrap(), "NORMAL-5");
}

#[test]
fn preserves_soft_wrap_provenance_and_wide_combining_rendition() {
    let mut oracle = Oracle::new(100);
    oracle.feed(&restore(&sample()).unwrap());
    let grid = oracle.term.grid();
    let first = -(grid.history_size() as i32);
    let soft_line = (first..0)
        .find(|n| grid[Line(*n)][Column(0)].c == 'S')
        .unwrap();
    assert!(
        grid[Line(soft_line)][Column(23)]
            .flags
            .contains(Flags::WRAPLINE)
    );
    let wide_line = (first..0)
        .find(|n| grid[Line(*n)][Column(0)].c == '界')
        .unwrap();
    assert!(
        grid[Line(wide_line)][Column(0)]
            .flags
            .contains(Flags::WIDE_CHAR)
    );
    assert_eq!(
        grid[Line(wide_line)][Column(0)].fg,
        Color::Named(NamedColor::Blue)
    );
    assert_eq!(
        grid[Line(wide_line)][Column(2)].zerowidth(),
        Some(&['\u{301}'][..])
    );
    oracle.term.resize(Size { rows: 6, cols: 12 });
    let joined = oracle.lines().join("\n");
    assert!(
        joined.contains("SOFT01234567\n890123456789\ntail"),
        "{joined}"
    );
    assert!(joined.contains("界e\u{301}"), "{joined}");
    assert!(joined.contains("HARD-A\n"), "{joined}");
    oracle.term.resize(Size { rows: 6, cols: 24 });
    assert!(
        oracle
            .lines()
            .join("\n")
            .contains("SOFT01234567890123456789\ntail"),
        "soft line failed to rejoin: {:?}",
        oracle.lines()
    );
}

#[test]
fn alternate_restore_then_return_exposes_underlying_normal_screen_and_history() {
    let mut snapshot = sample();
    snapshot.alternate = Some((0..6).map(|n| row(&format!("ALT-{n}"))).collect());
    let mut oracle = Oracle::new(100);
    oracle.prime();
    oracle.feed(&restore(&snapshot).unwrap());
    assert_eq!(
        oracle.lines(),
        (0..6).map(|n| format!("ALT-{n}")).collect::<Vec<_>>()
    );
    oracle.feed(b"\x1b[?1049l");
    let lines = oracle.lines();
    assert!(lines.iter().any(|line| line == "SAVED-SENTINEL"));
    assert!(lines.iter().any(|line| line == "HARD-A"));
    assert_eq!(lines.last().unwrap(), "NORMAL-5");
    assert!(!lines.iter().any(|line| line.starts_with("ALT-")));
    oracle.feed(b"\r\nAPPLICATION-ECHO\r\n");
    assert!(oracle.lines().iter().any(|line| line == "APPLICATION-ECHO"));
}

#[test]
fn rejects_control_bearing_cells_and_bad_geometry_before_emitting() {
    for text in ["\x1b]52;c;payload\x07", "\u{85}", "\n", "\t", ""] {
        let mut snapshot = sample();
        snapshot.history[0].cells[0].text = text.into();
        assert!(restore(&snapshot).is_err(), "{text:?}");
    }
    let mut snapshot = sample();
    snapshot.history[0].cells[0].width = 0;
    assert!(restore(&snapshot).is_err());
    let mut snapshot = sample();
    snapshot.history[0] = row(&"x".repeat(25));
    assert!(restore(&snapshot).is_err());
    let mut snapshot = sample();
    snapshot.history[0].soft_wrap = true;
    assert!(restore(&snapshot).is_err());
}

#[test]
fn carries_a_full_width_soft_line_across_history_screen_boundary() {
    let mut snapshot = sample();
    snapshot.history = vec![Row {
        cells: row(&"x".repeat(20)).cells,
        soft_wrap: true,
    }];
    snapshot.history[0].cells.push(Cell {
        text: "界".into(),
        width: 2,
        blue: false,
    });
    snapshot.history[0].cells.extend(row("ab").cells);
    let mut oracle = Oracle::new(100);
    oracle.feed(&restore(&snapshot).unwrap());
    let grid = oracle.term.grid();
    assert!(grid[Line(-1)][Column(23)].flags.contains(Flags::WRAPLINE));
    assert_eq!(grid[Line(-1)][Column(20)].c, '界');
    assert_eq!(oracle.lines().last().unwrap(), "NORMAL-5");
}

#[test]
fn emitted_controls_are_only_the_fixture_allowlist_not_historical_side_effects() {
    let mut snapshot = sample();
    snapshot.alternate = Some((0..6).map(|n| row(&format!("ALT-{n}"))).collect());
    let output = String::from_utf8(restore(&snapshot).unwrap()).unwrap();
    let mut remaining = output.as_str();
    while let Some(offset) = remaining.find('\x1b') {
        assert!(
            remaining[..offset]
                .chars()
                .all(|c| !c.is_control() || c == '\r' || c == '\n')
        );
        let control = &remaining[offset..];
        let allowed = [
            "\x1b[?1049l",
            "\x1b[?1049h",
            "\x1b[0m",
            "\x1b[34m",
            "\x1b[?6l",
            "\x1b[4l",
            "\x1b[?7h",
            "\x1b[r",
            "\x1b[6;1H",
            "\x1b[H",
        ];
        let matched = allowed
            .iter()
            .find(|candidate| control.starts_with(**candidate));
        assert!(matched.is_some(), "non-allowlisted control: {control:?}");
        remaining = &control[matched.unwrap().len()..];
    }
    assert!(
        remaining
            .chars()
            .all(|c| !c.is_control() || c == '\r' || c == '\n')
    );
}

#[test]
fn empty_history_and_hard_full_width_rows_restore_without_false_soft_wraps() {
    let mut snapshot = sample();
    snapshot.history = vec![row(&"h".repeat(24))];
    let mut oracle = Oracle::new(100);
    oracle.feed(&restore(&snapshot).unwrap());
    assert!(
        !oracle.term.grid()[Line(-1)][Column(23)]
            .flags
            .contains(Flags::WRAPLINE)
    );
    snapshot.history.clear();
    oracle.feed(&restore(&snapshot).unwrap());
    assert_eq!(oracle.lines().last().unwrap(), "NORMAL-5");
}
