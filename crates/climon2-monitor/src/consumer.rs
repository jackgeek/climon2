//! What a consumer of the monitor library reports (ADR 0011): only what
//! the library cannot know. The library asks when it needs an answer, so
//! none is ever stale; a consumer answers from its own state and never
//! calls back into the library.
use crate::toast::Toast;
use climon2_core::protocol::TerminalSize;

/// The program that steps a [`Monitor`](crate::Monitor) and shows it on
/// its device: the terminal interface, the web monitor.
pub trait Consumer {
    /// Whether a modal of its own covers the selected session.
    fn covered(&self) -> bool;

    /// Whether the user's window has focus.
    fn focused(&self) -> bool;

    /// The size it can show a session at; none while it has no room for
    /// one.
    fn size(&self) -> Option<TerminalSize>;

    /// Whether it has room to show `toast`.
    fn fits(&self, toast: &Toast) -> bool;

    /// Whether `toast`, where it would place it, covers any cell of a
    /// session's canvas of `geometry` (columns, rows), or of all the room
    /// it has for a canvas before the session has a geometry.
    fn obscures(&self, toast: &Toast, geometry: Option<(u16, u16)>) -> bool;
}
