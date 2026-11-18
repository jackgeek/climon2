//! Test-only fault injection: compiled only with `test-hooks`, which release
//! builds never enable (ADR 0004).
use std::io;

/// Whether `CLIMON2_MONITOR_FAULT=<point>` selects this point.
fn selected(point: &str) -> bool {
    std::env::var_os("CLIMON2_MONITOR_FAULT").is_some_and(|fault| fault == point)
}

/// Fails the selected point.
pub(crate) fn fail(point: &str) -> io::Result<()> {
    if selected(point) {
        return Err(io::Error::other(format!("injected {point} failure")));
    }
    Ok(())
}

/// The `poll` fault gives the kernel no descriptor table to wait on, so the
/// wait for host input itself fails.
pub(crate) fn poll_table(table: *mut libc::pollfd) -> *mut libc::pollfd {
    if selected("poll") {
        return std::ptr::null_mut();
    }
    table
}

/// The monitor's identifiers starting where `CLIMON2_MONITOR_FIRST_GENERATION`
/// and `CLIMON2_MONITOR_FIRST_REQUEST` say, near exhaustion (LIM-05),
/// otherwise as `defaults` start them.
pub(crate) fn identifiers(
    defaults: crate::supervisor::Identifiers,
) -> crate::supervisor::Identifiers {
    let first = |name: &str, default: u64| {
        let value = std::env::var(name).ok();
        value
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    crate::supervisor::Identifiers {
        generation: first("CLIMON2_MONITOR_FIRST_GENERATION", defaults.generation),
        request: first("CLIMON2_MONITOR_FIRST_REQUEST", defaults.request),
    }
}
