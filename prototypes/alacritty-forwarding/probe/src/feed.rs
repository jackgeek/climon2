//! THROWAWAY #48: bounds uninterruptible engine work per pump iteration.
//!
//! `Processor::advance` and `stop_sync` run to completion, and a single byte
//! can expand into up to 65,535 handler inputs (REP). The adapter therefore
//! feeds at most `slice_bytes` per step and ends a synchronized update itself
//! once more than `sync_hold_bytes` are buffered, so one step processes at most
//! `slice_bytes + sync_hold_bytes` bytes instead of vte's fixed 2 MiB.

use std::time::Instant;

use vte::ansi::{Handler, Processor};

use crate::timing::{FirstBeginTimeout, flush_if_due};

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Idle,
    DeadlineFlushed,
    Advanced { bytes: usize, hold_breached: bool },
}

pub struct EngineFeed {
    slice_bytes: usize,
    sync_hold_bytes: usize,
    pending: Vec<u8>,
    offset: usize,
    breaches: u64,
}

impl EngineFeed {
    pub fn new(slice_bytes: usize, sync_hold_bytes: usize) -> Self {
        assert!(slice_bytes > 0);
        Self {
            slice_bytes,
            sync_hold_bytes,
            pending: Vec::new(),
            offset: 0,
            breaches: 0,
        }
    }

    /// The pump should push one PTY read only when `pending()` is zero, so
    /// engine cost back-pressures the program through the PTY.
    pub fn push(&mut self, bytes: &[u8]) {
        self.pending.drain(..self.offset);
        self.offset = 0;
        self.pending.extend_from_slice(bytes);
    }

    pub fn pending(&self) -> usize {
        self.pending.len() - self.offset
    }

    pub fn sync_hold_breaches(&self) -> u64 {
        self.breaches
    }

    pub fn step<H: Handler>(
        &mut self,
        parser: &mut Processor<FirstBeginTimeout>,
        handler: &mut H,
        now: Instant,
    ) -> Step {
        if flush_if_due(parser, handler, now) {
            return Step::DeadlineFlushed;
        }
        let end = (self.offset + self.slice_bytes).min(self.pending.len());
        if end == self.offset {
            return Step::Idle;
        }
        parser.advance(handler, &self.pending[self.offset..end]);
        let bytes = end - self.offset;
        self.offset = end;
        let hold_breached = parser.sync_bytes_count() > self.sync_hold_bytes;
        if hold_breached {
            self.breaches += 1;
            parser.stop_sync(handler);
        }
        Step::Advanced {
            bytes,
            hold_breached,
        }
    }

    /// Drain everything, ending any synchronized update (e.g. at shutdown).
    pub fn flush<H: Handler>(
        &mut self,
        parser: &mut Processor<FirstBeginTimeout>,
        handler: &mut H,
    ) {
        parser.advance(handler, &self.pending[self.offset..]);
        self.pending.clear();
        self.offset = 0;
        if parser.sync_timeout().deadline().is_some() {
            parser.stop_sync(handler);
        }
    }
}
