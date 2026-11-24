//! The capability families this release relays (#93), in one place and in
//! core's own type: its description advertises exactly these, an Open may
//! require only these, and only these are ever selected. Support comes from
//! the daemon's hello advertisement, never from a release.

use climon2_core::wire::{Advertisement, Families, Family, Mode, Requested};

/// Core (roles, control, restoration and display), passive monitor entry,
/// titles, application focus (#94), status with viewing and
/// acknowledgement (#95), notifications and progress (#98), and Ctrl+Break
/// (#326): every revision-1 family.
pub(super) const RELAYED: [Family; 8] = [
    Family::DaemonWireV1,
    Family::MonitorV1,
    Family::TitleV1,
    Family::StatusV1,
    Family::NotificationV1,
    Family::ProgressV1,
    Family::FocusV1,
    Family::BreakV1,
];

/// The relayed family `token` names, if any.
pub(super) fn relayed(token: &str) -> Option<Family> {
    Family::from_token(token).filter(|family| RELAYED.contains(family))
}

/// Whether the daemon advertises the relayed family `token` names.
fn advertised(advertisement: &Advertisement, token: &str) -> bool {
    relayed(token).is_some_and(|family| advertisement.supports(family))
}

/// A first role, with the families it requires.
#[derive(Clone, Copy)]
pub(super) enum Role<'a> {
    /// An Open: `monitor-v1` for monitor mode, whatever the caller lists,
    /// and every required family.
    Open(Mode, &'a Requested),
    /// A Kill: every required family; core support is implicit (#96).
    Kill(&'a Families),
}

/// Whether the daemon advertises everything `role` requires.
pub(super) fn supports(advertisement: &Advertisement, role: Role) -> bool {
    let (monitor, required) = match role {
        Role::Open(mode, requested) => (mode == Mode::Monitor, requested.required()),
        Role::Kill(required) => (false, required),
    };
    (!monitor || advertisement.supports(Family::MonitorV1))
        && required
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
