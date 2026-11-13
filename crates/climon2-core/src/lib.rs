//! Platform-neutral session contracts: parsing and policy, never platform I/O.

pub mod cli;
pub mod config;
pub mod diagnostics;
pub mod exit;
pub mod hello;
mod identity;
mod json;
pub mod keys;
pub mod layout;
pub mod metadata;
mod names;
pub mod native;
pub mod preferences;
pub mod protocol;
pub mod selector;
pub mod session;
pub mod startup_envelope;
pub mod title;
pub mod wire;
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
