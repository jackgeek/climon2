//! The monitor's root OSC 7501 reports to its host
//! (docs/for-humans/protocol/monitor-host): how the terminal interface
//! delivers the monitor library's host attention event.

/// Pending attention, monitoring without it, and the root clear on orderly
/// exit.
pub(crate) const BLOCKED: &[u8] =
    b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\";
pub(crate) const WORKING: &[u8] = b"\x1b]7501;state=working:app=climon2-monitor\x1b\\";
pub(crate) const CLEAR: &[u8] = b"\x1b]7501;state=clear\x1b\\";

/// The report that tells the host attention is `pending` or not.
pub(crate) fn report(pending: bool) -> &'static [u8] {
    [WORKING, BLOCKED][usize::from(pending)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NOT-04 (pure supplement): the host hears each attention state with
    /// the exact bytes.
    #[test]
    fn the_host_hears_each_attention_state_with_its_exact_bytes() {
        let working = b"\x1b]7501;state=working:app=climon2-monitor\x1b\\";
        let blocked = b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\";
        assert_eq!(report(false), &working[..]);
        assert_eq!(report(true), &blocked[..]);
        assert_eq!(CLEAR, b"\x1b]7501;state=clear\x1b\\");
    }
}
