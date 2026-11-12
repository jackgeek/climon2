//! Preserve macOS's existing runtime timestamp behavior.
use std::{io, path::Path, time::Instant};

pub struct SocketFreshness;

impl SocketFreshness {
    pub fn new(_socket: &Path, _now: Instant) -> Self {
        Self
    }

    pub fn refresh(&mut self, _now: Instant) -> io::Result<()> {
        Ok(())
    }
}
