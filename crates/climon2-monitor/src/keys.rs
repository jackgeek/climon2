//! The prefix actions (spec #73, "Global actions"): the keys that complete
//! them after the configured prefix, and how the key introduction lists them.

/// What a key completes after the prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Action {
    Exit,
    /// Jumps to the session that needs attention (#114).
    Jump,
    /// Opens the selector.
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

/// Prefix+j, which the key introduction does not list.
const JUMP: Binding = Binding {
    key: b'j',
    action: Action::Jump,
    label: "Jump to the session that needs attention",
};

/// The action `byte` completes after a prefix, if any: only lowercase s,
/// x, j and ?.
pub(crate) fn action(byte: u8) -> Option<Action> {
    BINDINGS
        .iter()
        .chain([&JUMP])
        .find(|binding| binding.key == byte)
        .map(|binding| binding.action)
}
