//! The schema-1 `CLIMON2_CLIENT_STARTUP` envelope (plan 1.12, ADR 0006): the
//! client's operation and, for attach, its exact validated settings snapshot.
//! Additively frozen: never change the meaning of a field or operation.
use crate::{
    json::{self, Field, Object},
    keys::{ClientSettings, ControlKey},
};
use serde::Deserialize;
use std::fmt;

pub const ENVIRONMENT_VARIABLE: &str = "CLIMON2_CLIENT_STARTUP";
pub const MAX_ENVELOPE_LENGTH: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StartupEnvelope {
    Attach(ClientSettings),
    Kill,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EnvelopeError(String);

impl fmt::Display for EnvelopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid client startup envelope: {}", self.0)
    }
}

impl std::error::Error for EnvelopeError {}

fn error(detail: impl ToString) -> EnvelopeError {
    EnvelopeError(detail.to_string())
}

impl StartupEnvelope {
    /// A present envelope is used exactly or rejected; it never falls back.
    pub fn parse(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        if bytes.len() > MAX_ENVELOPE_LENGTH {
            return Err(error("larger than 4096 bytes"));
        }
        let raw: RawEnvelope = json::strict_object(bytes).map_err(error)?;
        if raw.schema != 1 {
            return Err(error("schema must be 1"));
        }
        raw.envelope()
    }

    pub fn to_json(&self) -> String {
        match self {
            Self::Attach(settings) => format!(
                r#"{{"schema":1,"operation":"attach","settings":{{"escape_prefix":{},"take_control_key":{}}}}}"#,
                byte(settings.escape_prefix()),
                byte(settings.take_control_key())
            ),
            Self::Kill => r#"{"schema":1,"operation":"kill"}"#.to_owned(),
        }
    }
}

fn byte(key: Option<ControlKey>) -> String {
    key.map_or_else(|| "null".to_owned(), |key| key.byte().to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Operation {
    Attach,
    Kill,
}

#[derive(Deserialize)]
struct RawEnvelope {
    schema: u64,
    operation: Operation,
    #[serde(default)]
    settings: Field<Object<RawSettings>>,
}

impl RawEnvelope {
    fn envelope(self) -> Result<StartupEnvelope, EnvelopeError> {
        match (self.operation, self.settings) {
            (Operation::Attach, Field::Value(Object(raw))) => {
                raw.settings().map(StartupEnvelope::Attach)
            }
            (Operation::Kill, Field::Missing) => Ok(StartupEnvelope::Kill),
            (Operation::Attach, _) => Err(error("attach requires settings")),
            (Operation::Kill, _) => Err(error("kill must not carry settings")),
        }
    }
}

#[derive(Deserialize)]
struct RawSettings {
    #[serde(default)]
    escape_prefix: Field<u8>,
    #[serde(default)]
    take_control_key: Field<u8>,
}

impl RawSettings {
    /// Revalidates everything config validated; never fills from defaults.
    fn settings(self) -> Result<ClientSettings, EnvelopeError> {
        let escape_prefix = required(self.escape_prefix, "escape_prefix")?;
        let take_control_key = required(self.take_control_key, "take_control_key")?;
        ClientSettings::new(escape_prefix, take_control_key).map_err(error)
    }
}

fn required(field: Field<u8>, name: &str) -> Result<Option<ControlKey>, EnvelopeError> {
    match field {
        Field::Missing => Err(error(format!("settings is missing {name}"))),
        Field::Null => Ok(None),
        Field::Value(byte) => ControlKey::from_byte(byte)
            .map(Some)
            .ok_or_else(|| error(format!("{name} is not a control key"))),
    }
}
