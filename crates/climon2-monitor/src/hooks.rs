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
