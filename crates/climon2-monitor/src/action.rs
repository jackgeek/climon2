//! What a consumer asks of the monitor library (ADR 0011): one closed set.
//! A consumer decodes its own device's input and sends these; the library
//! keeps the rules that belong to no device. The set has no wildcard
//! member and is not `non_exhaustive`, so a consumer that matches on it
//! fails its build when a member is added.
use climon2_core::SessionId;

/// One action, by what it acts on: which row is chosen, what drives the
/// selected session, or what the user looks at.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Action {
    Choose(Choice),
    Drive(Drive),
    Look(Look),
}

/// Which row is chosen, or removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Choice {
    /// Enter on a row (spec #73, "Enter"): selects it, and takes control
    /// of a connected one.
    Select(SessionId),
    /// The attention jump (spec #73, "Attention jump target").
    Jump,
    /// A confirmed Delete (#117): kills the session first while its row
    /// offers that, then removes it.
    Delete(SessionId),
}

/// What drives the selected session: its control, its application's
/// input, and its size.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Drive {
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
}

/// What the user looks at.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Look {
    /// Scrolls the selected session's history by rows, back when
    /// positive.
    Scroll(isize),
    /// Dismisses the shown toast.
    Dismiss,
    /// Acknowledges the selected session's attention once it is viewed.
    Acknowledge,
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
