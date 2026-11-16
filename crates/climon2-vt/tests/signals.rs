mod support;

use climon2_vt::{Fed, Signal};
use std::time::Instant;
use support::*;

/// One observed signal, owned, with what the terminal showed just after it.
#[derive(Debug, PartialEq, Eq)]
struct Seen {
    signal: Observed,
    viewport: String,
    title: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum Observed {
    Osc { body: Vec<u8>, length: usize },
    FullReset,
}

fn seen(signal: Observed, viewport: &str, title: Option<&str>) -> Seen {
    Seen {
        signal,
        viewport: viewport.into(),
        title: title.map(str::to_owned),
    }
}

fn osc(body: &[u8], length: usize) -> Observed {
    Observed::Osc {
        body: body.to_vec(),
        length,
    }
}

/// Feeds `reads` in order, recording every signal observed; each signal's
/// observer answers `answer` when one is given.
fn observed(vt: &mut climon2_vt::Vt, reads: &[&[u8]], answer: &[u8]) -> (Vec<Seen>, Vec<Fed>) {
    let mut seen = Vec::new();
    let feds = reads
        .iter()
        .map(|bytes| {
            vt.feed_observed(bytes, Instant::now(), |vt, signal, replies| {
                let signal = match signal {
                    Signal::Osc { body, length } => osc(body, length),
                    Signal::FullReset => Observed::FullReset,
                };
                seen.push(Seen {
                    signal,
                    viewport: vt.viewport_characters(),
                    title: vt.application_title().map(str::to_owned),
                });
                replies.extend_from_slice(answer);
            })
        })
        .collect();
    (seen, feds)
}

/// STA-03, STA-04 and STA-05 (crate case): the session daemon observes
/// every OSC string and full reset (`ESC c`) in stream order, each once the
/// engine executed it, so the viewport and title it can sample are the
/// terminal's at that point of the stream. An OSC string is observed by its
/// body, between `ESC ]` and its terminator (BEL, ST, or the ST a CAN or an
/// interrupting escape implies), and by the whole sequence's length. What the
/// observer answers reaches the program in order with the engine's own
/// replies. Observed sequences never reach the host's live output.
#[test]
fn osc_strings_and_full_resets_are_observed_in_stream_order() {
    let mut vt = vt(5, 1, 10);
    let reads: [&[u8]; 2] = [
        b"a\x1b]133;A\x07\x1b[cb\x1b]7501;state=working\x1b\\c\x1bcd\x1b]2;t\x07",
        b"\x1b]7501;state=idle\x18e\x1b]133;C\x1b[0m",
    ];
    let (seen_signals, feds) = observed(&mut vt, &reads, b"<answer>");
    assert_eq!(
        seen_signals,
        [
            seen(osc(b"133;A", 8), "a    ", None),
            seen(osc(b"7501;state=working", 22), "ab   ", None),
            seen(Observed::FullReset, "     ", None),
            seen(osc(b"2;t", 6), "d    ", Some("t")),
            seen(osc(b"7501;state=idle", 19), "d    ", Some("t")),
            seen(osc(b"133;C", 9), "de   ", Some("t")),
        ]
    );
    assert_eq!(feds[0].replies, b"<answer>\x1b[?6c<answer><answer><answer>");
    assert_eq!(feds[1].replies, b"<answer><answer>");
    for fed in &feds {
        assert_eq!(count(&fed.live, b"133"), 0, "{:?}", fed.live);
        assert_eq!(count(&fed.live, b"7501"), 0, "{:?}", fed.live);
    }
}

/// STA-03 and STA-05 (crate case): an OSC string split across reads is
/// observed once, when its last read completes it; an oversized one, which
/// the engine never receives, is not observed at all, and neither is text
/// that merely looks like a sequence.
#[test]
fn split_oversized_and_textual_sequences_are_observed_as_the_engine_sees_them() {
    let mut vt = vt(5, 1, 10);
    let mut oversized = b"\x1b]7501;".to_vec();
    oversized.resize(4_100, b'a');
    oversized.push(0x07);
    let reads: [&[u8]; 4] = [b"\x1b]133;", b"B\x1b\\", &oversized, b"]133;A\x07"];
    let (seen_signals, _) = observed(&mut vt, &reads, b"");
    assert_eq!(
        seen_signals,
        [seen(osc(b"133;B", 9), "     ", None)],
        "{seen_signals:?}"
    );
}

/// `needle`'s occurrences in `haystack`.
fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack
        .windows(needle.len())
        .filter(|window| *window == needle)
        .count()
}
