//! Host input while no session can take it (#74): the prefix actions, and the
//! typed keys the key introduction takes (#105). The full input router
//! replaces this when targets exist.
use climon2_core::keys::ControlKey;

/// What a key completes after the prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Exit,
    /// The session list arrives with the selector (#106); until then this
    /// action does nothing.
    SessionList,
    KeyIntroduction,
}

/// A prefix action: the key that completes it after the prefix, and how the
/// key introduction describes it.
#[derive(Clone, Copy)]
pub(crate) struct Binding {
    pub(crate) key: u8,
    pub(crate) action: Action,
    pub(crate) label: &'static str,
}

/// Prefix+x, which the empty state's key hint also names.
pub(crate) const EXIT: Binding = Binding {
    key: b'x',
    action: Action::Exit,
    label: "Exit the monitor",
};

/// Every prefix action, in the order the key introduction lists them
/// (selector document, "Key introduction and help").
pub(crate) const BINDINGS: [Binding; 3] = [
    EXIT,
    Binding {
        key: b's',
        action: Action::SessionList,
        label: "Open the session list",
    },
    Binding {
        key: b'?',
        action: Action::KeyIntroduction,
        label: "Show this dialog",
    },
];

/// The bracketed-paste markers the host sends once the monitor enables them.
const PASTE_START: &[u8; MARKER] = b"\x1b[200~";
const PASTE_END: &[u8; MARKER] = b"\x1b[201~";
const MARKER: usize = 6;

/// A typed key, as raw host bytes complete it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Key {
    /// A prefix and the key after it that completes an action.
    Action(Action),
    /// A typed byte outside a prefix. A prefix and the byte after it that
    /// completes no action reach no target yet, and neither does pasted data.
    Typed(u8),
}

/// Finds typed keys in raw host bytes, however the host splits its reads.
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

    /// The key `byte` completes, if any.
    pub(crate) fn key(&mut self, byte: u8) -> Option<Key> {
        match self.input {
            Input::Typed { pending } => self.typed(byte, pending),
            Input::Pasted => self.pasted(byte),
        }
    }

    /// A prefix after a prefix is one literal prefix, not the start of another
    /// action. A paste drops a pending prefix: nothing could take it.
    fn typed(&mut self, byte: u8, pending: bool) -> Option<Key> {
        if self.marks(byte, PASTE_START) {
            self.input = Input::Pasted;
            return None;
        }
        self.input = Input::Typed { pending: false };
        if pending {
            return action(byte);
        }
        if byte == self.prefix {
            self.input = Input::Typed { pending: true };
            return None;
        }
        Some(Key::Typed(byte))
    }

    /// Pasted bytes are never keys; the paste ends with no prefix pending.
    fn pasted(&mut self, byte: u8) -> Option<Key> {
        if self.marks(byte, PASTE_END) {
            self.input = Input::Typed { pending: false };
        }
        None
    }

    /// Whether `byte` completes `marker`.
    fn marks(&mut self, byte: u8, marker: &[u8; MARKER]) -> bool {
        self.recent.rotate_left(1);
        self.recent[MARKER - 1] = byte;
        self.recent == *marker
    }
}

/// The action a typed byte completes after a prefix, if any.
fn action(byte: u8) -> Option<Key> {
    BINDINGS
        .iter()
        .find(|binding| binding.key == byte)
        .map(|binding| Key::Action(binding.action))
}

#[cfg(test)]
mod tests {
    use super::{Action, Key, Keys};
    use climon2_core::keys::ControlKey;

    const EXIT: Key = Key::Action(Action::Exit);
    const KEY_INTRODUCTION: Key = Key::Action(Action::KeyIntroduction);

    fn keys(prefix: &str) -> Keys {
        Keys::new(ControlKey::from_name(prefix).unwrap())
    }

    impl Keys {
        /// Whether these bytes complete a typed prefix+x.
        fn exit(&mut self, bytes: &[u8]) -> bool {
            bytes.iter().any(|&byte| self.key(byte) == Some(EXIT))
        }

        /// Every key these bytes complete, in order.
        fn all(&mut self, bytes: &[u8]) -> Vec<Key> {
            bytes.iter().filter_map(|&byte| self.key(byte)).collect()
        }
    }

    /// HLP-01 (pure input-router supplement): prefix+? opens the key
    /// introduction however the host splits the reads, and only after the
    /// configured prefix.
    #[test]
    fn prefix_then_question_mark_opens_the_key_introduction_across_read_boundaries() {
        let mut split = keys("Ctrl+B");
        assert_eq!(split.all(b"\x02"), []);
        assert_eq!(split.all(b"?"), [KEY_INTRODUCTION]);
        assert_eq!(
            keys("Ctrl+B").all(b"?\x1d?"),
            [Key::Typed(b'?'), Key::Typed(0x1d), Key::Typed(b'?')]
        );
    }

    /// HLP-02 (pure input-router supplement): only typed keys outside a
    /// prefix reach the dialog. A prefix with a key it does not complete,
    /// a doubled prefix included, reaches nothing. Pasted Space, Enter,
    /// prefix+? and prefix+x are data, never keys.
    #[test]
    fn only_unprefixed_typed_keys_reach_the_dialog() {
        let mut keys = keys("Ctrl+]");
        assert_eq!(
            keys.all(b" \r\x03\x1d\r\x1d \x1d\x1dq"),
            [
                Key::Typed(b' '),
                Key::Typed(b'\r'),
                Key::Typed(0x03),
                Key::Typed(b'q')
            ]
        );
        let pasted = keys.all(b"\x1b[200~ \r\x1d?\x1dx\x1b[201~");
        assert!(
            !pasted
                .iter()
                .any(|key| matches!(key, Key::Action(_) | Key::Typed(b' ' | b'\r'))),
            "{pasted:?}"
        );
        assert_eq!(keys.all(b"\x1d?"), [KEY_INTRODUCTION]);
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
    /// uppercase suffixes and a doubled, literal prefix never exit.
    #[test]
    fn every_other_key_is_not_an_exit() {
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
