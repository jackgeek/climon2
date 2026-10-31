use alacritty_embedding_probe::input::{Bindings, Codec, Effect, Error, MAX_FEED, State};

#[test]
fn detach_is_consumed_after_preceding_input_and_stops_the_current_burst() {
    let mut codec = Codec::new(Bindings::default(), State::Ready);
    assert_eq!(
        codec.feed(b"before\x1ddafter"),
        Ok(vec![
            Effect::Application(b"before".to_vec()),
            Effect::Detach
        ])
    );
    assert_eq!(codec.feed(b"later"), Err(Error::Detached));
}

fn application_bytes(effects: impl IntoIterator<Item = Effect>) -> Vec<u8> {
    let mut output = Vec::new();
    for effect in effects {
        let Effect::Application(bytes) = effect else {
            panic!("unexpected client action: {effect:?}");
        };
        output.extend(bytes);
    }
    output
}

#[test]
fn recognised_paste_keeps_markers_and_never_runs_client_actions_across_any_split() {
    let paste = b"\x1b[200~a\x1dd\x14\x1b[200~z\x1b[201~";
    for split in 0..=paste.len() {
        let mut codec = Codec::new(Bindings::default(), State::Ready);
        let mut effects = codec.feed(&paste[..split]).unwrap();
        effects.extend(codec.feed(&paste[split..]).unwrap());
        assert_eq!(application_bytes(effects), paste, "split {split}");
    }
}

#[test]
fn interrupted_paste_requests_cleanup_once_and_discards_the_stale_tail_after_readiness() {
    let mut codec = Codec::new(Bindings::default(), State::Ready);
    assert_eq!(
        application_bytes(codec.feed(b"\x1b[200~old").unwrap()),
        b"\x1b[200~old"
    );
    assert_eq!(
        codec.transition(State::Viewing),
        Ok(vec![Effect::FinishPaste])
    );
    assert_eq!(codec.feed(b"stale\x1dd\x14\x1b[20"), Ok(vec![]));
    assert_eq!(codec.transition(State::Restoring), Ok(vec![]));
    assert_eq!(codec.transition(State::Ready), Ok(vec![]));
    assert_eq!(
        codec.feed(b"1~fresh"),
        Ok(vec![Effect::Application(b"fresh".to_vec())])
    );
}

#[test]
fn escape_prefix_doubling_sends_a_literal_immediately_without_a_timer() {
    let bindings = Bindings::new(Some(0x1b), Some(0x14)).unwrap();
    let mut codec = Codec::new(bindings, State::Ready);
    assert_eq!(codec.feed(b"\x1b"), Ok(vec![]));
    assert_eq!(
        codec.feed(b"\x1b"),
        Ok(vec![Effect::Application(b"\x1b".to_vec())])
    );
    assert_eq!(
        codec.feed(b"d"),
        Ok(vec![Effect::Application(b"d".to_vec())])
    );
}

#[test]
fn pending_marker_bytes_cannot_make_a_forwarded_frame_exceed_the_input_bound() {
    let mut codec = Codec::new(Bindings::default(), State::Ready);
    assert_eq!(codec.feed(b"\x1b[20"), Ok(vec![]));
    let chunk = vec![b'x'; MAX_FEED];
    let effects = codec.feed(&chunk).unwrap();
    for effect in &effects {
        let Effect::Application(bytes) = effect else {
            panic!("unexpected client action");
        };
        assert!(bytes.len() <= MAX_FEED);
    }
    let mut expected = b"\x1b[20".to_vec();
    expected.extend(chunk);
    assert_eq!(application_bytes(effects), expected);
}

#[test]
fn nested_prefix_unknown_suffix_and_reply_shaped_keys_are_preserved_at_every_split() {
    let input = b"\x1d\x1dd\x1dx\x03\x04\x1b[1;5R";
    let expected = b"\x1dd\x1dx\x03\x04\x1b[1;5R";
    for split in 0..=input.len() {
        let mut codec = Codec::new(Bindings::default(), State::Ready);
        let mut effects = codec.feed(&input[..split]).unwrap();
        effects.extend(codec.feed(&input[split..]).unwrap());
        assert_eq!(application_bytes(effects), expected);
    }
}

#[test]
fn all_control_prefixes_double_literally_and_paste_shields_custom_bindings() {
    let paste = b"\x1b[200~\0\x01\x02\x03\x04\x1bd\x1dd\x14\x1b[201~";
    for prefix in 0..=31 {
        let take = (prefix + 1) % 32;
        let bindings = Bindings::new(Some(prefix), Some(take)).unwrap();
        let mut codec = Codec::new(bindings, State::Ready);
        assert_eq!(codec.feed(&[prefix]), Ok(vec![]));
        assert_eq!(
            codec.feed(&[prefix]),
            Ok(vec![Effect::Application(vec![prefix])])
        );
        let mut forwarded = Vec::new();
        for &byte in paste {
            forwarded.extend(application_bytes(codec.feed(&[byte]).unwrap()));
        }
        assert_eq!(forwarded, paste, "prefix {prefix}");
    }
}

#[test]
fn binding_validation_is_explicit_and_disabling_actions_preserves_binary_input() {
    assert!(matches!(
        Bindings::new(Some(0x1d), Some(0x1d)),
        Err(Error::ConflictingBindings)
    ));
    assert!(matches!(
        Bindings::new(Some(b'x'), None),
        Err(Error::InvalidBinding)
    ));
    assert!(matches!(
        Bindings::new(None, Some(127)),
        Err(Error::InvalidBinding)
    ));
    let mut codec = Codec::new(Bindings::new(None, None).unwrap(), State::Ready);
    let binary: Vec<_> = (0..=255).collect();
    assert_eq!(application_bytes(codec.feed(&binary).unwrap()), binary);
}

#[test]
fn takeover_only_works_while_viewing_and_its_burst_never_replays_after_readiness() {
    let mut codec = Codec::new(Bindings::default(), State::Viewing);
    assert_eq!(
        codec.feed(b"ignored\x14stale"),
        Ok(vec![Effect::TakeControl])
    );
    assert_eq!(codec.transition(State::Restoring), Ok(vec![]));
    assert_eq!(codec.feed(b"not-ready\x14"), Ok(vec![]));
    assert_eq!(codec.transition(State::Ready), Ok(vec![]));
    assert_eq!(
        codec.feed(b"fresh\x14"),
        Ok(vec![Effect::Application(b"fresh\x14".to_vec())])
    );
    assert_eq!(codec.transition(State::Restoring), Ok(vec![]));
    assert_eq!(codec.feed(b"\x1dd"), Ok(vec![Effect::Detach]));
    assert_eq!(codec.transition(State::Ready), Err(Error::Detached));
}

#[test]
fn partial_paste_headers_from_an_old_admission_interval_cannot_activate_new_input() {
    for split in 1..6 {
        let mut codec = Codec::new(Bindings::default(), State::Viewing);
        assert_eq!(codec.feed(&b"\x1b[200~"[..split]), Ok(vec![]));
        assert_eq!(codec.transition(State::Ready), Ok(vec![]));
        let mut suffix = b"\x1b[200~"[split..].to_vec();
        suffix.extend_from_slice(b"old\x1dd\x14\x1b[201~new");
        assert_eq!(
            codec.feed(&suffix),
            Ok(vec![Effect::Application(b"new".to_vec())])
        );
    }
    let mut codec = Codec::new(Bindings::default(), State::Ready);
    assert_eq!(codec.feed(b"\x1d\x1b[20"), Ok(vec![]));
    assert_eq!(codec.transition(State::Ready), Ok(vec![]));
    assert_eq!(
        codec.feed(b"xd"),
        Ok(vec![Effect::Application(b"xd".to_vec())])
    );
}

#[test]
fn oversized_feeds_do_not_consume_pending_actions_and_long_pastes_keep_bounded_state() {
    let mut codec = Codec::new(Bindings::default(), State::Ready);
    assert_eq!(codec.feed(b"\x1d"), Ok(vec![]));
    assert_eq!(
        codec.feed(&vec![b'd'; MAX_FEED + 1]),
        Err(Error::InputTooLarge)
    );
    assert_eq!(
        codec.feed(b"x"),
        Ok(vec![Effect::Application(b"\x1dx".to_vec())])
    );
    let mut expected = b"\x1b[200~".to_vec();
    let mut actual = application_bytes(codec.feed(b"\x1b[200~").unwrap());
    let mut chunk = vec![b'a'; 4092];
    chunk.extend_from_slice(b"\x1b[20");
    for _ in 0..32 {
        expected.extend_from_slice(&chunk);
        actual.extend(application_bytes(codec.feed(&chunk).unwrap()));
        assert!(codec.buffered_bytes() <= 6);
    }
    expected.extend_from_slice(b"\x1b[201~");
    actual.extend(application_bytes(codec.feed(b"\x1b[201~").unwrap()));
    assert_eq!(actual, expected);
}

#[test]
fn escape_takeover_remains_ambiguous_with_the_first_byte_of_a_paste_header() {
    let bindings = Bindings::new(Some(0x1d), Some(0x1b)).unwrap();
    let mut codec = Codec::new(bindings, State::Viewing);
    assert_eq!(codec.feed(b"\x1b"), Ok(vec![]));
    assert_eq!(codec.buffered_bytes(), 1);
    assert_eq!(codec.feed(b"[200~\x1dd\x1b[201~"), Ok(vec![]));
    // A standalone Escape cannot trigger without a later byte or an unapproved timer.
    let mut standalone = Codec::new(bindings, State::Viewing);
    assert_eq!(standalone.feed(b"\x1b"), Ok(vec![]));
    assert_eq!(standalone.feed(b"x"), Ok(vec![Effect::TakeControl]));
}
