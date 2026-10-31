//! THROWAWAY #43: what OSC 8 data unchanged Alacritty retains, and whether
//! output derived from that retained data can carry injected controls.

use alacritty_embedding_probe::restore::{repaint, restore};
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::Processor;

/// Expected retained (explicit id, or engine-generated when `None`; uri).
type Retained = (Option<&'static str>, &'static str);

fn engine(bytes: &[u8]) -> Term<VoidListener> {
    let mut term = Term::new(Config::default(), &TermSize::new(20, 2), VoidListener);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, bytes);
    term
}

fn link_on(term: &Term<VoidListener>, wanted: char) -> Option<Hyperlink> {
    let grid = term.grid();
    (0..grid.screen_lines() as i32)
        .flat_map(|line| {
            (0..grid.columns()).map(move |column| Point::new(Line(line), Column(column)))
        })
        .map(|point| &grid[point])
        .find(|cell| cell.c == wanted)
        .expect("marker cell")
        .hyperlink()
}

/// Independent host-side reading: every OSC 8 payload must be printable
/// UTF-8 free of C0, DEL and C1 code points, because hosts that decode UTF-8
/// before parsing (e.g. xterm.js) treat U+0080..U+009F as controls.
fn unsafe_osc8_payloads(derived: &[u8]) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = derived;
    while let Some(start) = find(rest, b"\x1b]8;") {
        rest = &rest[start + 4..];
        let end = find(rest, b"\x1b\\").expect("derived OSC 8 must be ST-terminated");
        let payload = &rest[..end];
        let safe = std::str::from_utf8(payload).is_ok_and(|text| {
            text.chars()
                .all(|c| !c.is_control() && !('\u{80}'..='\u{9f}').contains(&c))
        });
        if !safe {
            found.push(String::from_utf8_lossy(payload).escape_debug().to_string());
        }
        rest = &rest[end + 2..];
    }
    found
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn derived(term: &mut Term<VoidListener>) -> Vec<u8> {
    [repaint(term), restore(term)].concat()
}

#[test]
fn engine_strips_c0_and_terminates_on_st_bel_can_and_rejects_non_utf8() {
    let cases: [(&[u8], Option<Retained>); 9] = [
        (
            b"\x1b]8;id=a\x01b;http://x/\x02y\x07L",
            Some((Some("ab"), "http://x/y")),
        ),
        (b"\x1b]8;;http://x\x1b\\\x1b[2JL", Some((None, "http://x"))),
        (b"\x1b]8;;http://x\x07\x1b[31mL", Some((None, "http://x"))),
        (b"\x1b]8;;http://a\x18L", Some((None, "http://a"))),
        (b"\x1b]8;;http://\xff\x1b\\L", None),
        (
            b"\x1b]8;id=\xff;http://ok\x1b\\L",
            Some((None, "http://ok")),
        ),
        (
            b"\x1b]8;;http://a;\xff;b\x1b\\L",
            Some((None, "http://a;;b")),
        ),
        (b"\x1b]8;;http://aL\x1b[mM", Some((None, "http://aL"))),
        (
            b"\x1b]8;id=1;http://a\x1b\\\x1b]8;id=2;http://b\x1b\\L",
            Some((Some("2"), "http://b")),
        ),
    ];
    for (input, expected) in cases {
        let mut term = engine(input);
        let marker = if input.ends_with(b"M") { 'M' } else { 'L' };
        let retained = link_on(&term, marker);
        let label = String::from_utf8_lossy(input).escape_debug().to_string();
        match (expected, &retained) {
            (None, None) => (),
            (Some((id, uri)), Some(link)) => {
                assert_eq!(link.uri(), uri, "{label}");
                match id {
                    Some(id) => assert_eq!(link.id(), id, "{label}"),
                    None => assert!(link.id().ends_with("_alacritty"), "{label}"),
                }
            }
            _ => panic!("{label}: retained {retained:?}, expected {expected:?}"),
        }
        let output = derived(&mut term);
        assert_eq!(
            unsafe_osc8_payloads(&output),
            Vec::<String>::new(),
            "{label}"
        );
        let mut host = vt100::Parser::new(
            core::num::NonZeroU16::new(2).unwrap(),
            core::num::NonZeroU16::new(20).unwrap(),
            10,
        );
        host.process(&output);
        assert_eq!(host.screen().contents(), marker.to_string(), "{label}");
        if let Some(link) = retained {
            let wire = format!(";{}\x1b\\", link.uri());
            assert!(
                find(&output, wire.as_bytes()).is_some(),
                "{label}: link not derived"
            );
        }
    }
}

#[test]
fn derived_links_round_trip_with_grouping_and_close() {
    let mut term = engine(b"\x1b]8;id=g;http://a\x1b\\A\x1b]8;;\x1b\\-\x1b]8;id=g;http://a\x1b\\B");
    let mut fresh = engine(&restore(&mut term));
    for marker in ['A', 'B'] {
        let link = link_on(&fresh, marker).expect("link restored");
        assert!(link.id().is_ascii());
        assert_ne!(link.id(), "g");
        assert_eq!(link.uri(), "http://a");
    }
    assert_eq!(link_on(&fresh, '-'), None);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut fresh, b"\r\nZ");
    assert_eq!(
        link_on(&fresh, 'Z').map(|link| link.uri().to_owned()),
        Some("http://a".into())
    );
}

#[test]
fn oversize_uri_is_retained_unbounded() {
    let uri = "a".repeat(1 << 20);
    let mut term = engine(format!("\x1b]8;;{uri}\x1b\\L").as_bytes());
    assert_eq!(link_on(&term, 'L').unwrap().uri().len(), 1 << 20);
    assert_eq!(
        unsafe_osc8_payloads(&derived(&mut term)),
        Vec::<String>::new()
    );
}

#[test]
fn engine_retained_c1_and_del_are_encoded_without_dropping_links() {
    let cases: [(&[u8], &str, &str); 3] = [
        (
            "\x1b]8;;http://a/\u{9c}\u{9b}2J\x1b\\L".as_bytes(),
            "http://a/\u{9c}\u{9b}2J",
            "http://a/%C2%9C%C2%9B2J",
        ),
        (
            "\x1b]8;id=\u{9d}x;http://a\x1b\\L".as_bytes(),
            "http://a",
            "http://a",
        ),
        (
            b"\x1b]8;;http://a/\x7f\x1b\\L",
            "http://a/\x7f",
            "http://a/%7F",
        ),
    ];
    for (input, uri, encoded) in cases {
        let mut term = engine(input);
        assert_eq!(link_on(&term, 'L').unwrap().uri(), uri);
        let output = derived(&mut term);
        assert_eq!(
            unsafe_osc8_payloads(&output),
            Vec::<String>::new(),
            "{uri:?}"
        );
        assert!(find(&output, format!(";{encoded}\x1b\\").as_bytes()).is_some());
    }
}

#[test]
fn every_c1_is_encoded_but_benign_utf8_and_existing_escapes_are_byte_identical() {
    let controls: String = ('\u{80}'..='\u{9f}').chain(['\u{7f}']).collect();
    let mut term = engine(format!("\x1b]8;id={controls};http://a/{controls}\x1b\\L").as_bytes());
    let output = derived(&mut term);
    assert!(unsafe_osc8_payloads(&output).is_empty());
    let expected: String = (0x80..=0x9f)
        .map(|byte| format!("%C2%{byte:02X}"))
        .collect();
    assert!(find(&output, format!(";http://a/{expected}%7F\x1b\\").as_bytes()).is_some());
    for uri in [
        "https://x/a;b?q=%7F&u=%C2%9C#z",
        "https://x/\u{e9}/\u{3042}",
    ] {
        let mut term = engine(format!("\x1b]8;id=benign;{uri}\x1b\\L").as_bytes());
        let output = derived(&mut term);
        assert!(find(&output, format!(";{uri}\x1b\\").as_bytes()).is_some());
    }
}

#[test]
fn remapped_ids_distinguish_groups_and_reuse_nonadjacent_group_tokens() {
    let mut term = engine(
        "\x1b]8;id=\u{9d}x;http://a\x1b\\A\x1b]8;id=other;http://a\x1b\\C\
         \x1b]8;id=\u{9d}x;http://a\x1b\\B\x1b]8;;\x1b\\-"
            .as_bytes(),
    );
    let fresh = engine(&restore(&mut term));
    let a = link_on(&fresh, 'A').unwrap();
    let b = link_on(&fresh, 'B').unwrap();
    let c = link_on(&fresh, 'C').unwrap();
    assert_eq!(a, b);
    assert_ne!(a.id(), c.id());
    assert_eq!(a.uri(), c.uri());
    assert!(a.id().is_ascii() && c.id().is_ascii());
    assert_eq!(link_on(&fresh, '-'), None);
}

#[test]
fn tokens_keep_groups_distinct_across_separate_derived_outputs() {
    let mut a = engine(b"\x1b]8;id=first;http://a\x1b\\L");
    let mut b = engine(b"\x1b]8;id=second;http://a\x1b\\L");
    let a = engine(&restore(&mut a));
    let b = engine(&restore(&mut b));
    assert_ne!(
        link_on(&a, 'L').unwrap().id(),
        link_on(&b, 'L').unwrap().id()
    );
}

#[test]
fn encoded_uri_aliases_do_not_merge_original_link_groups() {
    let mut term = engine(
        "\x1b]8;id=same;http://a/\u{9c}\x1b\\A\
         \x1b]8;id=same;http://a/%C2%9C\x1b\\B"
            .as_bytes(),
    );
    let restored = engine(&restore(&mut term));
    let a = link_on(&restored, 'A').unwrap();
    let b = link_on(&restored, 'B').unwrap();
    assert_eq!(a.uri(), b.uri());
    assert_ne!(
        a.id(),
        b.id(),
        "encoding must not merge previously distinct groups"
    );
}

#[test]
fn independent_vt100_host_is_not_erased_by_encoded_links() {
    use core::num::NonZeroU16;
    let mut term =
        engine("SAFE\r\n\x1b]8;id=\u{9d}x;http://a/\u{9c}\u{9b}2J\x1b\\L\x1b]8;;\x1b\\".as_bytes());
    for output in [repaint(&mut term), restore(&mut term)] {
        let mut host = vt100::Parser::new(
            NonZeroU16::new(2).unwrap(),
            NonZeroU16::new(20).unwrap(),
            10,
        );
        host.process(&output);
        assert_eq!(host.screen().contents(), "SAFE\nL");
    }
}

/// Contrast: printable cell text cannot hold C1, because `Term::input` drops
/// characters without a display width (term/mod.rs:1064-1066 at the pin).
#[test]
fn c1_in_ground_text_is_not_retained_or_derived() {
    let mut term = engine("A\u{9b}2J\u{9c}B".as_bytes());
    let output = derived(&mut term);
    assert!(find(&output, "A2JB".as_bytes()).is_some());
    assert!(
        !String::from_utf8_lossy(&output)
            .chars()
            .any(|c| ('\u{80}'..='\u{9f}').contains(&c))
    );
}
