//! Capability families: the tokens a peer requires, permits or advertises.
use super::FrameLimit;
use crate::{
    ContractError,
    hello::Advertised,
    json::Object,
    names::{name, named},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// A revision-1 capability family. Support is advertised, never inferred
/// from a release.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Family {
    /// Core: control, readiness, viewing, leave, final results and kill.
    DaemonWireV1,
    /// Passive monitor entry, successor registration and promotion.
    MonitorV1,
    TitleV1,
    StatusV1,
    /// Requires `status-v1`.
    NotificationV1,
    ProgressV1,
    /// Optional non-user application focus.
    FocusV1,
}

const FAMILIES: [(Family, &str); 7] = [
    (Family::DaemonWireV1, "daemon-wire-v1"),
    (Family::MonitorV1, "monitor-v1"),
    (Family::TitleV1, "title-v1"),
    (Family::StatusV1, "status-v1"),
    (Family::NotificationV1, "notification-v1"),
    (Family::ProgressV1, "progress-v1"),
    (Family::FocusV1, "focus-v1"),
];

impl Family {
    pub fn token(self) -> &'static str {
        name(&FAMILIES, self)
    }

    /// Tokens are case-sensitive; an unknown token is not a family.
    pub fn from_token(text: &str) -> Option<Self> {
        named(&FAMILIES, text)
    }
}

/// The daemon's maximum request and server frame lengths, each counting the
/// type byte and body.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct FrameLimits {
    pub max_request_frame_bytes: FrameLimit,
    pub max_server_frame_bytes: FrameLimit,
}

/// Revision-1 support advertised in hello: `daemon-wire-v1`, the other
/// families and the `daemon_wire_v1` frame limits.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Advertisement {
    families: BTreeSet<Family>,
    limits: FrameLimits,
}

impl Advertisement {
    /// `daemon-wire-v1` is always part of the advertisement.
    pub fn new(families: impl IntoIterator<Item = Family>, limits: FrameLimits) -> Self {
        let mut families: BTreeSet<_> = families.into_iter().collect();
        families.insert(Family::DaemonWireV1);
        Self { families, limits }
    }

    pub fn supports(&self, family: Family) -> bool {
        self.families.contains(&family)
    }

    pub fn families(&self) -> impl Iterator<Item = Family> + '_ {
        self.families.iter().copied()
    }

    pub fn limits(&self) -> FrameLimits {
        self.limits
    }

    pub(crate) fn tokens(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.families().map(Family::token)
    }

    /// Both the token and valid limits make revision 1 available. A missing
    /// or malformed advertisement makes it unavailable, never the session
    /// stale. Hello stays lenient: unknown tokens and members are ignored.
    pub(crate) fn advertised(reply: &Value) -> Advertised<Self> {
        let families = advertised_families(reply);
        let limits = reply.get("daemon_wire_v1").map(frame_limits);
        match (families.contains(&Family::DaemonWireV1), limits) {
            (false, None) => Advertised::Absent,
            (true, Some(Some(limits))) => Advertised::Valid(Self { families, limits }),
            _ => Advertised::Malformed,
        }
    }
}

fn advertised_families(reply: &Value) -> BTreeSet<Family> {
    let tokens = reply
        .get("capabilities")
        .and_then(|tokens| Vec::<String>::deserialize(tokens).ok());
    tokens
        .into_iter()
        .flatten()
        .filter_map(|text| Family::from_token(&text))
        .collect()
}

fn frame_limits(value: &Value) -> Option<FrameLimits> {
    Object::<FrameLimits>::deserialize(value)
        .ok()
        .map(|Object(limits)| limits)
}

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
