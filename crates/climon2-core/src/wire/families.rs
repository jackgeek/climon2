//! Capability families: the tokens a peer requires, permits or advertises.
use crate::ContractError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A capability array: unique, case-sensitive tokens in the order sent.
/// Unknown tokens are kept for the receiver to judge.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Vec<String>", into = "Vec<String>")]
pub struct Families(Vec<String>);

impl Families {
    pub fn new(tokens: Vec<String>) -> Option<Self> {
        let unique: BTreeSet<&String> = tokens.iter().collect();
        (unique.len() == tokens.len()).then_some(Self(tokens))
    }

    pub fn tokens(&self) -> &[String] {
        &self.0
    }

    fn overlaps(&self, other: &Self) -> bool {
        self.0.iter().any(|token| other.0.contains(token))
    }
}

impl TryFrom<Vec<String>> for Families {
    type Error = ContractError;

    fn try_from(tokens: Vec<String>) -> Result<Self, Self::Error> {
        Self::new(tokens).ok_or_else(|| ContractError("repeated capability".into()))
    }
}

impl From<Families> for Vec<String> {
    fn from(families: Families) -> Self {
        families.0
    }
}

/// Open's required and optional capability arrays, which never overlap.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct Requested {
    required: Families,
    optional: Families,
}

impl Requested {
    pub fn new(required: Families, optional: Families) -> Option<Self> {
        (!required.overlaps(&optional)).then_some(Self { required, optional })
    }

    pub fn required(&self) -> &Families {
        &self.required
    }

    pub fn optional(&self) -> &Families {
        &self.optional
    }
}
