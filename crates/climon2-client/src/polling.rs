//! Nonblocking I/O helpers the bridge, the attachment and kill mode share:
//! interruption, retryable reads and writes, and pending output.

use climon2_platform::terminal::Signals;
use std::io::{self, Write};

/// HUP, TERM or INT arrived: what was waited for is abandoned.
pub fn interrupted(signals: &Signals) -> io::Result<()> {
    signals
        .termination_signal()
        .map_or(Ok(()), |_| Err(io::ErrorKind::Interrupted.into()))
}

pub fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// `None` when a nonblocking read or write must simply be retried after the next poll.
pub fn ready(result: io::Result<usize>) -> io::Result<Option<usize>> {
    match result {
        Ok(count) => Ok(Some(count)),
        Err(error) if retryable(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn retryable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}

/// Bytes waiting to be written, in order.
#[derive(Default)]
pub struct Pending {
    bytes: Vec<u8>,
    position: usize,
}

impl Pending {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, position: 0 }
    }

    pub fn empty(&self) -> bool {
        self.position == self.bytes.len()
    }

    /// How many bytes still wait.
    pub fn backlog(&self) -> usize {
        self.bytes.len() - self.position
    }

    pub fn append(&mut self, mut bytes: Vec<u8>) {
        if self.empty() {
            self.bytes = bytes;
            self.position = 0;
        } else {
            self.bytes.append(&mut bytes);
        }
    }

    /// Only called while bytes remain, after poll reported the writer ready; a
    /// terminal write of a nonempty buffer then never returns 0.
    pub fn flush(&mut self, writer: &mut impl Write) -> io::Result<()> {
        if let Some(count) = ready(writer.write(&self.bytes[self.position..]))? {
            self.position += count;
        }
        Ok(())
    }
}
