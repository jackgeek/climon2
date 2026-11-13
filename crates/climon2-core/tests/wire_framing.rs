//! Daemon wire revision 1 framing through the public Read/Write seam:
//! `u32be length | u8 type | body`, bounded by a local frame ceiling.
use climon2_core::wire::{
    FatalCode, Frame, FrameLimit, WireError, defines, read_frame, write_frame,
};
use std::io::{self, Cursor, Read, Write};
use support::{Fragmented, limit};

mod support;

struct Broken;

impl Read for Broken {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::ConnectionReset.into())
    }
}

impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::ErrorKind::BrokenPipe.into())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn fatal(result: Result<Option<Frame>, WireError>) -> FatalCode {
    match result {
        Err(WireError::Fatal(code)) => code,
        other => panic!("expected a connection-fatal error, got {other:?}"),
    }
}

/// COM-02 (codec case): partial reads and writes do not define messages; the
/// length counts the type byte and body but not the four-byte prefix.
#[test]
fn revision1_frames_survive_partial_io_and_end_cleanly_at_a_frame_boundary() {
    let frame = Frame::new(0x37, br#"{"request_id":"8"}"#.to_vec());
    let mut wire = Fragmented(Vec::new());
    write_frame(&mut wire, &frame, limit(19)).unwrap();
    let mut expected = vec![0, 0, 0, 19, 0x37];
    expected.extend_from_slice(br#"{"request_id":"8"}"#);
    assert_eq!(wire.0, expected);

    let mut reader = Fragmented(Cursor::new(wire.0));
    let read = read_frame(&mut reader, limit(19)).unwrap().unwrap();
    assert_eq!(
        (read.code(), read.body()),
        (0x37, &br#"{"request_id":"8"}"#[..])
    );
    assert_eq!(read, frame);
    assert!(read_frame(&mut reader, limit(19)).unwrap().is_none());
}

/// COM-02 / LIM-01 (codec case): a frame exactly at the local ceiling is
/// accepted; one byte more is `frame_too_large`, decided from the prefix before
/// any body arrives.
#[test]
fn the_local_frame_ceiling_admits_its_limit_and_rejects_one_byte_more() {
    let at_limit = Frame::new(0x40, vec![b'x'; 15]);
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &at_limit, limit(16)).unwrap();
    assert_eq!(bytes[..4], [0, 0, 0, 16]);
    assert_eq!(
        read_frame(&mut bytes.as_slice(), limit(16)).unwrap(),
        Some(at_limit)
    );

    let mut header_only = [0, 0, 0, 17, 0x40].as_slice();
    assert_eq!(
        fatal(read_frame(&mut header_only, limit(16))),
        FatalCode::FrameTooLarge
    );
    let huge = u32::MAX.to_be_bytes();
    assert_eq!(
        fatal(read_frame(&mut huge.as_slice(), limit(1_048_576))),
        FatalCode::FrameTooLarge
    );

    let over = Frame::new(0x40, vec![b'x'; 16]);
    let mut unwritten = Vec::new();
    let error = write_frame(&mut unwritten, &over, limit(16)).unwrap_err();
    assert!(matches!(error, WireError::Fatal(FatalCode::FrameTooLarge)));
    assert!(
        unwritten.is_empty(),
        "an oversized frame is never partly sent"
    );
    write_frame(&mut unwritten, &over, limit(17)).unwrap();
    assert_eq!(unwritten.len(), 4 + 17);
}

/// COM-02 / LIM-01 (codec case): a nonblocking reader learns a frame's
/// length from its four-byte prefix alone, under the same rules as
/// `read_frame`: the ceiling is admitted, one byte more is `frame_too_large`
/// and zero is `invalid_frame_length`.
#[test]
fn a_prefix_alone_decides_the_admitted_frame_length() {
    assert_eq!(limit(16).frame_length([0, 0, 0, 16]), Ok(16));
    assert_eq!(limit(16).frame_length([0, 0, 0, 1]), Ok(1));
    assert_eq!(
        limit(16).frame_length([0, 0, 0, 17]),
        Err(FatalCode::FrameTooLarge)
    );
    assert_eq!(
        limit(1_048_576).frame_length([0, 0x10, 0, 0]),
        Ok(1_048_576)
    );
    assert_eq!(
        limit(1_048_576).frame_length([0, 0x10, 0, 1]),
        Err(FatalCode::FrameTooLarge)
    );
    assert_eq!(
        limit(16).frame_length([0, 0, 0, 0]),
        Err(FatalCode::InvalidFrameLength)
    );
}

/// COM-02 (codec case): revision 1 defines exactly the request types
/// `0x30`–`0x39` and the server types `0x40`–`0x4d`. A session daemon that
/// still speaks another post-hello protocol selects revision 1 by them.
#[test]
fn revision1_defines_the_request_and_server_types_only() {
    for code in [0x30, 0x37, 0x39, 0x40, 0x4c, 0x4d] {
        assert!(defines(code), "{code:#x}");
    }
    for code in [0x00, 0x01, 0x10, 0x16, 0x2f, 0x3a, 0x3f, 0x4e, 0x7f, 0xff] {
        assert!(!defines(code), "{code:#x}");
    }
}

/// COM-02 (codec case): length zero cannot hold the type byte; length one is a
/// type-only frame.
#[test]
fn zero_length_is_invalid_and_a_type_only_frame_has_an_empty_body() {
    assert_eq!(
        fatal(read_frame(&mut [0, 0, 0, 0, 0x37].as_slice(), limit(16))),
        FatalCode::InvalidFrameLength
    );
    let frame = read_frame(&mut [0, 0, 0, 1, 0x4a].as_slice(), limit(1))
        .unwrap()
        .unwrap();
    assert_eq!((frame.code(), frame.body()), (0x4a, &[][..]));
    assert_eq!(FrameLimit::new(0), None);
    assert_eq!(limit(1).get(), 1);
    assert_eq!(limit(u32::MAX).get(), u32::MAX);
}

/// COM-02 (codec case): a stream ending inside a prefix, type or body is
/// `truncated_frame`; ending exactly between frames is closure, not an error or
/// a final result.
#[test]
fn a_stream_ending_inside_a_frame_is_truncated() {
    assert!(read_frame(&mut [].as_slice(), limit(16)).unwrap().is_none());
    for bytes in [
        &[0][..],
        &[0, 0, 0],
        &[0, 0, 0, 2],
        &[0, 0, 0, 3, 0x37],
        &[0, 0, 0, 3, 0x37, b'{'],
    ] {
        assert_eq!(
            fatal(read_frame(&mut &*bytes, limit(16))),
            FatalCode::TruncatedFrame,
            "{bytes:?}"
        );
    }
}

/// COM-02 (codec case): transport failures stay I/O errors; they are never wire
/// validation codes.
#[test]
fn transport_failures_are_reported_as_io_errors() {
    match read_frame(&mut Broken, limit(16)) {
        Err(WireError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::ConnectionReset),
        other => panic!("expected an I/O error, got {other:?}"),
    }
    let frame = Frame::new(0x37, b"{}".to_vec());
    match write_frame(&mut Broken, &frame, limit(16)) {
        Err(WireError::Io(error)) => assert_eq!(error.kind(), io::ErrorKind::BrokenPipe),
        other => panic!("expected an I/O error, got {other:?}"),
    }
}
