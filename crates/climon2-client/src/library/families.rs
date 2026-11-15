//! The capability families this release relays (#93), in one place and in
//! core's own type: its description advertises exactly these, an Open may
//! require only these, and only these are ever selected. Support comes from
//! the daemon's hello advertisement, never from a release.

use climon2_core::wire::{Advertisement, Families, Family, Requested};

/// Core (Open, Leave, Response and Fatal error), passive monitor entry and
/// titles. `focus-v1` joins with #94, `status-v1` with #95, and
/// `notification-v1` and `progress-v1` with #98.
pub(super) const RELAYED: [Family; 3] = [Family::DaemonWireV1, Family::MonitorV1, Family::TitleV1];

/// The relayed family `token` names, if any.
pub(super) fn relayed(token: &str) -> Option<Family> {
    Family::from_token(token).filter(|family| RELAYED.contains(family))
}

/// Whether the daemon advertises the relayed family `token` names.
fn advertised(advertisement: &Advertisement, token: &str) -> bool {
    relayed(token).is_some_and(|family| advertisement.supports(family))
}

/// Whether the daemon advertises everything a monitor Open requires:
/// `monitor-v1`, whatever the caller lists, and every required family.
pub(super) fn supports(advertisement: &Advertisement, requested: &Requested) -> bool {
    advertisement.supports(Family::MonitorV1)
        && requested
            .required()
            .tokens()
            .iter()
            .all(|token| advertised(advertisement, token))
}

/// The Open to send: every required family, and the optional ones the
/// daemon advertises.
pub(super) fn offered(advertisement: &Advertisement, requested: &Requested) -> Requested {
    requested.retain_optional(|token| advertised(advertisement, token))
}

/// The families a daemon selected, in its order, held inline: a request
/// names each relayed family at most once, so at most all of them.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct Selection([Option<Family>; RELAYED.len()]);

impl Selection {
    pub(super) fn families(&self) -> impl Iterator<Item = Family> + '_ {
        self.0.iter().flatten().copied()
    }

    pub(super) fn contains(&self, family: Family) -> bool {
        self.0.contains(&Some(family))
    }
}

/// The daemon's selection, if it is one `sent` allows: every required
/// family, and nothing that was not requested. Every requested family is
/// relayed, and a selection names each once, so it fits a [`Selection`].
pub(super) fn selected(selection: &Families, sent: &Requested) -> Option<Selection> {
    let tokens = selection.tokens();
    let requested = |token: &String| {
        sent.required().tokens().contains(token) || sent.optional().tokens().contains(token)
    };
    let allowed = tokens.iter().all(requested)
        && sent
            .required()
            .tokens()
            .iter()
            .all(|token| tokens.contains(token));
    allowed.then(|| {
        let mut chosen = Selection::default();
        for (slot, token) in chosen.0.iter_mut().zip(tokens) {
            *slot = relayed(token);
        }
        chosen
    })
}
