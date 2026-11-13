//! A session's lifetime policy, chosen once when `exec` creates the session.
use crate::ContractError;
use std::str::FromStr;

/// A session's lifetime policy: whether it is an ordinary or a headless session.
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

impl Lifetime {
    const ALL: [Self; 2] = [Self::Ordinary, Self::Headless];

    /// The daemon's `--lifetime` startup argument value.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ordinary => "ordinary",
            Self::Headless => "headless",
        }
    }
}

impl FromStr for Lifetime {
    type Err = ContractError;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|lifetime| lifetime.name() == name)
            .ok_or_else(|| ContractError("unknown session lifetime".into()))
    }
}
