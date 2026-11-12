//! Host input while no session can take it: only the exit action means
//! anything (#74). The full input router replaces this when targets exist.
use climon2_core::keys::ControlKey;

/// The bracketed-paste markers the host sends once the monitor enables them.
const PASTE_START: &[u8; MARKER] = b"\x1b[200~";
const PASTE_END: &[u8; MARKER] = b"\x1b[201~";
const MARKER: usize = 6;

/// Finds typed prefix+x in raw host bytes, however the host splits its reads.
/// Every other byte, pasted or typed, is discarded: there is no target to
/// receive it yet.
pub(crate) struct Keys {
    prefix: u8,
    input: Input,
    /// The latest bytes, to recognise a paste marker as its last byte arrives.
    recent: [u8; MARKER],
}

/// What the host is sending.
#[derive(Clone, Copy)]
enum Input {
    /// Typed keys, with a prefix pending after a lone prefix.
    Typed { pending: bool },
    /// A bracketed paste: data until its end marker.
    Pasted,
}

impl Keys {
    pub(crate) fn new(prefix: ControlKey) -> Self {
        Self {
            prefix: prefix.byte(),
            input: Input::Typed { pending: false },
            recent: Default::default(),
        }
    }

    /// Whether these bytes complete a typed prefix+x.
    pub(crate) fn exit(&mut self, bytes: &[u8]) -> bool {
        bytes.iter().any(|&byte| self.key(byte))
    }

    fn key(&mut self, byte: u8) -> bool {
        match self.input {
            Input::Typed { pending } => self.typed(byte, pending),
            Input::Pasted => self.pasted(byte),
        }
    }

    /// A prefix after a prefix is one literal prefix, not the start of another
    /// action. A paste drops a pending prefix: nothing could take it.
    fn typed(&mut self, byte: u8, pending: bool) -> bool {
        if self.marks(byte, PASTE_START) {
            self.input = Input::Pasted;
            return false;
        }
        self.input = Input::Typed {
            pending: !pending && byte == self.prefix,
        };
        pending && byte == b'x'
    }

    /// Pasted bytes never act; the paste ends with no prefix pending.
    fn pasted(&mut self, byte: u8) -> bool {
        if self.marks(byte, PASTE_END) {
            self.input = Input::Typed { pending: false };
        }
        false
    }

    /// Whether `byte` completes `marker`.
    fn marks(&mut self, byte: u8, marker: &[u8; MARKER]) -> bool {
        self.recent.rotate_left(1);
        self.recent[MARKER - 1] = byte;
        self.recent == *marker
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

    /// INP-03 (pure input-router supplement): a pasted prefix+x is data, and
    /// typed prefix+x exits once the paste has ended.
    #[test]
    fn a_pasted_prefix_and_x_never_exit() {
        let mut keys = keys("Ctrl+]");
        assert!(!keys.exit(b"\x1b[200~\x1dx\x1b[201~"));
        assert!(keys.exit(b"\x1dx"));
    }

    /// INP-03 (pure input-router supplement): a paste discards a pending
    /// prefix, so neither the pasted x nor a typed x after it completes one,
    /// and a pasted prefix leaves none pending.
    #[test]
    fn a_paste_leaves_no_pending_prefix() {
        let mut keys = keys("Ctrl+]");
        assert!(!keys.exit(b"\x1d\x1b[200~x\x1b[201~x"));
        assert!(!keys.exit(b"\x1b[200~\x1d\x1b[201~x"));
        assert!(keys.exit(b"\x1dx"));
    }

    /// INP-02/INP-03 (pure input-router supplement): paste markers and keys
    /// are recognised however the host splits its reads, down to single bytes.
    #[test]
    fn paste_markers_split_into_single_bytes_are_recognised() {
        let mut keys = keys("Ctrl+]");
        for byte in b"\x1b[200~\x1dx\x1b[201~\x1d" {
            assert!(!keys.exit(&[*byte]));
        }
        assert!(keys.exit(b"x"));
    }

    /// INP-02/INP-03 (pure input-router supplement): with Ctrl+[ as prefix,
    /// the Escape that begins a paste marker is not a prefix and a pasted
    /// Escape x is data, while a typed Escape x exits.
    #[test]
    fn an_escape_prefix_still_recognises_pastes() {
        let mut keys = keys("Ctrl+[");
        assert!(!keys.exit(b"\x1b[200~\x1bx\x1b[201~"));
        assert!(keys.exit(b"\x1bx"));
    }

    /// INP-03 (pure input-router supplement): only a complete start marker
    /// begins a paste; a broken one is ordinary input, and a marker restarted
    /// at the breaking Escape still begins one.
    #[test]
    fn only_a_complete_start_marker_begins_a_paste() {
        let mut broken = keys("Ctrl+]");
        assert!(broken.exit(b"\x1b[200\x1dx"));
        let mut restarted = keys("Ctrl+]");
        assert!(!restarted.exit(b"\x1b[20\x1b[200~\x1dx"));
        assert!(restarted.exit(b"\x1b[201~\x1dx"));
    }
}
