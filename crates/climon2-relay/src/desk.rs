//! The relay's link invitation as its user handles it (P6-8, X6): the pair
//! key mints one, and the screen shows its PIN while it is live, then how
//! it ended. The PIN is shown here and nowhere else (P26).

use crate::links::System;
use climon2_link::{Invitation, Invitations, Pin, PublicKey, Time, display_name};

/// What the screen shows of the invitation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    /// None was minted.
    Nothing,
    /// A hub monitor holding this PIN can pair.
    Live { pin: String },
    /// A hub monitor paired: its name and key fingerprint (P23).
    Paired { name: String, fingerprint: String },
    /// It ended unused: why.
    Ended(&'static str),
}

/// The relay's invitation and the PIN of the one it minted last.
pub(crate) struct Desk {
    invitations: Invitations,
    pin: Option<Pin>,
}

impl Desk {
    pub(crate) fn new(invitations: Invitations) -> Self {
        Self {
            invitations,
            pin: None,
        }
    }

    /// Mints an invitation at `now`, revoking any before it.
    pub(crate) fn mint(&mut self, now: Time) {
        self.pin = Some(self.invitations.mint(now, &mut System));
    }

    /// What the invitation shows at `now`.
    pub(crate) fn shown(&self, now: Time) -> Shown {
        match self.invitations.invitation(now) {
            Invitation::None => Shown::Nothing,
            Invitation::Live { .. } => Shown::Live {
                pin: self.pin.as_ref().map(Pin::to_string).unwrap_or_default(),
            },
            Invitation::Paired(hub) => Shown::Paired {
                name: display_name(hub.name()),
                fingerprint: PublicKey::from_bytes(*hub.key()).fingerprint(),
            },
            Invitation::Expired => Shown::Ended("The invitation expired"),
            Invitation::UsedUp => Shown::Ended("The invitation was used up"),
            Invitation::Cancelled => Shown::Ended("The invitation was cancelled"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use climon2_link::{End, HubEnd, Input, Introduction, RelayEnd, Revisions};
    use std::time::Duration;

    /// A hub monitor named `name` runs pairing with `pin` against `desk`'s
    /// relay, in memory, at second 1.
    fn pair(desk: &Desk, pin: &str, name: &str) {
        let introduction = Introduction::new([9; 32], name).unwrap();
        let pin = Pin::parse(pin).unwrap();
        let mut hub = HubEnd::pair(Revisions::published(), pin, introduction);
        let mut relay = RelayEnd::new(Revisions::published(), desk.invitations.clone());
        let now = at(1);
        let mut from_hub = hub.step(now, Input::Tick, &mut System);
        loop {
            let from_relay = relay.step(now, Input::Bytes(&from_hub.bytes), &mut System);
            if from_hub.close {
                return;
            }
            from_hub = hub.step(now, Input::Bytes(&from_relay.bytes), &mut System);
            if from_relay.close {
                return;
            }
        }
    }

    fn other(pin: &str) -> String {
        let digits: String = pin.chars().filter(char::is_ascii_digit).collect();
        let last = digits.as_bytes()[7] - b'0';
        format!("{}{}", &digits[..7], (last + 1) % 10)
    }

    fn at(seconds: u64) -> Time {
        Time::since_start(Duration::from_secs(seconds))
    }

    fn desk() -> Desk {
        Desk::new(Invitations::new(
            Introduction::new([7; 32], "buildbox").unwrap(),
        ))
    }

    fn pin(shown: Shown) -> String {
        let Shown::Live { pin } = shown else {
            panic!("a live invitation: {shown:?}");
        };
        pin
    }

    #[test]
    fn nothing_is_shown_until_an_invitation_is_minted() {
        assert_eq!(desk().shown(at(0)), Shown::Nothing);
    }

    /// P5: the PIN is shown as `1234-5678` while the invitation is live.
    #[test]
    fn a_minted_invitation_shows_its_pin() {
        let mut desk = desk();
        desk.mint(at(0));
        let shown = pin(desk.shown(at(1)));
        assert_eq!(shown.len(), 9, "{shown}");
        assert_eq!(&shown[4..5], "-");
        assert!(Pin::parse(&shown).is_some(), "{shown}");
    }

    /// P7: minting again revokes the old invitation, and the new PIN shows.
    #[test]
    fn minting_again_shows_the_new_pin() {
        let mut desk = desk();
        let mut pins = Vec::new();
        for second in 0..8 {
            desk.mint(at(second));
            pins.push(pin(desk.shown(at(second))));
        }
        pins.sort();
        pins.dedup();
        assert!(pins.len() > 1, "eight mints gave one PIN: {pins:?}");
    }

    /// P9: an invitation expires 5 minutes after it was minted.
    #[test]
    fn an_expired_invitation_says_so() {
        let mut desk = desk();
        desk.mint(at(0));
        assert_eq!(desk.shown(at(300)), Shown::Ended("The invitation expired"));
    }

    #[test]
    fn a_cancelled_invitation_says_so() {
        let mut desk = desk();
        desk.mint(at(0));
        desk.invitations.cancel();
        assert_eq!(
            desk.shown(at(1)),
            Shown::Ended("The invitation was cancelled")
        );
    }

    /// P23: a hub monitor that paired is shown by its name and its key's
    /// fingerprint, the name without control characters.
    #[test]
    fn a_paired_invitation_shows_the_hub_monitors_name_and_fingerprint() {
        let mut desk = desk();
        desk.mint(at(0));
        let pin = pin(desk.shown(at(0)));
        pair(&desk, &pin, "lap\x1btop");
        assert_eq!(
            desk.shown(at(2)),
            Shown::Paired {
                name: "laptop".into(),
                fingerprint: PublicKey::from_bytes([9; 32]).fingerprint(),
            }
        );
    }

    /// P11: the third failed run uses the invitation up.
    #[test]
    fn a_used_up_invitation_says_so() {
        let mut desk = desk();
        desk.mint(at(0));
        let wrong = other(&pin(desk.shown(at(0))));
        for _ in 0..3 {
            pair(&desk, &wrong, "laptop");
        }
        assert_eq!(
            desk.shown(at(2)),
            Shown::Ended("The invitation was used up")
        );
    }
}
