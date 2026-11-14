use alacritty_terminal::vte::{Params, Parser, Perform};
use climon2_vt::token::{Kind, MAX_BODY, Token, Tokenizer};

fn tokens(reads: &[&[u8]]) -> Vec<(Kind, Vec<u8>)> {
    let mut tokenizer = Tokenizer::new();
    reads
        .iter()
        .flat_map(|read| tokenizer.push(read))
        .map(|Token { kind, bytes }| (kind, bytes))
        .collect()
}

fn text(bytes: &[u8]) -> (Kind, Vec<u8>) {
    (Kind::Text, bytes.to_vec())
}

fn token(kind: Kind, bytes: &[u8]) -> (Kind, Vec<u8>) {
    (kind, bytes.to_vec())
}

#[test]
fn text_and_escape_sequences_become_separate_tokens() {
    assert_eq!(
        tokens(&[b"ab\r\n\x1b[31mc\x1b7\x1b(Bd"]),
        [
            text(b"ab\r\n"),
            token(Kind::Csi, b"\x1b[31m"),
            text(b"c"),
            token(Kind::Escape, b"\x1b7"),
            token(Kind::Escape, b"\x1b(B"),
            text(b"d"),
        ]
    );
}

#[test]
fn every_introducer_selects_its_kind() {
    assert_eq!(
        tokens(&[b"\x1b]0;t\x07\x1bPq\x1b\\\x1bXs\x1b\\\x1b^p\x1b\\\x1b_a\x1b\\"]),
        [
            token(Kind::Osc, b"\x1b]0;t\x07"),
            token(Kind::Dcs, b"\x1bPq\x1b\\"),
            token(Kind::Sos, b"\x1bXs\x1b\\"),
            token(Kind::Sos, b"\x1b^p\x1b\\"),
            token(Kind::Sos, b"\x1b_a\x1b\\"),
        ]
    );
}

#[test]
fn a_token_split_across_reads_completes_once() {
    assert_eq!(
        tokens(&[b"x\x1b[", b"3", b"1", b"mz\x1b]2;ti", b"tle\x1b", b"\\"]),
        [
            text(b"x"),
            token(Kind::Csi, b"\x1b[31m"),
            text(b"z"),
            token(Kind::Osc, b"\x1b]2;title\x1b\\"),
        ]
    );
}

#[test]
fn unfinished_utf8_waits_for_the_rest_of_its_sequence() {
    let euro = "€".as_bytes();
    assert_eq!(
        tokens(&[b"a\xe2", b"\x82", &euro[2..], b"\xf0\x9f\x98", b"\x80"]),
        [text(b"a"), text(euro), text("😀".as_bytes()),]
    );
}

#[test]
fn invalid_utf8_is_text_and_never_held() {
    assert_eq!(
        tokens(&[b"\xe2\xe2\x82", b"\x1b7", b"\xff", b"\x80"]),
        [
            text(b"\xe2"),
            text(b"\xe2\x82"),
            token(Kind::Escape, b"\x1b7"),
            text(b"\xff"),
            text(b"\x80"),
        ]
    );
}

#[test]
fn c0_controls_inside_escape_sequences_keep_their_order() {
    assert_eq!(
        tokens(&[b"\x1b[3\n1\x08m\x1b(\rB\x1b\x07c"]),
        [
            token(Kind::Csi, b"\x1b[3\n1\x08m"),
            token(Kind::Escape, b"\x1b(\rB"),
            token(Kind::Escape, b"\x1b\x07c"),
        ]
    );
}

#[test]
fn c0_controls_inside_an_escape_sequence_remain_bounded() {
    let mut bytes = vec![0x1b, b'['];
    bytes.resize(MAX_BODY + 3, 0x1c);
    bytes.push(b'm');
    assert_eq!(tokens(&[&bytes]), [token(Kind::Oversized, b"")]);
}

#[test]
fn can_and_sub_abort_escape_sequences() {
    assert_eq!(
        tokens(&[b"\x1b[31\x18m\x1b(\x1aB\x1b\x18c"]),
        [
            text(b"\x18"),
            text(b"m"),
            text(b"\x1a"),
            text(b"B"),
            text(b"\x18"),
            text(b"c"),
        ]
    );
}

/// LIM-03: the controls an abandoned sequence held are released as one text
/// token, so a monitor model holds one buffer for thousands of them.
#[test]
fn controls_held_by_an_abandoned_sequence_are_one_text_token() {
    assert_eq!(
        tokens(&[b"\x1b[3\n\r1\x18\x1b(\x07\x08\x1b7"]),
        [
            text(b"\n\r"),
            text(b"\x18"),
            text(b"\x07\x08"),
            token(Kind::Escape, b"\x1b7"),
        ]
    );
}

#[test]
fn can_and_sub_end_control_strings_and_then_execute() {
    assert_eq!(
        tokens(&[b"\x1b]0;a\x18\x1bPqb\x1a\x1b_c\x18d"]),
        [
            token(Kind::Osc, b"\x1b]0;a\x1b\\"),
            text(b"\x18"),
            token(Kind::Dcs, b"\x1bPqb\x1b\\"),
            text(b"\x1a"),
            token(Kind::Sos, b"\x1b_c\x1b\\"),
            text(b"\x18"),
            text(b"d"),
        ]
    );
}

#[test]
fn an_escape_inside_a_control_string_ends_it_and_starts_a_sequence() {
    assert_eq!(
        tokens(&[b"\x1b]0;a\x1b[1m\x1bPq\x1b", b"7\x1b_x\x1b\x1b[2m"]),
        [
            token(Kind::Osc, b"\x1b]0;a\x1b\\"),
            token(Kind::Csi, b"\x1b[1m"),
            token(Kind::Dcs, b"\x1bPq\x1b\\"),
            token(Kind::Escape, b"\x1b7"),
            token(Kind::Sos, b"\x1b_x\x1b\\"),
            token(Kind::Csi, b"\x1b[2m"),
        ]
    );
}

#[test]
fn bel_ends_only_osc_and_st_byte_ends_only_dcs_passthrough() {
    assert_eq!(
        tokens(&[b"\x1bPq\x07\x9c\x1b_\x07\x9c\x1b\\\x1bP\x9cq\x9c"]),
        [
            token(Kind::Dcs, b"\x1bPq\x07\x9c"),
            token(Kind::Sos, b"\x1b_\x07\x9c\x1b\\"),
            token(Kind::Dcs, b"\x1bP\x9cq\x9c"),
        ]
    );
}

#[test]
fn bytes_the_parser_ignores_are_dropped_from_escape_sequences() {
    assert_eq!(
        tokens(&[b"\x1b[3\x7f1\xc3m\x1b\x1b(\x7f\xffB\x1b\x80c"]),
        [
            token(Kind::Csi, b"\x1b[31m"),
            token(Kind::Escape, b"\x1b(B"),
            token(Kind::Escape, b"\x1bc"),
        ]
    );
}

fn oversized(introducer: &[u8], body: usize, terminator: &[u8]) -> Vec<u8> {
    let mut bytes = introducer.to_vec();
    bytes.resize(introducer.len() + body, b'1');
    bytes.extend_from_slice(terminator);
    bytes
}

#[test]
fn bodies_up_to_the_cap_are_kept_and_longer_ones_are_dropped() {
    for (introducer, terminator, kind) in [
        (&b"\x1b["[..], &b"m"[..], Kind::Csi),
        (b"\x1b]", b"\x07", Kind::Osc),
        (b"\x1bP", b"\x1b\\", Kind::Dcs),
        (b"\x1b_", b"\x1b\\", Kind::Sos),
    ] {
        let kept = oversized(introducer, MAX_BODY, terminator);
        assert_eq!(tokens(&[&kept]), [(kind, kept.clone())]);
        let dropped = oversized(introducer, MAX_BODY + 1, terminator);
        assert_eq!(
            tokens(&[&dropped, b"a"]),
            [token(Kind::Oversized, b""), text(b"a")]
        );
    }
}

#[test]
fn an_oversized_string_ended_by_cancel_or_escape_is_still_dropped() {
    let cancelled = oversized(b"\x1b]", MAX_BODY + 1, b"\x18");
    assert_eq!(
        tokens(&[&cancelled]),
        [token(Kind::Oversized, b""), text(b"\x18")]
    );
    let interrupted = oversized(b"\x1bP", MAX_BODY + 5, b"\x1b7");
    assert_eq!(
        tokens(&[&interrupted]),
        [token(Kind::Oversized, b""), token(Kind::Escape, b"\x1b7")]
    );
}

#[test]
fn dcs_parameters_and_intermediates_do_not_end_the_string() {
    assert_eq!(
        tokens(&[b"\x1bP1;2$q\x9c\x1bP$1\x9cq\x1b\\\x1bP1<q\x9c\x1b\\\x1bP=q\x9c"]),
        [
            token(Kind::Dcs, b"\x1bP1;2$q\x9c"),
            token(Kind::Dcs, b"\x1bP$1\x9cq\x1b\\"),
            token(Kind::Dcs, b"\x1bP1<q\x9c\x1b\\"),
            token(Kind::Dcs, b"\x1bP=q\x9c"),
        ]
    );
}

/// Records parser actions so token streams can be compared with vte itself.
#[derive(Default)]
struct Log(Vec<String>);

impl Perform for Log {
    fn print(&mut self, c: char) {
        self.0.push(format!("print {c:?}"));
    }
    fn execute(&mut self, byte: u8) {
        self.0.push(format!("execute {byte}"));
    }
    fn hook(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        self.0.push(format!(
            "hook {params:?} {intermediates:?} {ignore} {action}"
        ));
    }
    fn put(&mut self, byte: u8) {
        self.0.push(format!("put {byte}"));
    }
    fn unhook(&mut self) {
        self.0.push("unhook".into());
    }
    fn osc_dispatch(&mut self, params: &[&[u8]], _: bool) {
        self.0.push(format!("osc {params:?}"));
    }
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, c: char) {
        self.0
            .push(format!("csi {params:?} {intermediates:?} {ignore} {c}"));
    }
    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if (intermediates, byte) != (&[][..], b'\\') {
            self.0
                .push(format!("esc {intermediates:?} {ignore} {byte}"));
        }
    }
}

fn vte_actions(reads: &[Vec<u8>]) -> Vec<String> {
    let mut parser = Parser::new();
    let mut log = Log::default();
    reads.iter().for_each(|read| parser.advance(&mut log, read));
    log.0
}

/// Deterministic inputs over the bytes that change parser state, each ending
/// with CAN so that no partial token is left pending.
fn tricky_inputs() -> Vec<Vec<u8>> {
    const ALPHABET: &[u8] =
        b"\x1b\x1b\x1b[]PX^_\\\x07\x18\x1a\x9c\x7f\n\r01;:?>$ mhqa\xc3\xa9\xe2\x82\xac\xf0\x80";
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    (0..3000)
        .map(|_| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let length = (seed % 24) as usize + 1;
            (0..length)
                .map(|index| {
                    let mixed = seed.rotate_left(index as u32 * 5) as usize;
                    ALPHABET[mixed % ALPHABET.len()]
                })
                .chain(*b"\x18Z")
                .collect()
        })
        .collect()
}

#[test]
fn feeding_tokens_drives_vte_exactly_as_the_raw_stream_does() {
    for input in tricky_inputs() {
        let expected = vte_actions(std::slice::from_ref(&input));
        for split in 0..=input.len() {
            let mut tokenizer = Tokenizer::new();
            let mut fed: Vec<Vec<u8>> = Vec::new();
            for read in [&input[..split], &input[split..]] {
                fed.extend(tokenizer.push(read).into_iter().map(|token| token.bytes));
            }
            assert_eq!(
                vte_actions(&fed),
                expected,
                "input {input:?} split {split} tokens {fed:?}"
            );
        }
    }
}

#[test]
fn c0_controls_in_aborted_escape_sequences_still_reach_the_engine() {
    let input = b"\x1b \n\x18Z".to_vec();
    let mut tokenizer = Tokenizer::new();
    let tokens = tokenizer.push(&input);
    assert_eq!(
        vte_actions(
            &tokens
                .into_iter()
                .map(|token| token.bytes)
                .collect::<Vec<_>>()
        ),
        vte_actions(std::slice::from_ref(&input))
    );
}

/// The parser actions each kind of token may cause: text is printed or
/// executed, and a sequence is dispatched as its kind, after executing the
/// C0 controls inside it. SOS, PM and APC strings, and the forms the parser
/// ignores, dispatch nothing.
fn dispatches(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Text => &["print", "execute"],
        Kind::Escape => &["execute", "esc"],
        Kind::Csi => &["execute", "csi"],
        Kind::Osc => &["execute", "osc"],
        Kind::Dcs => &["execute", "hook", "put", "unhook"],
        Kind::Sos => &["execute"],
        Kind::Oversized => &[],
    }
}

/// Plan section 1.9: the live output policy and the presentation classify
/// tokens by kind, so a token's kind must be what vte does with it. C0
/// controls vte executes between `ESC` and the introducer do not change
/// which sequence it then dispatches (TRM-02).
#[test]
fn vte_dispatches_each_token_as_its_kind() {
    for input in tricky_inputs() {
        for token in Tokenizer::new().push(&input) {
            let actions = vte_actions(std::slice::from_ref(&token.bytes));
            let allowed = dispatches(token.kind);
            assert!(
                actions
                    .iter()
                    .all(|action| allowed.contains(&action.split(' ').next().unwrap())),
                "input {input:?} token {token:?} actions {actions:?}"
            );
        }
    }
}

#[test]
fn completed_tokens_never_leave_vte_inside_a_sequence() {
    for input in tricky_inputs() {
        let mut tokenizer = Tokenizer::new();
        let mut parser = Parser::new();
        for token in tokenizer.push(&input) {
            let mut log = Log::default();
            parser.advance(&mut log, &token.bytes);
            parser.advance(&mut log, b"Z");
            assert_eq!(
                log.0.last().map(String::as_str),
                Some("print 'Z'"),
                "input {input:?} token {token:?}"
            );
        }
    }
}
