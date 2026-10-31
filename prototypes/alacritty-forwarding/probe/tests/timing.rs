use std::time::{Duration, Instant};

use alacritty_embedding_probe::timing::{FirstBeginTimeout, flush_if_due};
use alacritty_embedding_probe::{Observed, view};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::{Processor, Timeout};

#[test]
fn first_deadline_flushes_without_more_input_and_does_not_repeat() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut observed = Observed::new(&mut term);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    let before = Instant::now();
    parser.advance(&mut observed, b"\x1b[?2026hABC");
    let after = Instant::now();
    let deadline = parser.sync_timeout().deadline().unwrap();
    assert!(deadline >= before + Duration::from_millis(150));
    assert!(deadline <= after + Duration::from_millis(150));
    assert_eq!(view(observed.term).text()[0], "          ");
    assert_eq!(parser.sync_bytes_count(), 3);
    assert!(parser.sync_timeout().pending_timeout());
    assert!(!flush_if_due(
        &mut parser,
        &mut observed,
        deadline - Duration::from_nanos(1)
    ));
    assert_eq!(view(observed.term).text()[0], "          ");
    assert!(flush_if_due(&mut parser, &mut observed, deadline));
    assert_eq!(view(observed.term).text()[0], "ABC       ");
    assert_eq!(view(observed.term).cursor, (0, 3));
    assert_eq!(parser.sync_bytes_count(), 0);
    assert_eq!(parser.sync_timeout().deadline(), None);
    assert!(!flush_if_due(&mut parser, &mut observed, deadline));
}

#[test]
fn repeated_fragmented_begins_cannot_extend_the_first_deadline() {
    for chunk_size in [1, 2, 7, 65536] {
        let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
        let mut observed = Observed::new(&mut term);
        let mut parser = Processor::<FirstBeginTimeout>::new();
        parser.advance(&mut observed, b"\x1b[?2026hA");
        let deadline = parser.sync_timeout().deadline().unwrap();
        for bytes in b"\x1b[?2026hB\x1b[?2026hC".chunks(chunk_size) {
            parser.advance(&mut observed, bytes);
            assert_eq!(parser.sync_timeout().deadline(), Some(deadline));
        }
        assert_eq!(view(observed.term).text()[0], "          ");
        assert!(flush_if_due(&mut parser, &mut observed, deadline));
        assert_eq!(view(observed.term).text()[0], "ABC       ");
    }
}

#[test]
fn explicit_end_cancels_the_old_timer_and_next_begin_rearms() {
    let mut term = Term::new(Config::default(), &TermSize::new(10, 3), VoidListener);
    let mut observed = Observed::new(&mut term);
    let mut parser = Processor::<FirstBeginTimeout>::new();
    parser.advance(&mut observed, b"\x1b[?2026hA");
    let old = parser.sync_timeout().deadline().unwrap();
    parser.advance(&mut observed, b"\x1b[?2026l");
    assert_eq!(view(observed.term).text()[0], "A         ");
    assert_eq!(parser.sync_timeout().deadline(), None);
    assert!(!flush_if_due(&mut parser, &mut observed, old));
    parser.advance(&mut observed, b"\x1b[?2026hB");
    let new = parser.sync_timeout().deadline().unwrap();
    assert!(new >= old);
    assert_eq!(view(observed.term).text()[0], "A         ");
    assert!(flush_if_due(&mut parser, &mut observed, new));
    assert_eq!(view(observed.term).text()[0], "AB        ");
}
