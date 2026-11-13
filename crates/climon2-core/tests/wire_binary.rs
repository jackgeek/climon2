//! Daemon wire revision 1 binary records: Input `0x31` (client to daemon) and
//! Display `0x49` (daemon to client). A header of nonzero big-endian u64
//! counters comes first; the payload after it is exact bytes, never text.
//! Each test is the codec case of the matrix scenarios it cites.
use climon2_core::wire::{
    Display, FatalCode, Frame, Input, Request, ServerMessage, read_frame, write_frame,
};
use support::{Fragmented, counter, ids, limit};

mod support;

const INPUT: u8 = 0x31;
const DISPLAY: u8 = 0x49;

fn input(body: &[u8]) -> Result<Request, FatalCode> {
    Request::from_frame(&Frame::new(INPUT, body.to_vec()))
}

fn display(body: &[u8]) -> Result<ServerMessage, FatalCode> {
    ServerMessage::from_frame(&Frame::new(DISPLAY, body.to_vec()))
}

/// Big-endian header words, then the payload.
fn body(header: &[u64], payload: &[u8]) -> Vec<u8> {
    header
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .chain(payload.iter().copied())
        .collect()
}

fn input_record(header: [u64; 3], payload: &[u8]) -> Input {
    Input {
        request_id: counter(header[0]),
        authority: ids(header[1], header[2]),
        payload: payload.to_vec(),
    }
}

fn display_record(header: [u64; 2], payload: &[u8]) -> Display {
    Display {
        authority: ids(header[0], header[1]),
        payload: payload.to_vec(),
    }
}

/// COM-04 (codec case): Input and Display carry any bytes unchanged: the
/// published `00 ff 41`, a lone NUL, NUL inside and at both ends, invalid
/// UTF-8 (a lone continuation byte, an overlong form, an encoded surrogate,
/// a cut sequence and a code point past U+10FFFF) and all 256 byte values.
/// Nothing stops at a NUL or is checked as UTF-8, and each record re-encodes
/// to the frame it came from.
#[test]
fn input_and_display_carry_arbitrary_bytes_unchanged() {
    let every_byte: Vec<u8> = (0..=255).collect();
    let payloads: [&[u8]; 10] = [
        b"\x00\xff\x41",
        b"\x00",
        b"a\x00b",
        b"\x00a\x00",
        b"\x80",
        b"\xc0\xaf",
        b"\xed\xa0\x80",
        b"\xe2\x82",
        b"\xf4\x90\x80\x80",
        &every_byte,
    ];
    for payload in payloads {
        let frame = Frame::new(INPUT, body(&[2, 1, 1], payload));
        let expected = input_record([2, 1, 1], payload);
        assert_eq!(
            Request::from_frame(&frame),
            Ok(Request::Input(expected.clone()))
        );
        assert_eq!(expected.to_frame(), frame);

        let frame = Frame::new(DISPLAY, body(&[2, 3], payload));
        let expected = display_record([2, 3], payload);
        assert_eq!(
            ServerMessage::from_frame(&frame),
            Ok(ServerMessage::Display(expected.clone()))
        );
        assert_eq!(expected.to_frame(), frame);
    }
}

/// COM-04 (codec case): fragmented payloads stay exact. Frames read one byte
/// at a time decode to the payloads written, and a payload chunked across two
/// Display frames keeps its byte order.
#[test]
fn binary_payloads_survive_fragmented_reads_and_chunking() {
    let ceiling = limit(1_048_576);
    let payload: Vec<u8> = (0..=255).rev().chain(0..=255).collect();
    let (first, second) = payload.split_at(300);
    let records = [
        input_record([7, 2, 3], &payload).to_frame(),
        display_record([2, 3], first).to_frame(),
        display_record([2, 3], second).to_frame(),
    ];
    let mut stream = Vec::new();
    for frame in &records {
        write_frame(&mut stream, frame, ceiling).unwrap();
    }

    let mut reader = Fragmented(stream.as_slice());
    let mut next = || read_frame(&mut reader, ceiling).unwrap().unwrap();
    assert_eq!(
        Request::from_frame(&next()),
        Ok(Request::Input(input_record([7, 2, 3], &payload)))
    );
    let mut displayed = Vec::new();
    for _ in 0..2 {
        let Ok(ServerMessage::Display(chunk)) = ServerMessage::from_frame(&next()) else {
            panic!("a Display chunk");
        };
        displayed.extend(chunk.payload);
    }
    assert_eq!(displayed, payload);
    assert!(read_frame(&mut reader, ceiling).unwrap().is_none());
}

/// COM-02 (codec case): a body shorter than its header is `invalid_record`,
/// whatever bytes it holds: Input's header is 24 bytes and Display's 16, and
/// the length is checked before any counter, so a short Input whose request
/// ID bytes are zero is not `invalid_request_id`. A body of exactly the
/// header is an identifiable record with an empty payload, which only the
/// connection owner refuses (the session daemon rejects empty Input with an
/// operational error), and one byte more is a one-byte payload.
#[test]
fn binary_headers_are_complete_or_invalid_records() {
    for header in [body(&[2, 1, 1], &[]), body(&[0, 0, 0], &[])] {
        for length in [0, 8, 16, 23] {
            let short = &header[..length];
            assert_eq!(input(short), Err(FatalCode::InvalidRecord), "{short:?}");
        }
    }
    let header = body(&[2, 1, 1], &[]);
    assert_eq!(
        input(&header),
        Ok(Request::Input(input_record([2, 1, 1], &[])))
    );
    assert_eq!(
        input(&body(&[2, 1, 1], b"A")),
        Ok(Request::Input(input_record([2, 1, 1], b"A")))
    );

    let header = body(&[2, 3], &[]);
    for length in [0, 8, 15] {
        assert_eq!(
            display(&header[..length]),
            Err(FatalCode::InvalidRecord),
            "{length}"
        );
    }
    assert_eq!(
        display(&header),
        Ok(ServerMessage::Display(display_record([2, 3], &[])))
    );
    assert_eq!(
        display(&body(&[2, 3], b"A")),
        Ok(ServerMessage::Display(display_record([2, 3], b"A")))
    );
}

/// COM-02 (codec case): header counters are nonzero and span the whole u64
/// range: 1 and 18446744073709551615 decode and re-encode. A zero request ID
/// is `invalid_request_id`, because the message type makes Input's first
/// counter its request ID, as for JSON requests, and it outranks a zero
/// period. A zero period or restoration ID is `invalid_record`.
#[test]
fn binary_header_counters_are_nonzero_u64s() {
    for word in [1, u64::MAX] {
        let expected = input_record([word; 3], b"x");
        assert_eq!(
            input(&body(&[word; 3], b"x")),
            Ok(Request::Input(expected.clone()))
        );
        assert_eq!(expected.to_frame().body(), body(&[word; 3], b"x"));

        let expected = display_record([word; 2], b"x");
        assert_eq!(
            display(&body(&[word; 2], b"x")),
            Ok(ServerMessage::Display(expected.clone()))
        );
        assert_eq!(expected.to_frame().body(), body(&[word; 2], b"x"));
    }
    for (header, code) in [
        ([0, 1, 1], FatalCode::InvalidRequestId),
        ([0, 0, 0], FatalCode::InvalidRequestId),
        ([1, 0, 1], FatalCode::InvalidRecord),
        ([1, 1, 0], FatalCode::InvalidRecord),
    ] {
        assert_eq!(input(&body(&header, b"x")), Err(code), "{header:?}");
    }
    for header in [[0, 1], [1, 0], [0, 0]] {
        assert_eq!(
            display(&body(&header, b"x")),
            Err(FatalCode::InvalidRecord),
            "{header:?}"
        );
    }
}
