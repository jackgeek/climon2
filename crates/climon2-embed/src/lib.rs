//! Client embedding (ADR 0011): what it takes to run one client library
//! instance for one session on this machine, shared by the monitor and,
//! later, the relay. It loads and qualifies a release's client library
//! (`load`), runs each instance on a dedicated owner thread (`owner`),
//! takes requests in and gives deliveries, terminal evidence and the run
//! report out (`instance`, `request`, `stream`), accounts for what each
//! session's consumer has not yet processed (`backlog`) and how long a
//! congested client may stall (`stall`), and holds the arithmetic of the
//! admission ledger (`admission`), so that every consumer counts memory
//! the same way.
//!
//! It decides nothing about when an instance starts or what its records
//! mean: scheduling, retry and the reading of records belong to its
//! consumer. It has no terminal, HTTP or WebSocket dependency.
pub mod admission;
pub mod backlog;
pub mod instance;
pub mod load;
pub mod owner;
pub mod request;
pub mod stall;
pub mod stream;

/// A thread stack of `bytes`, saturating.
pub fn stack(bytes: u64) -> usize {
    usize::try_from(bytes).unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    /// A thread's stack is the bytes asked for.
    #[test]
    fn a_stack_is_the_bytes_asked_for() {
        assert_eq!(super::stack(0), 0);
        assert_eq!(super::stack(2 << 20), 2_097_152);
        assert_eq!(super::stack(u64::MAX), usize::MAX);
    }
}
