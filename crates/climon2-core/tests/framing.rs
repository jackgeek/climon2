//! #20 / ADR 0001: frozen framing through the public Read/Write seam.
use climon2_core::protocol::{Frame, read_frame, write_frame};
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

#[test]
fn issue20_frozen_frame_handles_partial_io_and_raw_bytes() {
    let frame = Frame::new(0x11, vec![0, 0xff, b'\n']).unwrap();
    let mut wire = Fragmented(Vec::new());
    write_frame(&mut wire, &frame).unwrap();
    assert_eq!(wire.0, [0, 0, 0, 4, 0x11, 0, 0xff, b'\n']);
    let mut reader = Fragmented(Cursor::new(wire.0));
    assert_eq!(read_frame(&mut reader).unwrap(), frame);
}

#[test]
fn issue20_frame_bounds_include_type_and_reject_truncation_before_allocation() {
    for (code, size) in [(0x01, 0), (0x01, 1_048_575), (0x11, 65_535), (0x21, 65_535)] {
        let frame = Frame::new(code, vec![0; size]).unwrap();
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).unwrap();
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), frame);
        bytes.pop();
        assert!(read_frame(&mut bytes.as_slice()).is_err());
    }
    for (code, size) in [(1, 1_048_576), (0x11, 65_536), (0x21, 65_536)] {
        assert!(Frame::new(code, vec![0; size]).is_err());
    }
    for length in [0_u32, 1_048_577, u32::MAX] {
        let error = read_frame(&mut length.to_be_bytes().as_slice()).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
    for code in [0, 2, 0x13, 0x23, 0x7e, 0x80, 0xff] {
        assert!(Frame::new(code, vec![]).is_err());
        assert_eq!(
            read_frame(&mut [0, 0, 0, 1, code].as_slice())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
    for bytes in [&[][..], &[0, 0, 0][..], &[0, 0, 0, 1][..]] {
        assert_eq!(
            read_frame(&mut &*bytes).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
