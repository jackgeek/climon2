use std::io;

/// Bytes owed to the PTY, written strictly in order across partial writes.
#[derive(Debug, Default)]
pub(super) struct PendingBytes {
    bytes: Vec<u8>,
    offset: usize,
}

impl PendingBytes {
    pub fn extend(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    pub fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Offers the unwritten bytes to `write` once; returns the bytes it took.
    /// `WouldBlock` is no progress, never a failure.
    pub fn write_with(
        &mut self,
        write: impl FnOnce(&[u8]) -> io::Result<usize>,
    ) -> io::Result<Vec<u8>> {
        if self.is_empty() {
            return Ok(Vec::new());
        }
        let start = self.offset;
        let count = crate::unless_would_block(write(&self.bytes[start..]))?.unwrap_or(0);
        self.offset += count;
        let written = self.bytes[start..self.offset].to_vec();
        if self.is_empty() {
            self.clear();
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::PendingBytes;
    use std::io;

    fn pending(bytes: &[u8]) -> PendingBytes {
        let mut pending = PendingBytes::default();
        pending.extend(bytes);
        pending
    }

    #[test]
    fn partial_writes_resume_where_the_previous_write_stopped() {
        let mut pending = pending(b"abcdef");
        assert_eq!(
            pending.write_with(|bytes| Ok(bytes.len().min(2))).unwrap(),
            b"ab"
        );
        assert!(!pending.is_empty());
        pending.extend(b"gh");
        let written = pending.write_with(|bytes| {
            assert_eq!(bytes, b"cdefgh");
            Ok(bytes.len())
        });
        assert_eq!(written.unwrap(), b"cdefgh");
        assert!(pending.is_empty());
        pending.extend(b"x");
        assert_eq!(pending.write_with(|bytes| Ok(bytes.len())).unwrap(), b"x");
    }

    #[test]
    fn nothing_pending_never_calls_the_writer() {
        let mut pending = PendingBytes::default();
        let written = pending.write_with(|_| panic!("no write without pending bytes"));
        assert!(written.unwrap().is_empty());
    }

    #[test]
    fn would_block_is_no_progress_and_other_errors_fail() {
        let mut pending = pending(b"ab");
        let blocked = pending.write_with(|_| Err(io::ErrorKind::WouldBlock.into()));
        assert!(blocked.unwrap().is_empty());
        assert!(
            pending
                .write_with(|_| Err(io::Error::other("gone")))
                .is_err()
        );
        assert_eq!(pending.write_with(|bytes| Ok(bytes.len())).unwrap(), b"ab");
    }

    #[test]
    fn clear_drops_unwritten_bytes() {
        let mut pending = pending(b"abc");
        pending.write_with(|_| Ok(1)).unwrap();
        pending.clear();
        assert!(pending.is_empty());
        pending.extend(b"z");
        assert_eq!(pending.write_with(|bytes| Ok(bytes.len())).unwrap(), b"z");
    }
}
