//! The hub monitor's end of a pairing run: it sends its share first and
//! its confirmation first, and keeps nothing of the relay until the relay's
//! own confirmation has been checked.

use super::keys::{Confirmed, Sender};
use super::{
    Arrived, CONFIRMATION_SIZES, Introduction, PairingRun, Pin, SHARE_SIZES, closing, concluded,
    cpace, exchange, read_message, share_message,
};
use crate::opening::Transcript;
use crate::{Input, Output, Randomness, Time};

/// What a hub monitor brings to a pairing run: the PIN its user typed, and
/// the key and name it hands the relay.
pub(crate) struct Offer {
    pub(crate) pin: Pin,
    pub(crate) introduction: Introduction,
}

/// One pairing run at the hub monitor, from a chosen link revision to the
/// close.
pub(crate) struct HubRun {
    introduction: Introduction,
    transcript: Transcript,
    arrived: Arrived,
    /// The key exchange until the relay's share arrives.
    initiated: Option<([u8; cpace::SHARE_SIZE], cpace::Initiated)>,
    /// What the relay's confirmation must confirm, once both shares have
    /// crossed.
    confirmed: Option<Confirmed>,
    pub(crate) outcome: PairingRun,
}

impl HubRun {
    /// Starts a run that must end by `deadline`: the run, and the hub
    /// monitor's share to send.
    pub(crate) fn start(
        offer: Offer,
        transcript: Transcript,
        deadline: Time,
        randomness: &mut dyn Randomness,
    ) -> (Self, Vec<u8>) {
        let initiated = exchange(&offer.pin, &transcript, |inputs| {
            cpace::initiate(inputs, randomness)
        });
        let share = initiated.as_ref().map(|(share, _)| share_message(share));
        let run = Self {
            introduction: offer.introduction,
            transcript,
            arrived: Arrived {
                deadline,
                bytes: Vec::new(),
            },
            initiated,
            confirmed: None,
            outcome: PairingRun::Pending,
        };
        (run, share.unwrap_or_default())
    }

    pub(crate) fn step(&mut self, now: Time, input: Input<'_>) -> Output {
        if self.outcome != PairingRun::Pending {
            return Output::default();
        }
        let sent = self.arrived.take(now, input).then(|| self.advance());
        concluded(&self.outcome, &self.arrived, now, sent.flatten()).unwrap_or_else(|| self.fail())
    }

    /// Closes without another byte: the run failed, and this end keeps
    /// nothing of the relay.
    fn fail(&mut self) -> Output {
        self.outcome = PairingRun::Failed {
            counted: self.initiated.is_none(),
        };
        closing(Vec::new())
    }

    /// Reads what has arrived and gives the bytes to send, or nothing when
    /// the run has failed.
    fn advance(&mut self) -> Option<Vec<u8>> {
        let mut sent = Vec::new();
        if self.read_share(&mut sent)? {
            let confirmation = self.arrived.message(CONFIRMATION_SIZES);
            read_message(confirmation, &mut sent, |body| self.finish(&body))?;
        }
        Some(sent)
    }

    /// Reads the relay's share, unless it has been read, adding the hub
    /// monitor's confirmation to `sent`. Whether the run reads on, or
    /// nothing when it has failed.
    fn read_share(&mut self, sent: &mut Vec<u8>) -> Option<bool> {
        if self.confirmed.is_some() {
            return Some(true);
        }
        let share = self.arrived.message(SHARE_SIZES);
        read_message(share, sent, |share| self.confirm(&share))
    }

    /// Takes the relay's confirmation: the run has paired with the relay it
    /// opens to, and sends nothing more.
    fn finish(&mut self, body: &[u8]) -> Option<Vec<u8>> {
        let relay = self.confirmed.as_ref()?.open(Sender::Relay, body)?;
        self.outcome = PairingRun::Paired(relay);
        Some(Vec::new())
    }

    /// Takes the relay's share and answers with the hub monitor's
    /// confirmation, which is sent first so that a relay without the PIN
    /// gets one guess from this run and no more.
    fn confirm(&mut self, relay_share: &[u8]) -> Option<Vec<u8>> {
        let (hub_share, initiated) = self.initiated.take()?;
        let confirmed = Confirmed {
            key: initiated.finish(relay_share)?,
            transcript: self.transcript.clone(),
            hub_share,
            relay_share: relay_share.try_into().ok()?,
        };
        let message = confirmed.message(Sender::Hub, &self.introduction);
        self.confirmed = Some(confirmed);
        Some(message)
    }
}
