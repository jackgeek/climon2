//! Test-only fault injection: compiled only with `test-hooks`, which release
//! builds never enable (ADR 0004).
use std::io;

/// Whether `CLIMON2_MONITOR_FAULT=<point>` selects this point.
fn selected(point: &str) -> bool {
    std::env::var_os("CLIMON2_MONITOR_FAULT").is_some_and(|fault| fault == point)
}

/// Fails the selected point.
pub fn fail(point: &str) -> io::Result<()> {
    if selected(point) {
        return Err(io::Error::other(format!("injected {point} failure")));
    }
    Ok(())
}

/// The `poll` fault fails the wait for host input as the kernel fails a wait
/// given no descriptor table: with EFAULT, which is 14 on Linux and macOS.
pub fn wait_failure() -> io::Result<()> {
    const EFAULT: i32 = 14;
    if selected("poll") {
        return Err(io::Error::from_raw_os_error(EFAULT));
    }
    Ok(())
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

/// Stalls the selected point for good: a thread that never finishes, for
/// the fail-stop (#118).
pub fn stall(point: &str) {
    while selected(point) {
        std::thread::park();
    }
}
