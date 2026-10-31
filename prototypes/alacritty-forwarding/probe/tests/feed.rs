use std::time::{Duration, Instant};

use alacritty_embedding_probe::feed::{EngineFeed, Step};
use alacritty_embedding_probe::timing::FirstBeginTimeout;
use alacritty_embedding_probe::view;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::{Handler, Processor};

#[derive(Default)]
struct Inputs(u64);

impl Handler for Inputs {
    fn input(&mut self, _: char) {
        self.0 += 1;
    }
}

const REP: &[u8] = b"X\x1b[65535b";

#[test]
fn engine_work_per_step_is_bounded_by_slice_plus_sync_hold() {
    let (slice, hold) = (64, 256);
    let mut feed = EngineFeed::new(slice, hold);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    let mut handler = Inputs::default();
    let mut stream = b"\x1b[?2026h".to_vec();
    let units = 65536 / REP.len();
    for _ in 0..units {
        stream.extend(REP);
    }
    feed.push(&stream);
    let now = Instant::now();
    let (mut worst, mut steps) = (0, 0);
    while feed.pending() > 0 {
        let before = handler.0;
        assert!(matches!(
            feed.step(&mut parser, &mut handler, now),
            Step::Advanced { .. }
        ));
        worst = worst.max(handler.0 - before);
        steps += 1;
        assert!(parser.sync_bytes_count() <= hold);
    }
    let per_unit = 65536;
    let bound = (slice + hold).div_ceil(REP.len()) as u64 * per_unit;
    assert!(worst <= bound, "worst {worst} > bound {bound}");
    assert!(feed.sync_hold_breaches() > 0);
    assert!(steps >= stream.len() / slice);
    feed.flush(&mut parser, &mut handler);
    assert_eq!(handler.0, units as u64 * per_unit);
}

#[test]
fn sync_hold_breach_flushes_early_and_preserves_content() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    let mut feed = EngineFeed::new(4, 8);
    feed.push(b"\x1b[?2026hABCDEFGHIJ");
    let now = Instant::now();
    while feed.pending() > 0 {
        feed.step(&mut parser, &mut term, now);
    }
    assert_eq!(feed.sync_hold_breaches(), 1);
    assert_eq!(parser.sync_bytes_count(), 0);
    assert_eq!(parser.sync_timeout().deadline(), None);
    assert_eq!(view(&term).text()[0], "ABCDEFGHIJ");
}

#[test]
fn zero_hold_never_leaves_bytes_buffered_in_the_engine() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    let mut feed = EngineFeed::new(3, 0);
    feed.push(b"\x1b[?2026hAB\x1b[?2026l\x1b[?2026hCDE");
    let now = Instant::now();
    while feed.pending() > 0 {
        feed.step(&mut parser, &mut term, now);
        assert_eq!(parser.sync_bytes_count(), 0);
    }
    assert_eq!(view(&term).text()[0], "ABCDE     ");
}

#[test]
fn updates_under_the_hold_wait_for_end_or_deadline() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    let mut feed = EngineFeed::new(4, 64);
    feed.push(b"\x1b[?2026hABC");
    let now = Instant::now();
    while feed.pending() > 0 {
        feed.step(&mut parser, &mut term, now);
    }
    assert_eq!(feed.sync_hold_breaches(), 0);
    assert_eq!(view(&term).text()[0], "          ");
    let deadline = parser.sync_timeout().deadline().unwrap();
    assert_eq!(
        feed.step(&mut parser, &mut term, deadline - Duration::from_nanos(1)),
        Step::Idle
    );
    assert_eq!(view(&term).text()[0], "          ");
    assert_eq!(
        feed.step(&mut parser, &mut term, deadline),
        Step::DeadlineFlushed
    );
    assert_eq!(view(&term).text()[0], "ABC       ");
}
