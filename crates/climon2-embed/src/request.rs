//! The requests a consumer submits through a session's client instance.
use climon2_core::protocol::TerminalSize;

/// A control period's authority: the connection generation, period and
/// restoration ID the daemon judges each request by.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Authority {
    pub generation: u64,
    pub period: u64,
    pub restoration: u64,
}

impl Authority {
    /// Whether `other` belongs to the same control period: its
    /// restoration may differ.
    pub fn same_period(self, other: Self) -> bool {
        (self.generation, self.period) == (other.generation, other.period)
    }
}

/// A request the monitor submits through the session's client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    /// Take control: explicit selection.
    Acquire,
    /// The restoration under this authority is applied.
    Ready(Authority),
    /// The selected controller's desired application area.
    Resize(Authority, TerminalSize),
    /// Application input for the ready controller, at most `chunk_bytes`
    /// once submitted. Refused, it carries the bytes that were not sent.
    Input(Authority, Vec<u8>),
    /// Part of a paste for the ready controller, submitted as input; once
    /// refused, the rest of its paste is discarded.
    Paste(Authority, Paste),
    /// The application's desired keyboard focus (`focus-v1`), under ready
    /// authority: non-user, never input.
    Focus(Authority, bool),
    /// Whether the monitor views the session (`status-v1`; #114): true
    /// only under ready authority.
    Viewing(Authority, bool),
    /// Acknowledges pending attention, after an explicit action, once the
    /// session is viewed (`status-v1`; #114).
    Acknowledge(Authority),
}

/// Part of a paste, as the application receives it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paste {
    /// The application's paste markers go around the paste: it had
    /// bracketed paste on when the paste began.
    pub bracketed: bool,
    /// The paste begins with this part.
    pub starts: bool,
    pub bytes: Vec<u8>,
    /// The paste ends with this part.
    pub ends: bool,
}

impl Paste {
    /// The part that closes a bracketed paste cut short: its end marker
    /// alone.
    pub fn end() -> Self {
        Self {
            bracketed: true,
            starts: false,
            bytes: Vec::new(),
            ends: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A control period is its generation and period together; its
    /// restorations differ within it.
    #[test]
    fn a_control_period_is_its_generation_and_period() {
        let held = Authority {
            generation: 2,
            period: 3,
            restoration: 4,
        };
        let restored = Authority {
            restoration: 5,
            ..held
        };
        assert!(held.same_period(held));
        assert!(held.same_period(restored));
        assert!(!held.same_period(Authority { period: 4, ..held }));
        assert!(!held.same_period(Authority {
            generation: 3,
            ..held
        }));
    }

    /// The part that closes a paste cut short is its end marker alone.
    #[test]
    fn a_closing_part_is_a_bracketed_end_with_no_bytes() {
        let end = Paste {
            bracketed: true,
            starts: false,
            bytes: Vec::new(),
            ends: true,
        };
        assert_eq!(Paste::end(), end);
    }
}
