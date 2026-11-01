//! `<home>/config.jsonc` (plan 1.3). A lexical normaliser turns JSONC into JSON
//! (comments become whitespace, trailing commas are dropped), then serde reads
//! operation-specific structs, so attach never validates scrollback it ignores.
use crate::{
    json::{self, Field, Object},
    keys::{ClientSettings, ControlKey, SettingsError},
    layout::HomeLayout,
};
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, Error as _},
};
use serde_json::error::Category;
use std::{fmt, io};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConfigError {
    /// The file exists but cannot be read.
    Unreadable(String),
    /// Not JSONC, including tokens split by a comment.
    Malformed(String),
    /// Duplicate known fields, wrong types, bad keys, conflicts or ranges.
    Invalid(String),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable(detail) => write!(formatter, "cannot read config.jsonc: {detail}"),
            Self::Malformed(detail) => write!(formatter, "config.jsonc is malformed: {detail}"),
            Self::Invalid(detail) => write!(formatter, "config.jsonc is invalid: {detail}"),
        }
    }
}

impl std::error::Error for ConfigError {}

const MAX_ROWS_LIMIT: u32 = 100_000;

/// Retained history rows, 0..=100000; zero disables retained history only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaxRows(u32);

impl MaxRows {
    pub fn new(rows: u32) -> Option<Self> {
        (rows <= MAX_ROWS_LIMIT).then_some(Self(rows))
    }

    pub fn get(self) -> u32 {
        self.0
    }
}

impl Default for MaxRows {
    fn default() -> Self {
        Self(10_000)
    }
}

impl<'de> Deserialize<'de> for MaxRows {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let rows = u32::deserialize(deserializer)?;
        Self::new(rows).ok_or_else(|| {
            D::Error::custom("scrollback.max_rows must be an integer from 0 to 100000")
        })
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct ScrollbackSettings {
    #[serde(default)]
    pub max_rows: MaxRows,
}

/// Everything `exec` validates before starting the daemon.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ExecConfig {
    pub client: ClientSettings,
    pub scrollback: ScrollbackSettings,
}

/// Everything `attach` validates: the client bindings only.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AttachConfig {
    pub client: ClientSettings,
}

impl ExecConfig {
    pub fn parse(text: &[u8]) -> Result<Self, ConfigError> {
        parse::<ExecFile>(text).map(|file| Self {
            client: file.client,
            scrollback: file.scrollback,
        })
    }
}

impl AttachConfig {
    pub fn parse(text: &[u8]) -> Result<Self, ConfigError> {
        parse::<AttachFile>(text).map(|file| Self {
            client: file.client,
        })
    }
}

/// The configuration file's bytes, read once per operation; `None` if absent.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigSource(Option<Vec<u8>>);

impl ConfigSource {
    pub fn read(home: &HomeLayout) -> Result<Self, ConfigError> {
        match std::fs::read(home.config()) {
            Ok(text) => Ok(Self(Some(text))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self(None)),
            Err(error) => Err(ConfigError::Unreadable(error.to_string())),
        }
    }

    pub fn exec(&self) -> Result<ExecConfig, ConfigError> {
        self.parse(ExecConfig::parse)
    }

    pub fn attach(&self) -> Result<AttachConfig, ConfigError> {
        self.parse(AttachConfig::parse)
    }

    fn parse<T: Default>(
        &self,
        parse: fn(&[u8]) -> Result<T, ConfigError>,
    ) -> Result<T, ConfigError> {
        self.0.as_deref().map_or_else(|| Ok(T::default()), parse)
    }
}

#[derive(Deserialize)]
struct ExecFile {
    #[serde(default, deserialize_with = "client")]
    client: ClientSettings,
    #[serde(default, deserialize_with = "object")]
    scrollback: ScrollbackSettings,
}

/// Unnamed `scrollback` is unknown here, so serde ignores it entirely.
#[derive(Deserialize)]
struct AttachFile {
    #[serde(default, deserialize_with = "client")]
    client: ClientSettings,
}

#[derive(Deserialize)]
struct RawClient {
    #[serde(default)]
    escape_prefix: Field<KeyName>,
    #[serde(default)]
    take_control_key: Field<KeyName>,
}

impl RawClient {
    fn settings(self) -> Result<ClientSettings, SettingsError> {
        let defaults = ClientSettings::default();
        ClientSettings::new(
            binding(self.escape_prefix, defaults.escape_prefix()),
            binding(self.take_control_key, defaults.take_control_key()),
        )
    }
}

fn binding(field: Field<KeyName>, default: Option<ControlKey>) -> Option<ControlKey> {
    match field {
        Field::Missing => default,
        Field::Null => None,
        Field::Value(KeyName(key)) => Some(key),
    }
}

struct KeyName(ControlKey);

impl<'de> Deserialize<'de> for KeyName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        ControlKey::from_name(&name)
            .map(Self)
            .ok_or_else(|| D::Error::custom(format!("invalid key string \"{name}\"")))
    }
}

fn object<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<T, D::Error> {
    Object::deserialize(deserializer).map(|Object(value)| value)
}

fn client<'de, D: Deserializer<'de>>(deserializer: D) -> Result<ClientSettings, D::Error> {
    let raw: RawClient = object(deserializer)?;
    raw.settings().map_err(D::Error::custom)
}

fn parse<T: DeserializeOwned>(text: &[u8]) -> Result<T, ConfigError> {
    let json = normalise(text)?;
    json::strict_object(&json).map_err(classify)
}

fn classify(error: serde_json::Error) -> ConfigError {
    match error.classify() {
        Category::Data => ConfigError::Invalid(error.to_string()),
        _ => ConfigError::Malformed(error.to_string()),
    }
}

/// Rewrites JSONC to JSON of the same length. Comment bytes become spaces
/// (newlines stay, so serde positions still match the file) and a trailing
/// comma becomes a space; strings, including their escapes, are untouched.
fn normalise(text: &[u8]) -> Result<Vec<u8>, ConfigError> {
    let mut lexer = Lexer {
        bytes: text.to_vec(),
        at: 0,
        comma: None,
    };
    while lexer.at < lexer.bytes.len() {
        lexer.step()?;
    }
    Ok(lexer.bytes)
}

struct Lexer {
    bytes: Vec<u8>,
    at: usize,
    /// A comma outside strings with only whitespace or comments after it.
    comma: Option<usize>,
}

impl Lexer {
    fn step(&mut self) -> Result<(), ConfigError> {
        match &self.bytes[self.at..] {
            [b'/', b'/', ..] => self.line_comment(),
            [b'/', b'*', ..] => self.block_comment(),
            [b'"', ..] => self.string(),
            _ => self.code(),
        }
    }

    fn line_comment(&mut self) -> Result<(), ConfigError> {
        let rest = &self.bytes[self.at..];
        let length = rest
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(rest.len());
        self.blank(self.at + length);
        Ok(())
    }

    fn block_comment(&mut self) -> Result<(), ConfigError> {
        let body = self.at + 2;
        let length = self.bytes[body..]
            .windows(2)
            .position(|pair| pair == b"*/")
            .ok_or_else(|| ConfigError::Malformed("unterminated comment".into()))?;
        self.blank(body + length + 2);
        Ok(())
    }

    fn blank(&mut self, end: usize) {
        for byte in &mut self.bytes[self.at..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
        self.at = end;
    }

    fn string(&mut self) -> Result<(), ConfigError> {
        self.comma = None;
        let mut at = self.at + 1;
        while at < self.bytes.len() && self.bytes[at] != b'"' {
            at += if self.bytes[at] == b'\\' { 2 } else { 1 };
        }
        self.at = at + 1;
        Ok(())
    }

    fn code(&mut self) -> Result<(), ConfigError> {
        match self.bytes[self.at] {
            b',' => self.comma = Some(self.at),
            b'}' | b']' => self.close(),
            b' ' | b'\t' | b'\n' | b'\r' => {}
            _ => self.comma = None,
        }
        self.at += 1;
        Ok(())
    }

    fn close(&mut self) {
        if let Some(comma) = self.comma.take() {
            self.bytes[comma] = b' ';
        }
    }
}
