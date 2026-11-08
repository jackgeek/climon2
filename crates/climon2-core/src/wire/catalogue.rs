//! The operational and connection-fatal error catalogues, and the shared
//! `Error` record.
use crate::{ContractError, json::Object};
use serde::{Deserialize, Serialize};
use std::str::FromStr;

/// The operational catalogue: why an identifiable request was rejected or
/// failed. The connection stays usable.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OperationalCode {
    UnsupportedRevision,
    UnsupportedCapability,
    InvalidState,
    InvalidArgument,
    InvalidGeometry,
    NotAttached,
    NotController,
    StalePeriod,
    StaleRestoration,
    NotReady,
    NotViewing,
    SessionTerminating,
    ResourceLimit,
    IoError,
    CleanupFailed,
    InternalError,
}

const OPERATIONAL: [(OperationalCode, &str); 16] = [
    (OperationalCode::UnsupportedRevision, "unsupported_revision"),
    (
        OperationalCode::UnsupportedCapability,
        "unsupported_capability",
    ),
    (OperationalCode::InvalidState, "invalid_state"),
    (OperationalCode::InvalidArgument, "invalid_argument"),
    (OperationalCode::InvalidGeometry, "invalid_geometry"),
    (OperationalCode::NotAttached, "not_attached"),
    (OperationalCode::NotController, "not_controller"),
    (OperationalCode::StalePeriod, "stale_period"),
    (OperationalCode::StaleRestoration, "stale_restoration"),
    (OperationalCode::NotReady, "not_ready"),
    (OperationalCode::NotViewing, "not_viewing"),
    (OperationalCode::SessionTerminating, "session_terminating"),
    (OperationalCode::ResourceLimit, "resource_limit"),
    (OperationalCode::IoError, "io_error"),
    (OperationalCode::CleanupFailed, "cleanup_failed"),
    (OperationalCode::InternalError, "internal_error"),
];

impl OperationalCode {
    pub fn as_str(self) -> &'static str {
        token(&OPERATIONAL, self)
    }
}

/// The connection-fatal catalogue. Each code ends the connection; delivering
/// it to the peer is best effort.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FatalCode {
    InvalidFrameLength,
    FrameTooLarge,
    TruncatedFrame,
    InvalidJson,
    InvalidRecord,
    InvalidRequestId,
    UnknownMessageType,
    WrongDirection,
    InternalError,
}

const FATAL: [(FatalCode, &str); 9] = [
    (FatalCode::InvalidFrameLength, "invalid_frame_length"),
    (FatalCode::FrameTooLarge, "frame_too_large"),
    (FatalCode::TruncatedFrame, "truncated_frame"),
    (FatalCode::InvalidJson, "invalid_json"),
    (FatalCode::InvalidRecord, "invalid_record"),
    (FatalCode::InvalidRequestId, "invalid_request_id"),
    (FatalCode::UnknownMessageType, "unknown_message_type"),
    (FatalCode::WrongDirection, "wrong_direction"),
    (FatalCode::InternalError, "internal_error"),
];

impl FatalCode {
    pub fn as_str(self) -> &'static str {
        token(&FATAL, self)
    }
}

/// A nonempty stable error code. An unknown code stays an explicit
/// non-success and never implies retry.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ErrorCode(String);

impl ErrorCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn operational(&self) -> Option<OperationalCode> {
        named(&OPERATIONAL, &self.0)
    }

    pub fn fatal(&self) -> Option<FatalCode> {
        named(&FATAL, &self.0)
    }
}

impl FromStr for ErrorCode {
    type Err = ContractError;

    fn from_str(code: &str) -> Result<Self, Self::Err> {
        (!code.is_empty())
            .then(|| Self(code.to_owned()))
            .ok_or_else(|| ContractError("empty error code".into()))
    }
}

impl TryFrom<String> for ErrorCode {
    type Error = ContractError;

    fn try_from(code: String) -> Result<Self, Self::Error> {
        code.parse()
    }
}

impl From<ErrorCode> for String {
    fn from(code: ErrorCode) -> Self {
        code.0
    }
}

impl From<OperationalCode> for ErrorCode {
    fn from(code: OperationalCode) -> Self {
        Self(code.as_str().to_owned())
    }
}

impl From<FatalCode> for ErrorCode {
    fn from(code: FatalCode) -> Self {
        Self(code.as_str().to_owned())
    }
}

/// A code with diagnostic text. The text is never terminal output.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(from = "Object<ErrorFields>")]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorFields {
    code: ErrorCode,
    message: String,
}

impl From<Object<ErrorFields>> for Error {
    fn from(Object(fields): Object<ErrorFields>) -> Self {
        Self {
            code: fields.code,
            message: fields.message,
        }
    }
}

/// Tables list every variant, so a lookup by variant always succeeds.
pub(crate) fn token<T: Copy + PartialEq>(table: &[(T, &'static str)], value: T) -> &'static str {
    table
        .iter()
        .find(|(known, _)| *known == value)
        .map(|(_, token)| *token)
        .expect("the table lists every variant")
}

pub(crate) fn named<T: Copy>(table: &[(T, &'static str)], text: &str) -> Option<T> {
    table
        .iter()
        .find(|(_, token)| *token == text)
        .map(|(value, _)| *value)
}
