//! Requests: client to session daemon, types `0x30` through `0x39`.
use super::{
    Counter, Families, FatalCode, Frame, Geometry, Input, Requested,
    record::{self, ControlRecord, Decoder, by_name, nullable},
};
use crate::ContractError;
use serde::{Deserialize, Serialize, Serializer};

/// The first role request after hello. It selects the revision and
/// capability families before any effect.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(try_from = "OpenFields")]
pub struct Open {
    pub request_id: Counter,
    pub revision: u32,
    pub capabilities: Requested,
    pub mode: Mode,
}

/// Monitor mode registers a passive monitor connection without geometry;
/// interactive mode attaches at the requested geometry.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Mode {
    Monitor,
    Interactive(Geometry),
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ModeName {
    Monitor,
    Interactive,
}

impl Mode {
    fn new(name: ModeName, geometry: Option<Geometry>) -> Option<Self> {
        match (name, geometry) {
            (ModeName::Monitor, None) => Some(Self::Monitor),
            (ModeName::Interactive, Some(geometry)) => Some(Self::Interactive(geometry)),
            _ => None,
        }
    }

    fn members(&self) -> (ModeName, Option<&Geometry>) {
        match self {
            Self::Monitor => (ModeName::Monitor, None),
            Self::Interactive(geometry) => (ModeName::Interactive, Some(geometry)),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OpenFields {
    request_id: Counter,
    revision: u32,
    required_capabilities: Families,
    optional_capabilities: Families,
    #[serde(deserialize_with = "by_name")]
    mode: ModeName,
    #[serde(deserialize_with = "nullable")]
    geometry: Option<Geometry>,
}

impl TryFrom<OpenFields> for Open {
    type Error = ContractError;

    fn try_from(fields: OpenFields) -> Result<Self, Self::Error> {
        let capabilities =
            Requested::new(fields.required_capabilities, fields.optional_capabilities);
        match (capabilities, Mode::new(fields.mode, fields.geometry)) {
            (Some(capabilities), Some(mode)) => Ok(Self {
                request_id: fields.request_id,
                revision: fields.revision,
                capabilities,
                mode,
            }),
            _ => Err(ContractError("invalid Open".into())),
        }
    }
}

#[derive(Serialize)]
struct OpenMembers<'a> {
    request_id: Counter,
    revision: u32,
    required_capabilities: &'a Families,
    optional_capabilities: &'a Families,
    mode: ModeName,
    geometry: Option<&'a Geometry>,
}

impl Serialize for Open {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (mode, geometry) = self.mode.members();
        OpenMembers {
            request_id: self.request_id,
            revision: self.revision,
            required_capabilities: self.capabilities.required(),
            optional_capabilities: self.capabilities.optional(),
            mode,
            geometry,
        }
        .serialize(serializer)
    }
}

/// Resize `0x32`: requested geometry under the current control period and
/// restoration, even before readiness. Acceptance is not the actual geometry,
/// which the Geometry event reports.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resize {
    pub request_id: Counter,
    pub period: Counter,
    pub restoration_id: Counter,
    pub geometry: Geometry,
}

/// Acquire `0x33`: an explicit request for control. While already
/// controlling it is a no-op that keeps the period and restoration.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acquire {
    pub request_id: Counter,
}

/// Ready `0x34`: the complete current restoration is applied, not merely
/// received.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ready {
    pub request_id: Counter,
    pub period: Counter,
    pub restoration_id: Counter,
}

/// Set viewing `0x35`: whether this attachment is viewing; true requires
/// readiness. It is a fact report, never an acknowledgement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetViewing {
    pub request_id: Counter,
    pub period: Counter,
    pub restoration_id: Counter,
    pub viewing: bool,
}

/// Acknowledge attention `0x36`: acknowledges whatever attention is pending.
/// It names no episode.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Acknowledge {
    pub request_id: Counter,
    pub period: Counter,
    pub restoration_id: Counter,
}

/// Accepted departure: removes a passive registration and any attachment.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Leave {
    pub request_id: Counter,
}

/// Standalone kill: a first role request that is never an attachment.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kill {
    pub request_id: Counter,
    pub revision: u32,
    pub required_capabilities: Families,
}

/// Focus `0x39`: the desired application focus, which needs `focus-v1`. It
/// is non-user traffic: never Input, never an acknowledgement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Focus {
    pub request_id: Counter,
    pub period: Counter,
    pub restoration_id: Counter,
    pub focused: bool,
}

impl ControlRecord for Open {
    const CODE: u8 = 0x30;
}

impl ControlRecord for Resize {
    const CODE: u8 = 0x32;
}

impl ControlRecord for Acquire {
    const CODE: u8 = 0x33;
}

impl ControlRecord for Ready {
    const CODE: u8 = 0x34;
}

impl ControlRecord for SetViewing {
    const CODE: u8 = 0x35;
}

impl ControlRecord for Acknowledge {
    const CODE: u8 = 0x36;
}

impl ControlRecord for Leave {
    const CODE: u8 = 0x37;
}

impl ControlRecord for Kill {
    const CODE: u8 = 0x38;
}

impl ControlRecord for Focus {
    const CODE: u8 = 0x39;
}

/// Every request the session daemon decodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    Open(Open),
    Input(Input),
    Resize(Resize),
    Acquire(Acquire),
    Ready(Ready),
    SetViewing(SetViewing),
    Acknowledge(Acknowledge),
    Leave(Leave),
    Kill(Kill),
    Focus(Focus),
}

const DECODERS: [(u8, Decoder<Request>); 10] = [
    (Open::CODE, |body| record::decode(body).map(Request::Open)),
    (Input::CODE, |body| Input::decode(body).map(Request::Input)),
    (Resize::CODE, |body| {
        record::decode(body).map(Request::Resize)
    }),
    (Acquire::CODE, |body| {
        record::decode(body).map(Request::Acquire)
    }),
    (Ready::CODE, |body| record::decode(body).map(Request::Ready)),
    (SetViewing::CODE, |body| {
        record::decode(body).map(Request::SetViewing)
    }),
    (Acknowledge::CODE, |body| {
        record::decode(body).map(Request::Acknowledge)
    }),
    (Leave::CODE, |body| record::decode(body).map(Request::Leave)),
    (Kill::CODE, |body| record::decode(body).map(Request::Kill)),
    (Focus::CODE, |body| record::decode(body).map(Request::Focus)),
];

impl Request {
    /// Decode a frame the session daemon received.
    pub fn from_frame(frame: &Frame) -> Result<Self, FatalCode> {
        record::dispatch(frame, &DECODERS, record::SERVER_TYPES)
    }
}
