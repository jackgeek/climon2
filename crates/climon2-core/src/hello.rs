//! Optional hello extensions (plan 1.11). The mandatory reply stays strict and
//! frozen (`protocol::HelloReply`); these fields are read leniently, so bad
//! optional data is reported to the caller but never makes a session stale.
//! Additively frozen: the tokens, the state fields and the lifecycle names.
use crate::{
    json::Object,
    protocol::{self, Frame, HelloReply},
    wire::{Advertisement, FrameLimits},
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
    /// Everything this daemon advertises. Never `nesting-v1`.
    pub const ADVERTISED: [Self; 4] = [
        Self::Attach,
        Self::Kill,
        Self::StateV1,
        Self::ClientStartupV1,
    ];

    pub fn token(self) -> &'static str {
        lookup(&TOKENS, |(capability, _)| *capability == self).1
    }

    fn from_token(token: &str) -> Option<Self> {
        find(&TOKENS, |(_, name)| *name == token).map(|(capability, _)| capability)
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
    /// Awaiting the initial attach.
    Starting,
    Attached,
    /// After an accepted explicit detach left no attachment.
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
        lookup(&LIFECYCLES, |(lifecycle, _)| *lifecycle == self).1
    }

    fn from_name(name: &str) -> Option<Self> {
        find(&LIFECYCLES, |(_, known)| *known == name).map(|(lifecycle, _)| lifecycle)
    }
}

fn find<T: Copy>(table: &[T], wanted: impl Fn(&T) -> bool) -> Option<T> {
    table.iter().copied().find(wanted)
}

/// Tables cover every variant, so a lookup by variant always succeeds.
fn lookup<T: Copy>(table: &[T], wanted: impl Fn(&T) -> bool) -> T {
    find(table, wanted).expect("table covers every variant")
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
            daemon_wire_v1: Advertisement::advertised(reply),
        }
    }
}

fn advertised<T>(reply: &Value, field: &str, parse: fn(&Value) -> Option<T>) -> Advertised<T> {
    match reply.get(field) {
        None => Advertised::Absent,
        Some(value) => parse(value).map_or(Advertised::Malformed, Advertised::Valid),
    }
}

fn capabilities(value: &Value) -> Option<Capabilities> {
    let tokens = Vec::<String>::deserialize(value).ok()?;
    let known = tokens
        .iter()
        .filter_map(|token| Capability::from_token(token));
    Some(Capabilities(known.collect()))
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
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<StateBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    daemon_wire_v1: Option<FrameLimits>,
}

#[derive(Serialize)]
struct StateBody {
    lifecycle: &'static str,
    attachments: u32,
    controlled: bool,
}

/// The daemon's hello reply: the frozen mandatory fields plus every advertised
/// capability and the current state snapshot.
pub fn advertise(reply: &HelloReply, state: SessionState) -> io::Result<Frame> {
    advertise_with(reply, &Capability::ADVERTISED, Some(state), None)
}

/// A hello reply with chosen optional advertisements. Revision-1 families
/// follow the frozen tokens; a daemon adds them only once it speaks revision 1.
pub fn advertise_with(
    reply: &HelloReply,
    capabilities: &[Capability],
    state: Option<SessionState>,
    daemon_wire_v1: Option<&Advertisement>,
) -> io::Result<Frame> {
    let frozen = capabilities.iter().map(|capability| capability.token());
    let revision1 = daemon_wire_v1.into_iter().flat_map(Advertisement::tokens);
    protocol::control(
        protocol::HELLO,
        &AdvertisedHello {
            reply,
            capabilities: frozen.chain(revision1).collect(),
            state: state.map(|state| StateBody {
                lifecycle: state.lifecycle.name(),
                attachments: state.attachments,
                controlled: state.controlled,
            }),
            daemon_wire_v1: daemon_wire_v1.map(Advertisement::limits),
        },
    )
}
