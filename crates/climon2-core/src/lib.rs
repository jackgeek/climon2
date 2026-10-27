//! Platform-neutral session contracts. Implementation follows in #32.

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
