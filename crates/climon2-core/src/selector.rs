//! Session selectors for attach and kill (plan 1.2): a full GUID or a unique
//! prefix of at least 1 hex digit, case-insensitive, hyphens ignored.
use crate::{SessionId, cli::UsageError};
use std::{ffi::OsStr, fmt};

/// A normalised selector: 1..=32 lowercase hex digits, no hyphens.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Selector(String);

impl Selector {
    pub fn parse(value: &OsStr) -> Result<Self, UsageError> {
        let text = value.to_str().ok_or(UsageError)?;
        let digits: String = text.chars().filter(|c| *c != '-').collect();
        let valid =
            (1..=32).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit());
        valid
            .then(|| Self(digits.to_ascii_lowercase()))
            .ok_or(UsageError)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn matches(&self, id: &SessionId) -> bool {
        id.as_str().replace('-', "").starts_with(&self.0)
    }

    /// Exactly one candidate must match; never narrow the candidate set first.
    pub fn select<'a, T>(
        &self,
        candidates: &'a [T],
        id: impl Fn(&T) -> &SessionId,
    ) -> Result<&'a T, SelectionError> {
        let found: Vec<&T> = candidates
            .iter()
            .filter(|candidate| self.matches(id(candidate)))
            .collect();
        match found.as_slice() {
            [only] => Ok(only),
            [] => Err(SelectionError::NoMatch {
                selector: self.clone(),
            }),
            many => Err(SelectionError::Ambiguous {
                selector: self.clone(),
                matches: many.iter().map(|candidate| id(candidate).clone()).collect(),
            }),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SelectionError {
    NoMatch {
        selector: Selector,
    },
    Ambiguous {
        selector: Selector,
        matches: Vec<SessionId>,
    },
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMatch { selector } => {
                write!(formatter, "no live session matches {}", selector.as_str())
            }
            Self::Ambiguous { selector, matches } => write!(
                formatter,
                "session selector {} is ambiguous; it matches {}",
                selector.as_str(),
                matches
                    .iter()
                    .map(SessionId::as_str)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for SelectionError {}
