//! Host input while no session can take it: only the exit action means
//! anything (#74). The full input router replaces this when targets exist.
use climon2_core::keys::ControlKey;

/// Finds prefix+x in raw host bytes, however the host splits its reads. Every
/// other byte is discarded: there is no target to receive it yet.
pub(crate) struct Keys {
    prefix: u8,
    pending: bool,
}

impl Keys {
    pub(crate) fn new(prefix: ControlKey) -> Self {
        Self {
            prefix: prefix.byte(),
            pending: false,
        }
    }

    /// Whether these bytes complete prefix+x.
    pub(crate) fn exit(&mut self, bytes: &[u8]) -> bool {
        bytes.iter().any(|&byte| self.key(byte))
    }

    /// A prefix after a prefix is one literal prefix, not the start of another
    /// action.
    fn key(&mut self, byte: u8) -> bool {
        let prefixed = std::mem::take(&mut self.pending);
        self.pending = !prefixed && byte == self.prefix;
        prefixed && byte == b'x'
    }
}

#[cfg(test)]
mod tests {
    use super::Keys;
    use climon2_core::keys::ControlKey;

    fn keys(prefix: &str) -> Keys {
        Keys::new(ControlKey::from_name(prefix).unwrap())
    }

    /// INP-01 (pure input-router supplement): prefix+x exits, however the
    /// host splits the reads.
    #[test]
    fn prefix_then_x_exits_across_read_boundaries() {
        let mut split = keys("Ctrl+]");
        assert!(!split.exit(b"\x1d"));
        assert!(split.exit(b"x"));
        assert!(keys("Ctrl+]").exit(b"typed\x1dx"));
    }

    /// INP-01 (pure input-router supplement): plain x, Ctrl+C, unknown and
    /// uppercase suffixes and a doubled, literal prefix are discarded.
    #[test]
    fn every_other_key_is_discarded() {
        let mut keys = keys("Ctrl+]");
        assert!(!keys.exit(b"x\x03\x1dy\x1dX\x1d\x1dx"));
        assert!(keys.exit(b"\x1dx"));
    }

    /// INP-01 (pure input-router supplement): only the configured prefix
    /// begins the action.
    #[test]
    fn only_the_configured_prefix_begins_the_action() {
        let mut keys = keys("Ctrl+B");
        assert!(!keys.exit(b"\x1dx"));
        assert!(keys.exit(b"\x02x"));
    }
}
