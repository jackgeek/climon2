//! What a consumer asks of the monitor library (ADR 0011): one closed set.
//! A consumer decodes its own device's input and sends these; the library
//! keeps the rules that belong to no device.
use climon2_core::SessionId;

/// One action. The set is closed: [`Kind`] names each member, and every
/// consumer says in one exhaustive match how its device reaches each, so
/// an action missing from a consumer fails that consumer's build.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    /// Enter on a row (spec #73, "Enter"): selects it, and takes control
    /// of a connected one.
    Select(SessionId),
    /// Takes control of the selected session while it shows the
    /// non-controller placeholder.
    TakeControl,
    /// Application input for the current target, as opaque bytes. It is
    /// refused without control.
    Input(Vec<u8>),
    /// One part of a paste, for the target that took input when it began.
    Paste(Paste),
    /// The size its consumer can show changed: the selected controller is
    /// resized to it.
    Resize,
    /// Scrolls the selected session's history by rows, back when
    /// positive.
    Scroll(isize),
    /// A confirmed Delete (#117): kills the session first while its row
    /// offers that, then removes it.
    Delete(SessionId),
    /// Dismisses the shown toast.
    Dismiss,
    /// Acknowledges the selected session's attention once it is viewed.
    Acknowledge,
    /// The attention jump (spec #73, "Attention jump target").
    Jump,
}

/// One part of a paste, as its consumer's device delivered it. The library
/// brackets it as the target's application asked when it began.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Paste {
    /// This part begins a paste.
    pub starts: bool,
    pub bytes: Vec<u8>,
    /// This part ends its paste.
    pub ends: bool,
}

/// Each member of [`Action`], without what it carries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Select,
    TakeControl,
    Input,
    Paste,
    Resize,
    Scroll,
    Delete,
    Dismiss,
    Acknowledge,
    Jump,
}

impl Kind {
    /// Every action there is.
    pub const ALL: [Self; 10] = [
        Self::Select,
        Self::TakeControl,
        Self::Input,
        Self::Paste,
        Self::Resize,
        Self::Scroll,
        Self::Delete,
        Self::Dismiss,
        Self::Acknowledge,
        Self::Jump,
    ];
}

impl Action {
    /// Which member of the set this is.
    pub fn kind(&self) -> Kind {
        match self {
            Self::Select(_) => Kind::Select,
            Self::TakeControl => Kind::TakeControl,
            Self::Input(_) => Kind::Input,
            Self::Paste(_) => Kind::Paste,
            Self::Resize => Kind::Resize,
            Self::Scroll(_) => Kind::Scroll,
            Self::Delete(_) => Kind::Delete,
            Self::Dismiss => Kind::Dismiss,
            Self::Acknowledge => Kind::Acknowledge,
            Self::Jump => Kind::Jump,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ADR 0011: the set is closed, and `Kind::ALL` names each member
    /// once, in order.
    #[test]
    fn every_action_has_one_kind() {
        let id: SessionId = "00000000-0000-4000-8000-000000000001".parse().unwrap();
        let paste = Paste {
            starts: true,
            bytes: Vec::new(),
            ends: true,
        };
        let actions = [
            Action::Select(id.clone()),
            Action::TakeControl,
            Action::Input(Vec::new()),
            Action::Paste(paste),
            Action::Resize,
            Action::Scroll(1),
            Action::Delete(id),
            Action::Dismiss,
            Action::Acknowledge,
            Action::Jump,
        ];
        let kinds: Vec<_> = actions.iter().map(Action::kind).collect();
        assert_eq!(kinds, Kind::ALL);
    }
}
