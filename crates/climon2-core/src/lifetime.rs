//! A session's lifetime policy, chosen once when `exec` creates the session.
use crate::ContractError;
use std::str::FromStr;

/// Whether a session needs a client to stay alive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifetime {
    /// `climon2 exec`: the leave policy ends the session when nothing attaches
    /// within five seconds of readiness, or when its last attachment is lost
    /// without a detach.
    Ordinary,
    /// `climon2 exec --headless`: the session survives with zero clients until
    /// its command completes or it is killed.
    Headless,
}

/// The daemon's `--lifetime` startup argument values.
const NAMES: [(Lifetime, &str); 2] = [
    (Lifetime::Ordinary, "ordinary"),
    (Lifetime::Headless, "headless"),
];

impl Lifetime {
    pub fn name(self) -> &'static str {
        NAMES
            .iter()
            .find(|(lifetime, _)| *lifetime == self)
            .expect("every lifetime is named")
            .1
    }
}

impl FromStr for Lifetime {
    type Err = ContractError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        NAMES
            .iter()
            .find(|(_, known)| *known == name)
            .map(|(lifetime, _)| *lifetime)
            .ok_or_else(|| ContractError("unknown session lifetime".into()))
    }
}
