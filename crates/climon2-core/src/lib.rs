//! Platform-neutral session contracts: parsing and policy, never platform I/O.

pub mod cli;
pub mod diagnostics;
pub mod exit;
mod identity;
mod json;
pub mod layout;
pub mod metadata;
pub mod protocol;
pub mod session;
pub use identity::{ContractError, SessionId, Timestamp, VersionName};

/// Keep hook instrumentation local to each binary's own feature selection (ADR 0004).
#[macro_export]
macro_rules! test_hook_marker {
    () => {
        #[cfg(feature = "test-hooks")]
        {
            static MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";
            std::hint::black_box(MARKER);
        }
    };
}
