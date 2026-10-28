//! #17 / #20 / ADR 0003: platform exit seams feed a neutral, lossless status.
use climon2_core::exit::{ExitStatus, LaunchFailure};
use std::num::NonZeroU32;

#[test]
fn issue17_failures_remain_distinct_from_command_status_and_signal_mapping() {
    let outcomes = [
        (ExitStatus::OwnFailure, 125),
        (ExitStatus::Usage, 2),
        (ExitStatus::Launch(LaunchFailure::NotFound), 127),
        (ExitStatus::Launch(LaunchFailure::CannotRun), 126),
        (ExitStatus::Command(125), 125),
        (ExitStatus::Command(0xc000013a), 0xc000013a),
        (ExitStatus::Signal(NonZeroU32::new(15).unwrap()), 143),
        (ExitStatus::TerminalGone, 129),
    ];
    for (status, expected) in outcomes {
        assert_eq!(status.code().unwrap(), expected);
    }
    assert!(
        ExitStatus::Signal(NonZeroU32::new(u32::MAX).unwrap())
            .code()
            .is_err()
    );
}
