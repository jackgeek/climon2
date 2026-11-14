//! Optional hello extensions (plan 1.11). The mandatory reply stays strict and
//! frozen (`protocol::HelloReply`); these fields are read leniently, so bad
//! optional data is reported to the caller but never makes a session stale.
//! Additively frozen: the tokens, the state fields and the lifecycle names.
use crate::{
    json::Object,
    names,
    protocol::{self, Frame, HelloReply},
    wire::{Advertisement, Family, FrameLimits},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, io};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Capability {
    Attach,
    Kill,
    StateV1,
    ClientStartupV1,
}

const TOKENS: [(Capability, &str); 4] = [
    (Capability::Attach, "attach"),
    (Capability::Kill, "kill"),
    (Capability::StateV1, "state-v1"),
    (Capability::ClientStartupV1, "client-startup-v1"),
];

impl Capability {
    /// The frozen tokens, which lead the capabilities this daemon advertises.
    /// Never `nesting-v1`.
    pub const ADVERTISED: [Self; 4] = [
        Self::Attach,
        Self::Kill,
        Self::StateV1,
        Self::ClientStartupV1,
    ];

    pub fn token(self) -> &'static str {
        names::name(&TOKENS, self)
    }

    fn from_token(token: &str) -> Option<Self> {
        names::named(&TOKENS, token)
    }
}

/// The advertised capabilities that this version recognises.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Capabilities(BTreeSet<Capability>);

impl Capabilities {
    pub fn contains(&self, capability: Capability) -> bool {
        self.0.contains(&capability)
    }

    pub fn supports_attach(&self) -> bool {
        self.contains(Capability::Attach) && self.contains(Capability::ClientStartupV1)
    }

    pub fn supports_kill(&self) -> bool {
        self.contains(Capability::Kill) && self.contains(Capability::ClientStartupV1)
    }

    pub fn supports_state(&self) -> bool {
        self.contains(Capability::StateV1)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    /// An ordinary session awaiting its initial attach.
    Starting,
    Attached,
    /// No attachment while the session keeps running: after an accepted
    /// explicit detach left none, or whenever a headless session has none.
    Detached,
    Terminating,
}

const LIFECYCLES: [(Lifecycle, &str); 4] = [
    (Lifecycle::Starting, "starting"),
    (Lifecycle::Attached, "attached"),
    (Lifecycle::Detached, "detached"),
    (Lifecycle::Terminating, "terminating"),
];

impl Lifecycle {
    pub fn name(self) -> &'static str {
        names::name(&LIFECYCLES, self)
    }

    fn from_name(name: &str) -> Option<Self> {
        names::named(&LIFECYCLES, name)
    }
}

/// One consistent point-in-time snapshot; `attachments` counts accepted
/// attachments only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionState {
    lifecycle: Lifecycle,
    attachments: u32,
    controlled: bool,
}

impl SessionState {
    pub fn new(lifecycle: Lifecycle, attachments: u32, controlled: bool) -> Option<Self> {
        consistent(lifecycle, attachments > 0, controlled).then_some(Self {
            lifecycle,
            attachments,
            controlled,
        })
    }

    pub fn lifecycle(&self) -> Lifecycle {
        self.lifecycle
    }

    pub fn attachments(&self) -> u32 {
        self.attachments
    }

    pub fn controlled(&self) -> bool {
        self.controlled
    }
}

fn consistent(lifecycle: Lifecycle, attached: bool, controlled: bool) -> bool {
    match lifecycle {
        Lifecycle::Starting | Lifecycle::Detached => (attached, controlled) == (false, false),
        Lifecycle::Attached => (attached, controlled) == (true, true),
        Lifecycle::Terminating => attached || !controlled,
    }
}

/// One optional field: absent, valid, or present but unusable (log it).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Advertised<T> {
    Absent,
    Valid(T),
    Malformed,
}

impl<T> Advertised<T> {
    pub fn valid(&self) -> Option<&T> {
        match self {
            Self::Valid(value) => Some(value),
            _ => None,
        }
    }

    pub fn is_malformed(&self) -> bool {
        matches!(self, Self::Malformed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HelloExtensions {
    pub capabilities: Advertised<Capabilities>,
    pub state: Advertised<SessionState>,
    /// Daemon wire revision 1, available only when valid.
    pub daemon_wire_v1: Advertised<Advertisement>,
}

impl HelloExtensions {
    /// A body that is not JSON has no extensions; the mandatory parse fails it.
    pub fn from_body(body: &[u8]) -> Self {
        Self::from_value(&serde_json::from_slice(body).unwrap_or(Value::Null))
    }

    pub fn from_value(reply: &Value) -> Self {
        Self {
            capabilities: advertised(reply, "capabilities", capabilities),
            state: advertised(reply, "state", state),
            daemon_wire_v1: revision1(reply),
        }
    }
}

fn advertised<T>(reply: &Value, field: &str, parse: fn(&Value) -> Option<T>) -> Advertised<T> {
    match reply.get(field) {
        None => Advertised::Absent,
        Some(value) => parse(value).map_or(Advertised::Malformed, Advertised::Valid),
    }
}

/// The tokens of a capability array that `from_token` knows; others are
/// ignored.
fn known<T: Ord>(tokens: &Value, from_token: fn(&str) -> Option<T>) -> Option<BTreeSet<T>> {
    let tokens = Vec::<String>::deserialize(tokens).ok()?;
    Some(
        tokens
            .iter()
            .filter_map(|token| from_token(token))
            .collect(),
    )
}

fn capabilities(value: &Value) -> Option<Capabilities> {
    known(value, Capability::from_token).map(Capabilities)
}

/// Both the `daemon-wire-v1` token and valid limits make revision 1
/// available. A missing or malformed advertisement makes it unavailable,
/// never the session stale; unknown tokens and members are ignored.
fn revision1(reply: &Value) -> Advertised<Advertisement> {
    let families = reply
        .get("capabilities")
        .and_then(|tokens| known(tokens, Family::from_token))
        .unwrap_or_default();
    let limits = advertised(reply, "daemon_wire_v1", frame_limits);
    match (families.contains(&Family::DaemonWireV1), limits) {
        (false, Advertised::Absent) => Advertised::Absent,
        (true, Advertised::Valid(limits)) => {
            Advertised::Valid(Advertisement::new(families, limits))
        }
        _ => Advertised::Malformed,
    }
}

fn frame_limits(value: &Value) -> Option<FrameLimits> {
    Object::<FrameLimits>::deserialize(value)
        .ok()
        .map(|Object(limits)| limits)
}

#[derive(Deserialize)]
struct RawState {
    lifecycle: String,
    attachments: u32,
    controlled: bool,
}

fn state(value: &Value) -> Option<SessionState> {
    let Object(raw) = Object::<RawState>::deserialize(value).ok()?;
    SessionState::new(
        Lifecycle::from_name(&raw.lifecycle)?,
        raw.attachments,
        raw.controlled,
    )
}

#[derive(Serialize)]
struct AdvertisedHello<'a> {
    #[serde(flatten)]
    reply: &'a HelloReply,
    capabilities: Vec<&'static str>,
    state: StateBody,
    daemon_wire_v1: FrameLimits,
}

#[derive(Serialize)]
struct StateBody {
    lifecycle: &'static str,
    attachments: u32,
    controlled: bool,
}

/// The daemon's hello reply: the frozen mandatory fields, the frozen tokens
/// followed by its revision-1 families, the current state snapshot, then
/// `daemon_wire_v1`. No reply can omit the tokens and state that admission
/// and `list` rely on.
pub fn advertise_revision1(
    reply: &HelloReply,
    state: SessionState,
    revision1: &Advertisement,
) -> io::Result<Frame> {
    let frozen = Capability::ADVERTISED.map(Capability::token);
    let families = revision1.tokens();
    protocol::control(
        protocol::HELLO,
        &AdvertisedHello {
            reply,
            capabilities: frozen.into_iter().chain(families).collect(),
            state: StateBody {
                lifecycle: state.lifecycle.name(),
                attachments: state.attachments,
                controlled: state.controlled,
            },
            daemon_wire_v1: revision1.limits(),
        },
    )
}
