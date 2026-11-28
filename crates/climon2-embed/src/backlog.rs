//! What each client instance delivered that its consumer has not yet
//! processed (#119, LIM-02). Each delivery is copied and released at once,
//! so ownership of the bytes moves to the consumer and so does their
//! accounting: what was copied is charged here, by the allocation it
//! holds, until the consumer has processed it. A client whose unprocessed
//! deliveries fill its `queues.event_bytes` is not drained again until the
//! consumer catches up; the library holds what it has not delivered, and
//! nothing is dropped.
use climon2_core::SessionId;
use std::{
    collections::BTreeMap,
    sync::{Mutex, PoisonError},
};

/// Each session's deliveries copied for the consumer and not yet
/// processed, in bytes.
#[derive(Default)]
pub struct Backlog(Mutex<BTreeMap<SessionId, usize>>);

impl Backlog {
    /// `bytes`, copied for `id`, are charged until they are processed.
    pub fn charge(&self, id: &SessionId, bytes: usize) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let entry = held.entry(id.clone()).or_default();
        *entry = entry.saturating_add(bytes);
    }

    /// The consumer processed `bytes` charged to `id`: they are free
    /// again.
    pub fn release(&self, id: &SessionId, bytes: usize) {
        let mut held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(entry) = held.get_mut(id) {
            *entry = entry.saturating_sub(bytes);
        }
    }

    /// The bytes `id`'s unprocessed deliveries hold.
    pub fn held(&self, id: &SessionId) -> usize {
        let held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.get(id).copied().unwrap_or_default()
    }

    /// The bytes every session's unprocessed deliveries hold together.
    pub fn total(&self) -> usize {
        let held = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        held.values().sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u8) -> SessionId {
        format!("0d5c0000-0000-4000-8000-0000000000{index:02x}")
            .parse()
            .unwrap()
    }

    /// LIM-02 (pure supplement): a session's deliveries are held until
    /// each is processed, apart from every other session's, and the total
    /// is theirs together; releasing what was never charged frees nothing,
    /// and neither a charge nor a release wraps.
    #[test]
    fn deliveries_are_held_per_session_until_processed() {
        let backlog = Backlog::default();
        let (first, second) = (id(1), id(2));
        backlog.release(&first, 40);
        assert_eq!(backlog.held(&first), 0);
        assert_eq!(backlog.total(), 0);
        backlog.charge(&first, 90);
        backlog.charge(&first, 40);
        backlog.charge(&second, 7);
        assert_eq!(backlog.held(&first), 130);
        assert_eq!(backlog.held(&second), 7);
        assert_eq!(backlog.total(), 137);
        backlog.release(&first, 90);
        assert_eq!(backlog.held(&first), 40);
        backlog.release(&first, 41);
        assert_eq!(backlog.held(&first), 0);
        assert_eq!(backlog.total(), 7);
        backlog.charge(&second, usize::MAX);
        assert_eq!(backlog.held(&second), usize::MAX);
    }
}
