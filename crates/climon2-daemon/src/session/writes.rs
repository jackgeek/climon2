use super::pending::PendingBytes;
use std::{collections::VecDeque, io};

/// The one ordered queue of everything owed to the PTY (plan section 3.3,
/// loop steps 3 and 9): terminal replies and paste cleanup in the order the
/// daemon produced them, ahead of admitted input. A write in progress always
/// completes before anything else starts, so no two sequences interleave
/// across partial writes. Only input not yet started can be discarded, and
/// the unwritten rest of a started input when its owner loses admission.
#[derive(Debug, Default)]
pub(super) struct PtyWrites {
    writing: Option<Unit>,
    daemon: Vec<u8>,
    input: VecDeque<(u64, Vec<u8>)>,
}

/// One run of bytes written whole: daemon bytes have no owner.
#[derive(Debug)]
struct Unit {
    owner: Option<u64>,
    bytes: PendingBytes,
}

impl Unit {
    fn new(owner: Option<u64>, bytes: &[u8]) -> Self {
        let mut pending = PendingBytes::default();
        pending.extend(bytes);
        Self {
            owner,
            bytes: pending,
        }
    }
}

impl PtyWrites {
    /// Terminal replies and paste cleanup: never discarded.
    pub fn daemon(&mut self, bytes: &[u8]) {
        self.daemon.extend_from_slice(bytes);
    }

    /// One admitted input frame from `owner`.
    pub fn input(&mut self, owner: u64, bytes: &[u8]) {
        self.input.push_back((owner, bytes.to_vec()));
    }

    /// Drops every unwritten input byte of `owner`.
    pub fn discard(&mut self, owner: u64) {
        self.input.retain(|(id, _)| *id != owner);
        if self
            .writing
            .as_ref()
            .is_some_and(|unit| unit.owner == Some(owner))
        {
            self.writing = None;
        }
    }

    /// Offers bytes in order until `write` takes less than offered; returns
    /// the input bytes written, for the paste tracker.
    pub fn write_with(
        &mut self,
        mut write: impl FnMut(&[u8]) -> io::Result<usize>,
    ) -> io::Result<Vec<u8>> {
        let mut input = Vec::new();
        while let Some(unit) = self.next() {
            let written = unit.bytes.write_with(&mut write)?;
            if unit.owner.is_some() {
                input.extend(written);
            }
            if !unit.bytes.is_empty() {
                break;
            }
            self.writing = None;
        }
        Ok(input)
    }

    fn next(&mut self) -> Option<&mut Unit> {
        if self.writing.is_none() {
            self.writing = self.take();
        }
        self.writing.as_mut()
    }

    fn take(&mut self) -> Option<Unit> {
        if !self.daemon.is_empty() {
            return Some(Unit::new(None, &std::mem::take(&mut self.daemon)));
        }
        let (owner, bytes) = self.input.pop_front()?;
        Some(Unit::new(Some(owner), &bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::PtyWrites;
    use std::io;

    /// A PTY taking at most `limit` bytes per write, recording what it took.
    fn pty(written: &mut Vec<u8>, limit: usize) -> impl FnMut(&[u8]) -> io::Result<usize> + '_ {
        move |bytes| {
            let count = bytes.len().min(limit);
            written.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
    }

    #[test]
    fn a_started_input_completes_before_a_later_reply() {
        let mut writes = PtyWrites::default();
        let mut pty_bytes = Vec::new();
        writes.input(1, b"abc");
        assert_eq!(writes.write_with(pty(&mut pty_bytes, 1)).unwrap(), b"a");
        writes.daemon(b"R");
        writes.input(1, b"d");
        assert_eq!(writes.write_with(pty(&mut pty_bytes, 9)).unwrap(), b"bcd");
        assert_eq!(pty_bytes, b"abcRd");
    }

    #[test]
    fn a_started_reply_completes_before_cleanup_and_queued_input() {
        let mut writes = PtyWrites::default();
        let mut pty_bytes = Vec::new();
        writes.input(1, b"in");
        writes.daemon(b"reply");
        assert!(
            writes
                .write_with(pty(&mut pty_bytes, 2))
                .unwrap()
                .is_empty()
        );
        writes.daemon(b"~");
        assert_eq!(writes.write_with(pty(&mut pty_bytes, 3)).unwrap(), b"in");
        assert_eq!(pty_bytes, b"reply~in");
    }

    #[test]
    fn discarding_drops_only_that_owners_unwritten_input() {
        let mut writes = PtyWrites::default();
        let mut pty_bytes = Vec::new();
        writes.input(1, b"abc");
        writes.write_with(pty(&mut pty_bytes, 1)).unwrap();
        writes.input(2, b"x");
        writes.input(1, b"def");
        writes.daemon(b"R");
        writes.discard(1);
        assert_eq!(writes.write_with(pty(&mut pty_bytes, 9)).unwrap(), b"x");
        assert_eq!(pty_bytes, b"aRx");
    }

    #[test]
    fn discarding_another_owner_keeps_the_input_in_progress() {
        let mut writes = PtyWrites::default();
        let mut pty_bytes = Vec::new();
        writes.input(1, b"ab");
        writes.write_with(pty(&mut pty_bytes, 1)).unwrap();
        writes.discard(2);
        assert_eq!(writes.write_with(pty(&mut pty_bytes, 9)).unwrap(), b"b");
        assert_eq!(pty_bytes, b"ab");
    }

    #[test]
    fn a_blocked_or_failed_pty_keeps_every_byte_in_order() {
        let mut writes = PtyWrites::default();
        writes.daemon(b"R");
        writes.input(1, b"in");
        let blocked = writes.write_with(|_| Err(io::ErrorKind::WouldBlock.into()));
        assert!(blocked.unwrap().is_empty());
        assert!(
            writes
                .write_with(|_| Err(io::Error::other("gone")))
                .is_err()
        );
        let mut pty_bytes = Vec::new();
        assert_eq!(writes.write_with(pty(&mut pty_bytes, 9)).unwrap(), b"in");
        assert_eq!(pty_bytes, b"Rin");
        assert!(
            writes
                .write_with(|_| panic!("nothing owed"))
                .unwrap()
                .is_empty()
        );
    }
}
