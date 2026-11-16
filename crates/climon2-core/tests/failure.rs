//! #155: the one failure type the CLI, the client executable and the monitor
//! return to their thin mains, which print it after `climon2: error: ` and exit
//! with its code.
use climon2_core::failure::Failure;
use std::{io, path::Path};

/// #155: the diagnostic is one line: control characters in the message are
/// escaped, other characters are kept, and there is no logs suffix until the
/// failure is logged.
#[test]
fn issue155_a_failure_reads_as_one_line_without_logs() {
    let failure = Failure::new(3, "first\nsecond\tthird\u{7f} é");
    assert_eq!(failure.code, 3);
    assert_eq!(failure.logs, None);
    assert_eq!(failure.to_string(), "first\\nsecond\\tthird\\u{7f} é");
}

/// #155: a logged failure names its logs after the message, on the same line.
#[test]
fn issue155_a_logged_failure_names_its_logs_on_the_same_line() {
    let failure = Failure::new(125, "cannot start").logged(Path::new("/home/a\nb/logs"));
    assert_eq!(failure.logs.as_deref(), Some(Path::new("/home/a\nb/logs")));
    assert_eq!(failure.to_string(), "cannot start (logs: /home/a\\nb/logs)");
}

/// #155: the first logs to record a failure keep it, so the most specific log
/// the failure reached stays named as it propagates outwards.
#[test]
fn issue155_the_first_logs_to_record_a_failure_keep_it() {
    let failure = Failure::new(125, "lost")
        .logged(Path::new("/session"))
        .logged(Path::new("/general"));
    assert_eq!(failure.to_string(), "lost (logs: /session)");
}

/// #155: an I/O failure is a climon2 failure: the climon2 error code, 125, and
/// the error's own text.
#[test]
fn issue155_an_io_failure_has_the_climon2_error_code() {
    let failure = Failure::from(io::Error::other("disk on fire"));
    assert_eq!(failure.code, 125);
    assert_eq!(failure.message, "disk on fire");
    assert_eq!(failure.to_string(), "disk on fire");
}
