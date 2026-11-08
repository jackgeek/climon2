//! Test-only fault injection: compiled only with `test-hooks`, which release
//! builds never enable (ADR 0004).
use std::io;

/// `CLIMON2_MONITOR_FAULT=<point>` makes that point fail.
pub(crate) fn fail(point: &str) -> io::Result<()> {
    if std::env::var_os("CLIMON2_MONITOR_FAULT").is_some_and(|fault| fault == point) {
        return Err(io::Error::other(format!("injected {point} failure")));
    }
    Ok(())
}
