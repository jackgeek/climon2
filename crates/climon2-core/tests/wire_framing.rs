//! Daemon wire revision 1 framing through the public Read/Write seam:
//! `u32be length | u8 type | body`, bounded by a local frame ceiling.
use climon2_core::wire::{FatalCode, Frame, FrameLimit, WireError, read_frame, write_frame};
use std::io::{self, Cursor, Read, Write};

struct Fragmented<T>(T);

impl<T: Read> Read for Fragmented<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let length = buf.len().min(2);
        self.0.read(&mut buf[..length])
    }
}

impl<T: Write> Write for Fragmented<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(&buf[..buf.len().min(2)])
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

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

fn limit(bytes: u32) -> FrameLimit {
    FrameLimit::new(bytes).unwrap()
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
