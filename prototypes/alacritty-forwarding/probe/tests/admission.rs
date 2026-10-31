//! THROWAWAY #48/#43: control-string admission ahead of the unchanged engine.

use alacritty_embedding_probe::admission::OscAdmission;
use alacritty_embedding_probe::view;
use std::cell::RefCell;
use std::rc::Rc;

use alacritty_terminal::event::{Event, EventListener, VoidListener};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term};
use vte::ansi::{Processor, StdSyncHandler};

fn term() -> Term<VoidListener> {
    Term::new(Config::default(), &TermSize::new(12, 3), VoidListener)
}

#[derive(Clone, Default)]
struct Titles(Rc<RefCell<Vec<String>>>);

impl EventListener for Titles {
    fn send_event(&self, event: Event) {
        if let Event::Title(title) = event {
            self.0.borrow_mut().push(title);
        }
    }
}

fn feed<T: EventListener>(
    term: &mut Term<T>,
    admission: &mut OscAdmission,
    chunks: &[&[u8]],
) -> Vec<u8> {
    let mut parser = Processor::<StdSyncHandler>::new();
    let mut forwarded = Vec::new();
    for chunk in chunks {
        let mut out = Vec::new();
        admission.admit(chunk, &mut out);
        parser.advance(term, &out);
        forwarded.extend(out);
    }
    forwarded
}

fn link_uri(term: &Term<VoidListener>, column: usize) -> Option<String> {
    term.grid()[Line(0)][Column(column)]
        .hyperlink()
        .map(|link| link.uri().to_owned())
}

/// Deterministic xorshift so every run replays the same streams.
struct Stream(u64);

impl Stream {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn bytes(&mut self, len: usize) -> Vec<u8> {
        const ALPHABET: &[u8] =
            b"\x1b\x1b\x1b]]]\x07\x18\x1a\\8;;a b#[?2026hlP_^X\x05\x7f\x9c\xc2\x9d\xe2\x82\xac(0m";
        (0..len)
            .map(|_| ALPHABET[(self.next() % ALPHABET.len() as u64) as usize])
            .collect()
    }
}

#[test]
fn admission_within_the_cap_is_byte_transparent_at_every_chunk_boundary() {
    let mut stream = Stream(0x9e37_79b9_7f4a_7c15);
    for case in 0..1_500 {
        let input = stream.bytes(1 + case % 97);
        for chunk_size in [1, 2, 3, 5, 64] {
            let mut admission = OscAdmission::new(input.len());
            let mut admitted = term();
            let chunks: Vec<&[u8]> = input.chunks(chunk_size).collect();
            let mut forwarded = feed(&mut admitted, &mut admission, &chunks);
            forwarded.extend(admission.pending());
            assert_eq!(forwarded, input, "case {case} chunk {chunk_size}");
            assert_eq!(admission.breaches(), 0);

            // vte's own partial-UTF-8 recovery depends on feed boundaries, so
            // the control engine sees the same chunks.
            let mut control = term();
            let mut parser = Processor::<StdSyncHandler>::new();
            for chunk in &chunks {
                parser.advance(&mut control, chunk);
            }
            if admission.pending().is_empty() {
                assert_eq!(
                    view(&admitted),
                    view(&control),
                    "case {case} chunk {chunk_size}"
                );
            }
        }
    }
}

#[test]
fn oversized_hyperlink_is_never_admitted_and_later_text_is_unaffected() {
    let long = format!("https://a.invalid/{}", "x".repeat(64));
    let input = format!("a\x1b]8;;{long}\x1b\\b\x1b]8;;\x1b\\c\x1b]8;;https://ok\x07d");
    for chunk_size in [1, 2, 7, 4096] {
        let mut admission = OscAdmission::new(32);
        let mut admitted = term();
        let chunks: Vec<&[u8]> = input.as_bytes().chunks(chunk_size).collect();
        feed(&mut admitted, &mut admission, &chunks);
        assert_eq!(&view(&admitted).text()[0][..4], "abcd");
        assert_eq!(link_uri(&admitted, 0), None);
        assert_eq!(link_uri(&admitted, 1), None, "chunk {chunk_size}");
        assert_eq!(link_uri(&admitted, 2), None);
        assert_eq!(link_uri(&admitted, 3).as_deref(), Some("https://ok"));
        assert_eq!(admission.breaches(), 1);
    }
}

#[test]
fn oversized_titles_are_dropped_whatever_terminates_them() {
    let cases: &[(&[u8], &str)] = &[
        (b"\x1b]2;0123456789ABCDEF\x07X", "X           "),
        (b"\x1b]2;0123456789ABCDEF\x1b\\X", "X           "),
        (b"\x1b]2;0123456789ABCDEF\x18X", "X           "),
        (b"\x1b]2;0123456789ABCDEF\x1aY", "Y           "),
        (b"\x1b]2;0123456789ABCDEF\x1b[2CX", "  X         "),
        (b"\x1b]2;0123456789ABCDEF\x1b\x1b[1CX", " X          "),
    ];
    for (input, row) in cases {
        let titles = Titles::default();
        let mut admission = OscAdmission::new(8);
        let mut admitted = Term::new(Config::default(), &TermSize::new(12, 3), titles.clone());
        feed(&mut admitted, &mut admission, &[input, b"\x1b]2;short\x07"]);
        assert_eq!(view(&admitted).text()[0], *row, "{input:?}");
        assert_eq!(*titles.0.borrow(), ["short"], "{input:?}");
        assert_eq!(admission.breaches(), 1);
    }
}

#[test]
fn unterminated_oversized_string_retains_at_most_the_cap() {
    let cap = 4096;
    let mut admission = OscAdmission::new(cap);
    let mut admitted = term();
    let mut parser = Processor::<StdSyncHandler>::new();
    let mut out = Vec::new();
    admission.admit(b"\x1b]8;;", &mut out);
    let body = vec![b'u'; 64 * 1024];
    for _ in 0..64 {
        out.clear();
        admission.admit(&body, &mut out);
        assert!(out.is_empty());
        assert!(admission.retained_capacity() <= cap);
    }
    out.clear();
    admission.admit(b"\x07Z", &mut out);
    parser.advance(&mut admitted, &out);
    assert_eq!(view(&admitted).text()[0], "Z           ");
    assert_eq!(link_uri(&admitted, 0), None);
    assert_eq!(admission.breaches(), 1);
}
