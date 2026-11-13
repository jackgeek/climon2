//! The monitor preference file, `<climon2 home>/monitor-preferences.json`
//! (spec #73, Introduction preference): the key introduction's **Don't show
//! again** choice, kept apart from `config.jsonc`. A release writes version 1,
//! `{"version":1,"suppress_key_introduction":<bool>}`. The schema is additive:
//! later versions only add fields, so every release reads a file of any
//! version, ignoring the fields it does not know.
use crate::json;
use serde::Deserialize;
use std::{fmt, num::NonZeroU32};

/// The file's name in a climon2 home.
pub const FILE_NAME: &str = "monitor-preferences.json";

/// The saved preferences.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MonitorPreferences {
    suppress_key_introduction: bool,
}

/// Why a file's bytes are not a preference file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MalformedPreferences(String);

impl fmt::Display for MalformedPreferences {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{FILE_NAME} is malformed: {}", self.0)
    }
}

impl std::error::Error for MalformedPreferences {}

/// What every version holds. Any version is read; serde ignores the fields
/// later versions add.
#[derive(Deserialize)]
struct File {
    #[serde(rename = "version")]
    _version: NonZeroU32,
    suppress_key_introduction: bool,
}

impl MonitorPreferences {
    pub fn new(suppress_key_introduction: bool) -> Self {
        Self {
            suppress_key_introduction,
        }
    }

    /// Whether the user saved **Don't show again**.
    pub fn suppresses_key_introduction(self) -> bool {
        self.suppress_key_introduction
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, MalformedPreferences> {
        json::strict_object::<File>(bytes)
            .map(|file| Self::new(file.suppress_key_introduction))
            .map_err(|error| MalformedPreferences(error.to_string()))
    }

    /// The file this release writes: version 1, one line.
    pub fn to_json(self) -> Vec<u8> {
        format!(
            "{{\"version\":1,\"suppress_key_introduction\":{}}}\n",
            self.suppress_key_introduction
        )
        .into_bytes()
    }
}
