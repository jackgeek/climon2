//! One parser per monitor (#166): every session keeps its own model, but one
//! `Parser` serves them all. Each model must receive only its own tokens,
//! whatever the others were fed in between, and must repeat its own
//! preceding character for `CSI n b`. Expectations are hand-written.

use climon2_vt::{Geometry, Limits, Model, Parser};

const ROOMY: usize = 64 * 1024 * 1024;

fn model(cols: u16, rows: u16, history_rows: usize) -> Model {
    let limits = Limits {
        bytes: ROOMY,
        history_rows,
    };
    Model::new(Geometry::for_host(cols, rows).unwrap(), limits).unwrap()
}

/// Feed each `(model, bytes)` in turn through one parser.
fn interleave(models: &mut [Model], feeds: &[(usize, &[u8])]) {
    let mut parser = Parser::new();
    for &(index, bytes) in feeds {
        models[index].feed(&mut parser, bytes).unwrap();
    }
}

/// LIM-04 Contract (shared resources count once): a partial escape sequence
/// and a UTF-8 sequence cut off by an escape stay with the model they were
/// fed to while another model's output passes through the same parser. The
/// cut-off sequence prints one replacement character in its own model.
#[test]
fn each_model_receives_only_its_own_tokens() {
    let mut models = [model(10, 2, 0), model(10, 2, 0)];
    interleave(
        &mut models,
        &[
            (0, b"x\xe2\x82\x1b"),
            (1, b"y"),
            (0, b"[31mz\x1b["),
            (1, b"3Cb"),
            (0, b"2Ca"),
        ],
    );
    assert_eq!(models[0].screen(), ["x\u{fffd}z  a", ""]);
    assert_eq!(models[1].screen(), ["y3Cb", ""]);
}

/// LIM-04 Contract: each model repeats the character it printed last, not
/// the one another model printed through the shared parser; a model that
/// never printed anything repeats nothing.
#[test]
fn each_model_repeats_its_own_preceding_character() {
    let mut models = [model(10, 2, 0), model(10, 2, 0)];
    interleave(
        &mut models,
        &[
            (0, b"a"),
            (1, b"\x1b[3b"),
            (1, b"q"),
            (0, b"\x1b[2b"),
            (1, b"\x1b[1b"),
            (0, b"\x1b[1b"),
        ],
    );
    assert_eq!(models[0].screen(), ["aaaa", ""]);
    assert_eq!(models[1].screen(), ["qq", ""]);
}

/// The screen of a fresh 10x2 model fed `bytes`.
fn screen(bytes: &[u8]) -> Vec<String> {
    let mut models = [model(10, 2, 0)];
    interleave(&mut models, &[(0, bytes)]);
    models[0].screen()
}

/// TRM-01 Contract: `CSI n b` repeats the preceding character `n` times,
/// once for an absent or zero count; the controls executed inside the
/// sequence leave the preceding character as it was, and so do controls and
/// sequences before it, C1 controls (encoded, or a lone byte up to 0x9f)
/// included. An invalid byte above 0x9f, or a sequence cut off by an
/// escape, prints a replacement character, which is then the one repeated.
#[test]
fn repeat_counts_at_their_lower_boundaries() {
    for (bytes, expected) in [
        (&b"a\x1b[b"[..], ["aa", ""]),
        (b"a\x1b[0b", ["aa", ""]),
        (b"a\x1b[1b", ["aa", ""]),
        (b"a\x1b[2b", ["aaa", ""]),
        (b"a\x1b[2bz", ["aaaz", ""]),
        (b"a\x1b[\r\n2b", ["a", "aa"]),
        (b"a\x1b[1m\n\x1b[1b", ["a", " a"]),
        (b"\xff\x1b[1b", ["\u{fffd}\u{fffd}", ""]),
        (b"\x1b[5b", ["", ""]),
        (b"a\x1f\x1b[1b", ["aa", ""]),
        ("a\u{80}\x1b[1b".as_bytes(), ["aa", ""]),
        ("a\u{9f}\x1b[1b".as_bytes(), ["aa", ""]),
        ("a\u{a1}\x1b[1b".as_bytes(), ["a\u{a1}\u{a1}", ""]),
        (b"a\x9f\x1b[1b", ["aa", ""]),
        (b"a\xa0\x1b[1b", ["a\u{fffd}\u{fffd}", ""]),
        (b"a\xe2\x82\x1b[1b", ["a\u{fffd}\u{fffd}", ""]),
        (b"a\xe2\x1b[1bz", ["a\u{fffd}\u{fffd}z", ""]),
    ] {
        assert_eq!(screen(bytes), expected, "{bytes:?}");
    }
}

/// TRM-01 Contract: a wide character repeats as wide characters, and one
/// that no longer fits at the right margin wraps to the next row, as does a
/// narrow repeat past the margin.
#[test]
fn repeats_after_a_wide_character_and_at_the_right_margin() {
    for (bytes, expected) in [
        ("\u{4e00}\x1b[2b", ["\u{4e00}\u{4e00}\u{4e00}", ""]),
        ("123456789a\x1b[2b", ["123456789a", "aa"]),
        (
            "123456789\u{4e00}\x1b[1b",
            ["123456789", "\u{4e00}\u{4e00}"],
        ),
        ("12345678\u{4e00}\x1b[1b", ["12345678\u{4e00}", "\u{4e00}"]),
    ] {
        assert_eq!(screen(bytes.as_bytes()), expected, "{bytes:?}");
    }
}

/// TRM-01 Contract: the largest count the parser keeps, 65535, repeats that
/// many times; one past it repeats the same number of times.
#[test]
fn repeat_counts_at_the_maximum_and_one_past_it() {
    for repeat in [&b"\x1b[65535b"[..], b"\x1b[65536b"] {
        let mut models = [model(10, 2, 10_000)];
        interleave(&mut models, &[(0, b"a"), (0, repeat)]);
        // 65,536 characters: 6,553 full rows of ten and six on the last.
        let history = models[0].history();
        assert_eq!(history, vec!["a".repeat(10); 6552], "{repeat:?}");
        assert_eq!(models[0].screen(), ["a".repeat(10), "a".repeat(6)]);
    }
}

/// TRM-01 Contract: a restoration starts a fresh model with no preceding
/// character, and once it is applied the model repeats what it printed.
#[test]
fn a_restoration_repeats_only_what_it_printed() {
    let mut parser = Parser::new();
    let mut session = model(10, 2, 0);
    session.feed(&mut parser, b"a").unwrap();
    session.stage(session.geometry()).unwrap();
    session.feed(&mut parser, b"\x1b[2b").unwrap();
    session.apply();
    assert_eq!(session.screen(), ["", ""]);

    session.feed(&mut parser, b"b\x1b[1b").unwrap();
    assert_eq!(session.screen(), ["bb", ""]);
}
