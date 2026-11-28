//! The PIN key exchange: CPace on ristretto255 with SHA-512, in initiator
//! and responder mode, as `draft-irtf-cfrg-cpace-21` specifies it. The
//! `pakery-cpace` crate does the arithmetic; this module fixes the suite and
//! the mode, and draws each scalar from the caller's randomness.

use crate::Randomness;
use pakery_cpace::{CpaceInitiator, CpaceMode, CpaceResponder, InitiatorState};
use pakery_crypto::CpaceRistretto255;
use rand_core::{TryCryptoRng, TryRng};
use std::convert::Infallible;

/// The size of a public share: one ristretto255 group element.
pub(crate) const SHARE_SIZE: usize = 32;
/// The size of the key an exchange produces: one SHA-512 output.
pub(crate) const KEY_SIZE: usize = 64;

/// What both ends of one exchange must hold alike for their keys to match.
#[derive(Clone, Copy)]
pub(crate) struct Inputs<'a> {
    /// The password, CPace's `PRS`.
    pub(crate) password: &'a [u8],
    /// The channel identifier, `CI`.
    pub(crate) channel: &'a [u8],
    /// The session identifier, `sid`.
    pub(crate) session: &'a [u8],
    /// The initiator's associated data, `ADa`.
    pub(crate) initiator_data: &'a [u8],
    /// The responder's associated data, `ADb`.
    pub(crate) responder_data: &'a [u8],
}

/// An initiator that has sent its share and awaits the responder's.
pub(crate) struct Initiated {
    state: InitiatorState<CpaceRistretto255>,
    responder_data: Vec<u8>,
}

/// Starts an exchange: the initiator's share, and what it keeps to finish.
/// The scalar is 64 bytes of `randomness`, little-endian, reduced modulo
/// the group's order.
pub(crate) fn initiate(
    inputs: Inputs<'_>,
    randomness: &mut dyn Randomness,
) -> Option<([u8; SHARE_SIZE], Initiated)> {
    let (share, state) = CpaceInitiator::<CpaceRistretto255>::start(
        inputs.password,
        inputs.channel,
        inputs.session,
        inputs.initiator_data,
        &mut Drawn(randomness),
    )
    .ok()?;
    let initiated = Initiated {
        state,
        responder_data: inputs.responder_data.to_vec(),
    };
    Some((share.try_into().ok()?, initiated))
}

impl Initiated {
    /// The key, given the responder's share. Nothing when the share is not
    /// a group element or is the neutral element.
    pub(crate) fn finish(self, responder_share: &[u8]) -> Option<[u8; KEY_SIZE]> {
        let mode = CpaceMode::InitiatorResponder;
        let output = self
            .state
            .finish(responder_share, &self.responder_data, mode)
            .ok()?;
        output.isk.as_bytes().try_into().ok()
    }
}

/// Answers an initiator's share: the responder's share and the key. Nothing
/// when the initiator's share is not a group element or is the neutral
/// element. The scalar is drawn as `initiate` draws its own.
pub(crate) fn respond(
    inputs: Inputs<'_>,
    initiator_share: &[u8],
    randomness: &mut dyn Randomness,
) -> Option<([u8; SHARE_SIZE], [u8; KEY_SIZE])> {
    let (share, output) = CpaceResponder::<CpaceRistretto255>::respond(
        initiator_share,
        inputs.password,
        inputs.channel,
        inputs.session,
        inputs.initiator_data,
        inputs.responder_data,
        CpaceMode::InitiatorResponder,
        &mut Drawn(randomness),
    )
    .ok()?;
    Some((
        share.try_into().ok()?,
        output.isk.as_bytes().try_into().ok()?,
    ))
}

/// The caller's randomness, in the shape `pakery` draws it through.
struct Drawn<'a>(&'a mut dyn Randomness);

impl TryRng for Drawn<'_> {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let mut bytes = [0; 4];
        self.0.fill(&mut bytes);
        Ok(u32::from_le_bytes(bytes))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        let mut bytes = [0; 8];
        self.0.fill(&mut bytes);
        Ok(u64::from_le_bytes(bytes))
    }

    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), Infallible> {
        self.0.fill(bytes);
        Ok(())
    }
}

impl TryCryptoRng for Drawn<'_> {}

#[cfg(test)]
mod tests {
    //! SEC-08: the published vectors of `draft-irtf-cfrg-cpace-21`, appendix
    //! B.3 (ristretto255 with SHA-512), run through the link code.

    use super::*;

    /// Randomness that hands out the bytes a test wrote, then zeros.
    struct Scripted(Vec<u8>);

    impl Randomness for Scripted {
        fn fill(&mut self, bytes: &mut [u8]) {
            for byte in bytes {
                *byte = if self.0.is_empty() {
                    0
                } else {
                    self.0.remove(0)
                };
            }
        }
    }

    fn hex(text: &str) -> Vec<u8> {
        let digits: Vec<u8> = text
            .bytes()
            .filter(|byte| !byte.is_ascii_whitespace())
            .map(|byte| (byte as char).to_digit(16).expect("a hex digit") as u8)
            .collect();
        digits
            .chunks(2)
            .map(|pair| pair[0] << 4 | pair[1])
            .collect()
    }

    /// B.3.1: the inputs of the generator.
    const SESSION: &str = "7e4b4791d6a8ef019b936c79fb7f2c57";
    /// B.3.2 and B.3.3: each end's scalar, little-endian.
    const YA_SCALAR: &str = "da3d23700a9e5699258aef94dc060dfda5ebb61f02a5ea77fad53f4ff0976d08";
    const YB_SCALAR: &str = "d2316b454718c35362d83d69df6320f38578ed5984651435e2949762d900b80d";
    /// B.3.2 and B.3.3: each end's share.
    const YA: &str = "d6bac480f2c386c394efc7c47adb9925dcd2630b64f240c50f8d0eec482b9157";
    const YB: &str = "3ea7e0b19560d7c0b0f5734f63b955286dfa8232b5ebe63324e2d9e7433f7258";
    /// B.3.5: the key in initiator and responder mode.
    const ISK: &str = "b69effbf61b51d56401c0f65601abe428de8206feaaf0e32198896dcae7b35cd
                       2b38950a39dfd5d4a79164614c2984f7daa460b588c1e80c3fa2068af7900447";
    /// B.3.11: an encoding that is no group element, and the neutral element.
    const INVALID: &str = "2b3c6b8c4f3800e7aef6864025b4ed79bd599117e427c41bd47d93d654b4a51c";
    const NEUTRAL: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn with<R>(run: impl FnOnce(Inputs<'_>) -> R) -> R {
        let session = hex(SESSION);
        run(Inputs {
            password: b"Password",
            channel: b"\x0bA_initiator\x0bB_responder",
            session: &session,
            initiator_data: b"ADa",
            responder_data: b"ADb",
        })
    }

    /// The published scalar as the randomness that yields it: a scalar below
    /// the group's order survives the reduction of 64 bytes unchanged.
    fn scalar(text: &str) -> Scripted {
        Scripted(hex(text))
    }

    #[test]
    fn sec_08_the_initiator_matches_the_published_cpace_vector() {
        let (share, initiated) =
            with(|inputs| initiate(inputs, &mut scalar(YA_SCALAR))).expect("a share");

        assert_eq!(share.to_vec(), hex(YA));
        assert_eq!(initiated.finish(&hex(YB)).map(Vec::from), Some(hex(ISK)));
    }

    #[test]
    fn sec_08_the_responder_matches_the_published_cpace_vector() {
        let (share, key) = with(|inputs| respond(inputs, &hex(YA), &mut scalar(YB_SCALAR)))
            .expect("a share and a key");

        assert_eq!(share.to_vec(), hex(YB));
        assert_eq!(key.to_vec(), hex(ISK));
    }

    #[test]
    fn sec_08_the_published_invalid_shares_abort_the_exchange() {
        for bad in [INVALID, NEUTRAL] {
            let (_, initiated) =
                with(|inputs| initiate(inputs, &mut scalar(YA_SCALAR))).expect("a share");
            assert!(initiated.finish(&hex(bad)).is_none(), "{bad}");
            let answered = with(|inputs| respond(inputs, &hex(bad), &mut scalar(YB_SCALAR)));
            assert!(answered.is_none(), "{bad}");
        }
        let (_, initiated) =
            with(|inputs| initiate(inputs, &mut scalar(YA_SCALAR))).expect("a share");
        assert!(initiated.finish(&hex(YB)[..31]).is_none());
    }

    #[test]
    fn each_input_changes_the_key() {
        let key = |change: fn(&mut Inputs<'_>)| {
            with(|mut inputs| {
                change(&mut inputs);
                respond(inputs, &hex(YA), &mut scalar(YB_SCALAR)).map(|(_, key)| key)
            })
        };
        let published = key(|_| ());
        assert_eq!(published.map(Vec::from), Some(hex(ISK)));
        let changes: [fn(&mut Inputs<'_>); 5] = [
            |inputs| inputs.password = b"password",
            |inputs| inputs.channel = b"\x0bA_initiator\x0bB_respondeR",
            |inputs| inputs.session = b"",
            |inputs| inputs.initiator_data = b"",
            |inputs| inputs.responder_data = b"",
        ];
        for change in changes {
            assert_ne!(key(change), published);
        }
    }

    #[test]
    fn randomness_reaches_pakery_in_the_order_it_was_drawn() {
        let mut script = Scripted((1..=20).collect());
        let mut drawn = Drawn(&mut script);

        assert_eq!(drawn.try_next_u32(), Ok(0x0403_0201));
        assert_eq!(drawn.try_next_u64(), Ok(0x0c0b_0a09_0807_0605));
        let mut rest = [0; 8];
        assert_eq!(drawn.try_fill_bytes(&mut rest), Ok(()));
        assert_eq!(rest, [13, 14, 15, 16, 17, 18, 19, 20]);
    }
}
