use climon2_core::{
    protocol::{PTY_DRAIN_CAP, PTY_DRAIN_IDLE},
    wire::CommandResult,
};
use std::{io, time::Instant};

/// One PTY output read, classified.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Read {
    Bytes(usize),
    /// End of output: zero bytes or `EIO` once every slave handle closed.
    End,
    Idle,
    Failed,
}

pub(super) fn classify(result: io::Result<usize>) -> Read {
    match result {
        Ok(count) => bytes(count),
        Err(error) => failure(&error),
    }
}

fn bytes(count: usize) -> Read {
    if count == 0 {
        return Read::End;
    }
    Read::Bytes(count)
}

fn failure(error: &io::Error) -> Read {
    if error.raw_os_error() == Some(libc::EIO) {
        return Read::End;
    }
    if error.kind() == io::ErrorKind::WouldBlock {
        return Read::Idle;
    }
    Read::Failed
}

/// The bounded drain of output still buffered after the command exited.
#[derive(Debug)]
pub(super) struct Drain {
    started: Instant,
    idle_since: Option<Instant>,
    ended: bool,
}

impl Drain {
    pub fn new(now: Instant) -> Self {
        Self {
            started: now,
            idle_since: None,
            ended: false,
        }
    }

    pub fn observe(&mut self, read: &Read, now: Instant) {
        match read {
            Read::Bytes(_) => self.idle_since = None,
            Read::End => self.ended = true,
            Read::Idle | Read::Failed => {
                self.idle_since.get_or_insert(now);
            }
        }
    }

    pub fn finished(&self, now: Instant) -> bool {
        self.ended
            || self
                .idle_since
                .is_some_and(|idle| now.duration_since(idle) >= PTY_DRAIN_IDLE)
            || now.duration_since(self.started) >= PTY_DRAIN_CAP
    }
}

/// The command's progress as the session sees it.
#[derive(Debug)]
pub(super) enum Command {
    Running,
    /// Reaped with `result`; its remaining output is being drained.
    Draining {
        result: CommandResult,
        drain: Drain,
    },
    /// Hung up; the rest of its terminal session is killed at `until`.
    HangingUp {
        until: Instant,
    },
    /// Its exit status is known to the control state machine.
    Ended,
}

impl Command {
    /// Whether the PTY may still have output to read.
    pub fn reads(&self, now: Instant) -> bool {
        match self {
            Self::Running => true,
            Self::Draining { drain, .. } => !drain.finished(now),
            Self::HangingUp { .. } | Self::Ended => false,
        }
    }

    pub fn observe(&mut self, read: &Read, now: Instant) {
        if let Self::Draining { drain, .. } = self {
            drain.observe(read, now);
        }
    }

    /// Whether the hang-up grace has elapsed.
    pub fn hung_up(&self, now: Instant) -> bool {
        matches!(self, Self::HangingUp { until } if now >= *until)
    }

    /// The command's result, once the drain finished.
    pub fn drained(&self, now: Instant) -> Option<CommandResult> {
        match self {
            Self::Draining { result, drain } if drain.finished(now) => Some(result.clone()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reads_are_classified_by_their_result() {
        assert_eq!(classify(Ok(3)), Read::Bytes(3));
        assert_eq!(classify(Ok(0)), Read::End);
        assert_eq!(
            classify(Err(io::Error::from_raw_os_error(libc::EIO))),
            Read::End
        );
        assert_eq!(classify(Err(io::ErrorKind::WouldBlock.into())), Read::Idle);
        assert_eq!(classify(Err(io::Error::other("bad"))), Read::Failed);
    }

    #[test]
    fn a_drain_ends_at_end_of_output() {
        let now = Instant::now();
        let mut drain = Drain::new(now);
        assert!(!drain.finished(now));
        drain.observe(&Read::End, now);
        assert!(drain.finished(now));
    }

    #[test]
    fn a_drain_ends_after_an_idle_period_that_output_restarts() {
        let now = Instant::now();
        let mut drain = Drain::new(now);
        drain.observe(&Read::Idle, now);
        drain.observe(&Read::Idle, now + PTY_DRAIN_IDLE / 2);
        assert!(!drain.finished(now + PTY_DRAIN_IDLE - Duration::from_nanos(1)));
        assert!(drain.finished(now + PTY_DRAIN_IDLE));
        drain.observe(&Read::Bytes(1), now + PTY_DRAIN_IDLE);
        assert!(!drain.finished(now + PTY_DRAIN_IDLE));
        drain.observe(&Read::Failed, now + PTY_DRAIN_IDLE);
        assert!(drain.finished(now + 2 * PTY_DRAIN_IDLE));
    }

    #[test]
    fn a_drain_is_capped_even_while_output_continues() {
        let now = Instant::now();
        let drain = Drain::new(now);
        assert!(!drain.finished(now + PTY_DRAIN_CAP - Duration::from_nanos(1)));
        assert!(drain.finished(now + PTY_DRAIN_CAP));
    }

    #[test]
    fn the_command_reads_until_drained_and_reports_its_result_once_drained() {
        let now = Instant::now();
        let mut command = Command::Running;
        assert!(command.reads(now));
        assert_eq!(command.drained(now), None);
        command.observe(&Read::End, now);
        let result = CommandResult::Exit { code: 7 };
        command = Command::Draining {
            result: result.clone(),
            drain: Drain::new(now),
        };
        assert!(command.reads(now));
        assert_eq!(command.drained(now), None);
        command.observe(&Read::End, now);
        assert!(!command.reads(now));
        assert_eq!(command.drained(now), Some(result));
        assert!(!Command::Ended.reads(now));
        assert_eq!(Command::Ended.drained(now), None);
    }

    #[test]
    fn a_hang_up_neither_reads_nor_drains_and_ends_at_its_deadline() {
        let now = Instant::now();
        let command = Command::HangingUp { until: now };
        assert!(!command.reads(now));
        assert_eq!(command.drained(now), None);
        assert!(!command.hung_up(now - Duration::from_nanos(1)));
        assert!(command.hung_up(now));
        assert!(!Command::Ended.hung_up(now));
        assert!(!Command::Running.hung_up(now));
    }
}
