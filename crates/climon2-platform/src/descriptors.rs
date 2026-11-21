//! The process's descriptor budget: how many it may hold, and whether a
//! failure means none is left.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use std::io;

    /// The soft limit on this process's descriptors, read afresh so that a
    /// raised limit counts. A limit that cannot be read reads as none.
    pub fn limit() -> u64 {
        let mut limit = libc::rlimit {
            rlim_cur: u64::MIN,
            rlim_max: u64::MIN,
        };
        // SAFETY: getrlimit writes only `limit`.
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
        limit.rlim_cur
    }

    /// Whether `error` is the process or the system running out of
    /// descriptors.
    pub fn exhausted(error: &io::Error) -> bool {
        matches!(error.raw_os_error(), Some(libc::EMFILE | libc::ENFILE))
    }

    #[cfg(test)]
    mod tests {
        use super::exhausted;
        use std::io;

        /// Only the process's or the system's descriptor table being full is
        /// exhaustion.
        #[test]
        fn only_a_full_descriptor_table_is_exhaustion() {
            for (code, full) in [
                (libc::EMFILE, true),
                (libc::ENFILE, true),
                (libc::EBADF, false),
            ] {
                let error = io::Error::from_raw_os_error(code);
                assert_eq!(exhausted(&error), full, "{error}");
            }
            assert!(!exhausted(&io::Error::other("no code")));
        }
    }
}
