//! Where toasts (#115) meet viewing and the actions that dismiss them
//! (owner decision on #114, 2026-11-16): a toast covering the selected
//! session stops viewing, so an explicit action's acknowledgement waits
//! until it clears, and prefix+j and Enter dismiss a toast covering their
//! target. No toast is drawn yet.
use climon2_core::SessionId;

/// The toasts on screen: none until #115 draws them.
#[derive(Default)]
pub(crate) struct Toasts;

impl Toasts {
    /// Whether a toast covers `id`'s canvas: never yet.
    pub(crate) fn cover(&self, _id: &SessionId) -> bool {
        false
    }

    /// Prefix+j or Enter on `id` dismisses a toast covering it: none yet.
    pub(crate) fn dismiss(&mut self, _id: &SessionId) {}
}
