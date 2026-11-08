//! Requests: client to session daemon, types `0x30` through `0x39`.
use super::{
    Counter, Families, FatalCode, Frame, Geometry, Requested,
    record::{self, ControlRecord, Decoder, nullable},
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

impl ControlRecord for Open {
    const CODE: u8 = 0x30;
    const REQUEST_ID: bool = true;
}

impl ControlRecord for Leave {
    const CODE: u8 = 0x37;
    const REQUEST_ID: bool = true;
}

impl ControlRecord for Kill {
    const CODE: u8 = 0x38;
    const REQUEST_ID: bool = true;
}

/// Every request the session daemon decodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Request {
    Open(Open),
    Leave(Leave),
    Kill(Kill),
}

const DECODERS: [(u8, Decoder<Request>); 3] = [
    (Open::CODE, |body| record::decode(body).map(Request::Open)),
    (Leave::CODE, |body| record::decode(body).map(Request::Leave)),
    (Kill::CODE, |body| record::decode(body).map(Request::Kill)),
];

impl Request {
    /// Decode a frame the session daemon received.
    pub fn from_frame(frame: &Frame) -> Result<Self, FatalCode> {
        record::dispatch(frame, &DECODERS, record::SERVER_TYPES)
    }
}
