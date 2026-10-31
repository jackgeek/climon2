//! THROWAWAY #48/#43: public seams, independent replay, and restore ordering.
use alacritty_embedding_probe::followup::{PtyBuffer, Resizes, select_shared_link};
use alacritty_embedding_probe::restore::{SlicedRestore, Snapshot, restore};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use std::num::NonZeroU16;
use vte::ansi::{Processor, StdSyncHandler};

fn engine(bytes: &[u8]) -> Term<VoidListener> {
    let mut term = Term::new(
        Config {
            scrolling_history: 3,
            ..Config::default()
        },
        &TermSize::new(12, 3),
        VoidListener,
    );
    Processor::<StdSyncHandler>::new().advance(&mut term, bytes);
    term
}

#[test]
fn a_single_osc_shares_uri_storage_across_cells_even_after_copy_on_write() {
    let mut term = engine(b"\x1b]8;id=a;https://a\x1b\\abc\x1b]8;;\x1b\\");
    let a = term.grid()[Line(0)][Column(0)].hyperlink().unwrap();
    term.grid_mut()[Line(0)][Column(1)].push_zerowidth('\u{301}');
    let b = term.grid()[Line(0)][Column(1)].hyperlink().unwrap();
    assert_eq!(a.uri().as_ptr(), b.uri().as_ptr(), "URI was deep-copied");
}

#[test]
fn public_template_setter_reuses_an_interned_link_across_separate_oscs() {
    let mut term = engine(b"");
    let shared = Hyperlink::new(Some("a"), "https://a".into());
    for _ in 0..3 {
        Processor::<StdSyncHandler>::new().advance(&mut term, b"\x1b]8;id=a;https://a\x1b\\");
        select_shared_link(&mut term, shared.clone());
        Processor::<StdSyncHandler>::new().advance(&mut term, b"x");
    }
    for column in 0..3 {
        let link = term.grid()[Line(0)][Column(column)].hyperlink().unwrap();
        assert_eq!(
            link.uri().as_ptr(),
            shared.uri().as_ptr(),
            "interning seam absent"
        );
    }
}

#[test]
fn cell_extra_weak_is_not_a_hyperlink_lifetime_notification() {
    let mut term = engine(b"\x1b]8;id=a;https://a\x1b\\x\x1b]8;;\x1b\\");
    let weak = std::sync::Arc::downgrade(term.grid()[Line(0)][Column(0)].extra.as_ref().unwrap());
    let link = term.grid()[Line(0)][Column(0)].hyperlink().unwrap();
    term.grid_mut()[Line(0)][Column(0)].push_zerowidth('\u{301}');
    assert!(
        weak.upgrade().is_none(),
        "Arc::make_mut disassociates Weak<CellExtra>"
    );
    let still_live = term.grid()[Line(0)][Column(0)].hyperlink().unwrap();
    assert_eq!(still_live.uri().as_ptr(), link.uri().as_ptr());
    assert_eq!(still_live.uri(), "https://a");
}

const NORMAL: &[u8] = b"h0\r\nh1\r\nh2\r\n\x1b[1mB\x1b[0mn1\r\nn2";
const ALT: &[u8] = b"\x1b[?1049h\x1b[2;3HALT";
const ARRIVAL: &[u8] = b"\x1b[1;1H\x1b[0mNEW";

fn oracle(bytes: &[u8], alt: bool) {
    let mut host = vt100::Parser::new(NonZeroU16::new(3).unwrap(), NonZeroU16::new(12).unwrap(), 3);
    host.process(bytes);
    assert_eq!(host.screen().alternate_screen(), alt);
    let rows: Vec<_> = host.screen().rows(0, 12).collect();
    assert_eq!(
        rows,
        if alt {
            vec!["", "  ALT", ""]
        } else {
            vec!["h2", "Bn1", "n2"]
        }
    );
    if alt {
        host.process(b"\x1b[?1049l");
    }
    assert_eq!(host.screen().cell(1, 0).unwrap().contents(), "B");
    assert!(host.screen().cell(1, 0).unwrap().bold());
    host.screen_mut().set_scrollback(2);
    assert_eq!(host.screen().rows(0, 12).next().unwrap(), "h0");
    host.screen_mut().set_scrollback(0);
}

#[test]
fn clone_worker_snapshot_is_consistent_while_new_output_changes_the_engine() {
    for alt in [false, true] {
        let mut term = engine(&[NORMAL, if alt { ALT } else { b"" }].concat());
        let expected = restore(&mut term);
        let snapshot = Snapshot::capture(&mut term);
        Processor::<StdSyncHandler>::new().advance(&mut term, ARRIVAL);
        let bytes = std::thread::spawn(move || snapshot.serialize())
            .join()
            .unwrap();
        assert_eq!(bytes, expected, "snapshot changed with live output");
        oracle(&bytes, alt);
        let mut host =
            vt100::Parser::new(NonZeroU16::new(3).unwrap(), NonZeroU16::new(12).unwrap(), 3);
        host.process(&bytes);
        host.process(ARRIVAL);
        assert_eq!(host.screen().cell(0, 0).unwrap().contents(), "N");
        assert_eq!(term.grid()[Line(0)][Column(0)].c, 'N');
    }
}

#[test]
fn clone_extra_copy_on_write_preserves_wide_combining_and_links_at_capture() {
    let mut term = engine("\x1b]8;id=a;https://a\x1b\\\u{754c}e\u{301}".as_bytes());
    let expected = restore(&mut term);
    let snapshot = Snapshot::capture(&mut term);
    Processor::<StdSyncHandler>::new().advance(&mut term, "\u{308}".as_bytes());
    let bytes = std::thread::spawn(move || snapshot.serialize())
        .join()
        .unwrap();
    assert_eq!(bytes, expected);
    let mut host = vt100::Parser::new(NonZeroU16::new(3).unwrap(), NonZeroU16::new(12).unwrap(), 3);
    host.process(&bytes);
    assert_eq!(host.screen().contents(), "\u{754c}e\u{301}");
    assert_eq!(
        term.grid()[Line(0)][Column(2)].zerowidth().unwrap(),
        ['\u{301}', '\u{308}']
    );
    assert_eq!(
        term.grid()[Line(0)][Column(2)].hyperlink().unwrap().uri(),
        "https://a"
    );
}

#[test]
fn sliced_restore_freezes_one_snapshot_and_replays_buffered_output_afterward() {
    for alt in [false, true] {
        let mut term = engine(&[NORMAL, if alt { ALT } else { b"" }].concat());
        let expected = restore(&mut term);
        let mut waiting = PtyBuffer::new(ARRIVAL.len());
        let mut sliced = SlicedRestore::new(&mut term);
        assert!(!sliced.step(1));
        waiting.push(&ARRIVAL[..4]).unwrap();
        waiting.push(&ARRIVAL[4..]).unwrap();
        while !sliced.step(1) {}
        let bytes = sliced.finish();
        assert_eq!(bytes, expected);
        oracle(&bytes, alt);
        let live = waiting.drain();
        let mut host =
            vt100::Parser::new(NonZeroU16::new(3).unwrap(), NonZeroU16::new(12).unwrap(), 3);
        host.process(&bytes);
        host.process(&live);
        Processor::<StdSyncHandler>::new().advance(&mut term, &live);
        assert_eq!(term.grid()[Line(0)][Column(0)].c, 'N');
        assert_eq!(host.screen().cell(0, 0).unwrap().contents(), "N");
    }
}

#[test]
fn pty_buffer_refuses_overflow_without_losing_or_reordering_accepted_bytes() {
    let mut waiting = PtyBuffer::new(5);
    waiting.push(b"abc").unwrap();
    assert_eq!(waiting.push(b"def"), Err("PTY buffer full"));
    waiting.push(b"de").unwrap();
    assert_eq!(waiting.drain(), b"abcde");
}

#[test]
fn back_to_back_resizes_apply_only_the_final_geometry_and_reflow_once() {
    let mut term = engine(b"abcdefghijklmnop");
    let mut expected = engine(b"abcdefghijklmnop");
    expected.resize(TermSize::new(9, 4));
    let mut resizes = Resizes::default();
    resizes.request(11, 3);
    resizes.request(8, 3);
    resizes.request(9, 4);
    assert!(resizes.apply(&mut term));
    assert!(!resizes.apply(&mut term));
    assert_eq!(resizes.reflows(), 1);
    assert_eq!(term.columns(), 9);
    assert_eq!(
        alacritty_embedding_probe::view(&term),
        alacritty_embedding_probe::view(&expected)
    );
}
