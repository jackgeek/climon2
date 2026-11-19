//! Attention as the monitor acts on it (spec #73, "Viewing, focus,
//! acknowledgement and host signals"; #114): pure. An explicit action's
//! acknowledgement waits until its target is viewed, the host hears whether
//! any connected session needs attention, and prefix+j that finds nothing
//! says so for a while.
use crate::terminal::{Authority, Control};
use climon2_core::SessionId;
use std::time::{Duration, Instant};

/// What prefix+j says when no session qualifies.
pub(crate) const NO_ATTENTION: &str = "No sessions need attention";
/// How long [`NO_ATTENTION`] shows (owner decision on #114, 2026-11-16).
const NOTICE_FOR: Duration = Duration::from_secs(3);

/// The monitor's root OSC 7501 reports to its host
/// (docs/for-humans/protocol/monitor-host): pending attention, monitoring without it,
/// and the root clear on orderly exit.
pub(crate) const BLOCKED: &[u8] =
    b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\";
pub(crate) const WORKING: &[u8] = b"\x1b]7501;state=working:app=climon2-monitor\x1b\\";
pub(crate) const CLEAR: &[u8] = b"\x1b]7501;state=clear\x1b\\";

/// What the host was last told: nothing yet, or whether attention was
/// pending.
#[derive(Default)]
pub(crate) struct HostReport {
    pending: Option<bool>,
}

impl HostReport {
    /// The report to write now that attention is `pending` or not: the
    /// initial state, then only changes. There is no heartbeat.
    pub(crate) fn report(&mut self, pending: bool) -> Option<&'static [u8]> {
        let changed = self.pending.replace(pending) != Some(pending);
        changed.then_some([WORKING, BLOCKED][usize::from(pending)])
    }
}

/// [`NO_ATTENTION`], from when prefix+j found nothing until
/// [`NOTICE_FOR`] has passed.
#[derive(Default)]
pub(crate) struct Notice {
    since: Option<Instant>,
}

impl Notice {
    /// Prefix+j found nothing at `now`: the notice shows afresh.
    pub(crate) fn show(&mut self, now: Instant) {
        self.since = Some(now);
    }

    /// Prefix+j found a session: no notice.
    pub(crate) fn clear(&mut self) {
        self.since = None;
    }

    /// The notice's text at `now`, while it shows.
    pub(crate) fn text(&self, now: Instant) -> Option<&'static str> {
        self.since
            .filter(|since| now.saturating_duration_since(*since) < NOTICE_FOR)
            .map(|_| NO_ATTENTION)
    }
}

/// An explicit action's acknowledgement (Enter, prefix+j, or closing the
/// selector back to the session), deferred until its target is viewed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Deferred {
    id: SessionId,
    /// The generation and control period the target held when the
    /// acknowledgement first saw it held.
    period: Option<(u64, u64)>,
}

/// The selected target as one painted frame left it.
pub(crate) struct Target<'a> {
    pub(crate) id: Option<&'a SessionId>,
    /// A modal covers it.
    pub(crate) covered: bool,
    pub(crate) control: Control,
    /// The authority its control period and restoration hold, if any.
    pub(crate) authority: Option<Authority>,
    /// The monitor views it ([`Target::viewing`] is reported to its daemon).
    pub(crate) viewing: bool,
}

/// What a deferred acknowledgement does after a frame.
#[derive(Debug, Eq, PartialEq)]
pub(crate) enum Next {
    /// Its target is not viewed yet: a toast covers it, the host is
    /// unfocused, its view is scrolled back, it is too small or not ready.
    Wait(Deferred),
    /// Its target is viewed: acknowledge it under this authority, once.
    Submit(SessionId, Authority),
    /// The target context changed first: it is cancelled.
    Cancel,
}

impl Deferred {
    pub(crate) fn new(id: SessionId) -> Self {
        Self { id, period: None }
    }

    /// Submits once `target` is viewed, waits while it is not, and is
    /// cancelled once the target context changed: another selection, a
    /// modal opening over it, control lost or never gained, or another
    /// control period than the one it first saw. A new restoration in that
    /// period is no change: a resize brings one.
    pub(crate) fn next(self, target: &Target<'_>) -> Next {
        if !self.unchanged(target) {
            return Next::Cancel;
        }
        match (target.viewing, target.authority) {
            (true, Some(authority)) => Next::Submit(self.id, authority),
            (_, authority) => Next::Wait(Self {
                period: authority.map(period).or(self.period),
                ..self
            }),
        }
    }

    fn unchanged(&self, target: &Target<'_>) -> bool {
        let selected = target.id == Some(&self.id);
        selected && !target.covered && controlled(target.control) && self.kept(target.authority)
    }

    /// Whether the control period it first saw, if any, is still held.
    fn kept(&self, authority: Option<Authority>) -> bool {
        self.period.is_none() || authority.map(period) == self.period
    }
}

/// Whether control is held or being acquired.
fn controlled(control: Control) -> bool {
    !matches!(control, Control::NotControlling | Control::Ended)
}

fn period(authority: Authority) -> (u64, u64) {
    (authority.generation, authority.period)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(suffix: u8) -> SessionId {
        format!("00000000-0000-4000-8000-0000000000{suffix:02x}")
            .parse()
            .unwrap()
    }

    fn authority(period: u64, restoration: u64) -> Authority {
        Authority {
            generation: 1,
            period,
            restoration,
        }
    }

    fn target(
        id: &SessionId,
        control: Control,
        authority: Option<Authority>,
        viewing: bool,
    ) -> Target<'_> {
        Target {
            id: Some(id),
            covered: false,
            control,
            authority,
            viewing,
        }
    }

    /// NOT-04 (pure supplement): the host hears the initial state, then each
    /// change once, with the exact bytes; repeats send nothing.
    #[test]
    fn the_host_hears_the_initial_attention_state_and_changes_only() {
        let mut host = HostReport::default();
        let reports: Vec<_> = [false, false, true, true, false, true]
            .into_iter()
            .map(|pending| host.report(pending))
            .collect();
        let working = b"\x1b]7501;state=working:app=climon2-monitor\x1b\\";
        let blocked = b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x1b\\";
        assert_eq!(
            reports,
            [
                Some(&working[..]),
                None,
                Some(&blocked[..]),
                None,
                Some(&working[..]),
                Some(&blocked[..])
            ]
        );
        assert_eq!(HostReport::default().report(true), Some(&blocked[..]));
        assert_eq!(CLEAR, b"\x1b]7501;state=clear\x1b\\");
    }

    /// NOT-03 (pure supplement; owner decision on #114): `No sessions need
    /// attention` shows from prefix+j for 3 seconds, up to the limit and
    /// not at it; a later prefix+j that finds nothing shows it afresh, and
    /// one that finds a session clears it.
    #[test]
    fn the_notice_shows_for_three_seconds() {
        let start = Instant::now();
        let mut notice = Notice::default();
        assert_eq!(notice.text(start), None);
        notice.show(start);
        let limit = start + Duration::from_secs(3);
        assert_eq!(notice.text(start), Some("No sessions need attention"));
        let almost = limit - Duration::from_nanos(1);
        assert_eq!(notice.text(almost), Some(NO_ATTENTION));
        assert_eq!(notice.text(limit), None);
        notice.show(limit);
        assert_eq!(
            notice.text(limit + Duration::from_secs(2)),
            Some(NO_ATTENTION)
        );
        notice.clear();
        assert_eq!(notice.text(limit), None);
    }

    /// STA-08 and NOT-03 (pure supplement): a deferred acknowledgement waits
    /// while its target is acquired, restored or not viewed, keeping the
    /// first control period it sees across restorations in it, and submits
    /// under the authority held once its target is viewed.
    #[test]
    fn a_deferred_acknowledgement_submits_once_its_target_is_viewed() {
        let alpha = id(1);
        let deferred = Deferred::new(alpha.clone());
        let acquiring = target(&alpha, Control::Acquiring, None, false);
        let Next::Wait(deferred) = deferred.next(&acquiring) else {
            panic!("acquiring waits");
        };
        let restoring = target(&alpha, Control::Restoring, Some(authority(2, 1)), false);
        let Next::Wait(deferred) = deferred.next(&restoring) else {
            panic!("restoring waits");
        };
        let resized = target(&alpha, Control::Controlling, Some(authority(2, 2)), false);
        let Next::Wait(deferred) = deferred.next(&resized) else {
            panic!("an unviewed target waits");
        };
        let viewed = target(&alpha, Control::Controlling, Some(authority(2, 3)), true);
        assert_eq!(
            deferred.next(&viewed),
            Next::Submit(alpha.clone(), authority(2, 3))
        );
        let viewed = target(&alpha, Control::Controlling, Some(authority(4, 1)), true);
        assert_eq!(
            Deferred::new(alpha.clone()).next(&viewed),
            Next::Submit(alpha.clone(), authority(4, 1))
        );
    }

    /// STA-08 and NOT-03 (pure supplement): a deferred acknowledgement is
    /// cancelled when the target context changes first: another selection
    /// or none, a modal opening over it, control lost, ended or never
    /// asked for, or another control period, also of another generation.
    #[test]
    fn a_deferred_acknowledgement_is_cancelled_when_its_target_context_changes() {
        let (alpha, beta) = (id(1), id(2));
        let held = Some(authority(2, 1));
        let seen = || {
            let first = target(&alpha, Control::Controlling, held, false);
            match Deferred::new(alpha.clone()).next(&first) {
                Next::Wait(deferred) => deferred,
                next => panic!("{next:?}"),
            }
        };
        let changed = [
            target(&beta, Control::Controlling, held, true),
            Target {
                id: None,
                ..target(&alpha, Control::Controlling, held, true)
            },
            Target {
                covered: true,
                ..target(&alpha, Control::Controlling, held, false)
            },
            target(&alpha, Control::NotControlling, None, false),
            target(&alpha, Control::Ended, held, false),
            target(&alpha, Control::Acquiring, None, false),
            target(&alpha, Control::Controlling, Some(authority(3, 1)), true),
            target(
                &alpha,
                Control::Controlling,
                Some(Authority {
                    generation: 2,
                    ..authority(2, 1)
                }),
                true,
            ),
        ];
        for (index, target) in changed.iter().enumerate() {
            assert_eq!(seen().next(target), Next::Cancel, "{index}");
        }
        let unasked = target(&alpha, Control::NotControlling, None, false);
        assert_eq!(Deferred::new(alpha.clone()).next(&unasked), Next::Cancel);
    }
}
