//! `<home>/config.jsonc` (plan 1.3). A lexical normaliser turns JSONC into JSON
//! (comments become whitespace, trailing commas are dropped), then serde reads
//! operation-specific structs, so attach never validates scrollback it ignores.
mod limits;

use crate::{
    json::{self, Field, Object},
    keys::{ClientSettings, ControlKey, SettingsError},
    layout::HomeLayout,
};
pub use limits::{DiscoveryLimits, Lanes, Limits};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
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

/// The integers a setting accepts, `min` through `max`. `scrollback.max_rows`
/// and every `monitor.limits` setting check their ranges here, so a refusal
/// always has one shape and names its setting.
#[derive(Clone, Copy)]
struct Range {
    min: u64,
    max: u64,
}

impl Range {
    /// Why `setting` may not hold `value`, if it may not.
    fn refusal(self, setting: &str, value: u64) -> Option<String> {
        let Self { min, max } = self;
        (!(min..=max).contains(&value))
            .then(|| format!("{setting} must be an integer from {min} to {max}"))
    }
}

const MAX_ROWS: &str = "scrollback.max_rows";
const MAX_ROWS_RANGE: Range = Range {
    min: 0,
    max: 100_000,
};

/// Retained history rows, 0..=100000; zero disables retained history only.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaxRows(u32);

impl MaxRows {
    pub fn new(rows: u32) -> Option<Self> {
        Self::refusal(rows).is_none().then_some(Self(rows))
    }

    pub fn get(self) -> u32 {
        self.0
    }

    fn refusal(rows: u32) -> Option<String> {
        MAX_ROWS_RANGE.refusal(MAX_ROWS, rows.into())
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
        Self::refusal(rows).map_or(Ok(Self(rows)), |error| Err(D::Error::custom(error)))
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

/// The `monitor` section, which `exec` validates too.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct MonitorSettings {
    /// Whether implicit key hints show; bindings never change.
    pub show_key_hints: bool,
    /// Whether a toast rings the host bell as it first draws; only reported
    /// until toasts read it.
    bell: bool,
    #[serde(deserialize_with = "object")]
    pub limits: Limits,
}

impl Default for MonitorSettings {
    fn default() -> Self {
        Self {
            show_key_hints: true,
            bell: true,
            limits: Limits::default(),
        }
    }
}

const NULL_PREFIX: &str = "client.escape_prefix may not be null for climon2 monitor";

/// Everything the monitor validates before it changes the host terminal (spec
/// #73): the client bindings, with the escape prefix it needs to reach its
/// actions, and the `monitor` section. It is read once; nothing reloads it.
/// `Display` writes the effective values as JSON in the file's own shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct MonitorConfig {
    client: MonitorKeys,
    monitor: MonitorSettings,
}

/// The client bindings as the monitor reports them; it acts only on the
/// escape prefix so far.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct MonitorKeys {
    #[serde(serialize_with = "key_name")]
    escape_prefix: ControlKey,
    #[serde(serialize_with = "optional_key_name")]
    take_control_key: Option<ControlKey>,
}

impl MonitorConfig {
    pub fn parse(text: &[u8]) -> Result<Self, ConfigError> {
        let file = parse::<MonitorFile>(text)?;
        let escape_prefix = file
            .client
            .escape_prefix()
            .ok_or_else(|| ConfigError::Invalid(NULL_PREFIX.into()))?;
        Ok(Self {
            client: MonitorKeys {
                escape_prefix,
                take_control_key: file.client.take_control_key(),
            },
            monitor: file.monitor,
        })
    }

    pub fn escape_prefix(&self) -> ControlKey {
        self.client.escape_prefix
    }

    /// The take-control key every library instance receives, if enabled.
    pub fn take_control_key(&self) -> Option<ControlKey> {
        self.client.take_control_key
    }

    pub fn settings(&self) -> &MonitorSettings {
        &self.monitor
    }
}

impl fmt::Display for MonitorConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let json = serde_json::to_string(self).map_err(|_| fmt::Error)?;
        formatter.write_str(&json)
    }
}

fn key_name<S: Serializer>(key: &ControlKey, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(key)
}

fn optional_key_name<S: Serializer>(
    key: &Option<ControlKey>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    key.map(|key| key.to_string()).serialize(serializer)
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

    pub fn monitor(&self) -> Result<MonitorConfig, ConfigError> {
        self.parse(MonitorConfig::parse)
    }

    /// A missing file reads as an empty object: every setting its default.
    fn parse<T>(&self, parse: fn(&[u8]) -> Result<T, ConfigError>) -> Result<T, ConfigError> {
        parse(self.0.as_deref().unwrap_or(b"{}"))
    }
}

/// Exec validates the `monitor` section and passes none of it on.
#[derive(Deserialize)]
struct ExecFile {
    #[serde(default, deserialize_with = "client")]
    client: ClientSettings,
    #[serde(default, deserialize_with = "object")]
    scrollback: ScrollbackSettings,
    #[serde(default, rename = "monitor", deserialize_with = "object")]
    _monitor: MonitorSettings,
}

/// Unnamed `scrollback` and `monitor` are unknown here, so serde ignores them
/// entirely.
#[derive(Deserialize)]
struct AttachFile {
    #[serde(default, deserialize_with = "client")]
    client: ClientSettings,
}

/// Unnamed `scrollback` is unknown here: the monitor never changes it.
#[derive(Deserialize)]
struct MonitorFile {
    #[serde(default, deserialize_with = "client")]
    client: ClientSettings,
    #[serde(default, deserialize_with = "object")]
    monitor: MonitorSettings,
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
