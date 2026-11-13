//! The capability families this release relays (#93), in one place: its
//! description advertises exactly these, an Open may require only these, and
//! only these are ever selected. Support comes from the daemon's hello
//! advertisement, never from a release.

use climon2_core::wire::{Advertisement, Family};

/// Core (Open, Leave, Response and Fatal error), passive monitor entry and
/// titles. `focus-v1` joins with #94, `status-v1` with #95, and
/// `notification-v1` and `progress-v1` with #98.
pub(super) const RELAYED: [&str; 3] = ["daemon-wire-v1", "monitor-v1", TITLE];

/// Current titles and their changes.
pub(super) const TITLE: &str = "title-v1";

/// What a monitor-mode Open needs the daemon to advertise, whatever the
/// caller lists, beside `daemon-wire-v1`, which every valid advertisement has.
const MONITOR: Family = Family::MonitorV1;

/// The relayed family `token` names, if any.
pub(super) fn relayed(token: &[u8]) -> Option<&'static str> {
    RELAYED
        .into_iter()
        .find(|family| family.as_bytes() == token)
}

/// Whether the daemon advertises `family`.
fn advertised(advertisement: &Advertisement, family: &str) -> bool {
    Family::from_token(family).is_some_and(|family| advertisement.supports(family))
}

/// Whether the daemon advertises everything a monitor Open requires.
pub(super) fn supports(advertisement: &Advertisement, required: &[&str]) -> bool {
    advertisement.supports(MONITOR)
        && required
            .iter()
            .all(|family| advertised(advertisement, family))
}

/// The optional families to request: those the caller asked for and the
/// daemon advertises, all of them relayed.
pub(super) fn offered(
    advertisement: &Advertisement,
    optional: &[&'static str],
) -> Vec<&'static str> {
    optional
        .iter()
        .copied()
        .filter(|family| advertised(advertisement, family))
        .collect()
}

/// The daemon's selection, as relayed families, if it is one the request
/// allows: every required family, and nothing that was not requested.
pub(super) fn selected(
    selection: &[String],
    required: &[&'static str],
    requested: &[&'static str],
) -> Option<Vec<&'static str>> {
    let selected: Option<Vec<_>> = selection
        .iter()
        .map(|token| requested.iter().copied().find(|family| family == token))
        .collect();
    selected.filter(|selected| required.iter().all(|family| selected.contains(family)))
}
