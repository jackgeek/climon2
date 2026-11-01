//! Client key vocabulary and the validated client-settings snapshot (plan 1.3).
//! The vocabulary is exactly the 32 C0 control bytes, named `Ctrl+Space`,
//! `Ctrl+A`..`Ctrl+Z`, `Ctrl+[`, `Ctrl+\`, `Ctrl+]`, `Ctrl+^` and `Ctrl+_`.
use std::fmt;

const ESCAPE: ControlKey = ControlKey(0x1B);

/// One single-byte control key from the configurable vocabulary.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ControlKey(u8);

impl ControlKey {
    pub fn from_byte(byte: u8) -> Option<Self> {
        (byte < 0x20).then_some(Self(byte))
    }

    /// Accepts only the exact spellings; `Ctrl+@` is not an alias of Space.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.strip_prefix("Ctrl+")?.as_bytes() {
            b"Space" => Some(Self(0x00)),
            [caret @ b'A'..=b'_'] => Some(Self(caret - 0x40)),
            _ => None,
        }
    }

    pub fn byte(self) -> u8 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsError {
    TakeControlIsEscape,
    Conflict,
}

impl fmt::Display for SettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TakeControlIsEscape => "take_control_key may not be Ctrl+[",
            Self::Conflict => "escape_prefix and take_control_key are the same key",
        })
    }
}

impl std::error::Error for SettingsError {}

/// A validated client-settings snapshot; `None` disables that binding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientSettings {
    escape_prefix: Option<ControlKey>,
    take_control_key: Option<ControlKey>,
}

impl ClientSettings {
    pub fn new(
        escape_prefix: Option<ControlKey>,
        take_control_key: Option<ControlKey>,
    ) -> Result<Self, SettingsError> {
        if take_control_key == Some(ESCAPE) {
            return Err(SettingsError::TakeControlIsEscape);
        }
        if escape_prefix.is_some() && escape_prefix == take_control_key {
            return Err(SettingsError::Conflict);
        }
        Ok(Self {
            escape_prefix,
            take_control_key,
        })
    }

    pub fn escape_prefix(&self) -> Option<ControlKey> {
        self.escape_prefix
    }

    pub fn take_control_key(&self) -> Option<ControlKey> {
        self.take_control_key
    }
}

impl Default for ClientSettings {
    /// Ctrl+] escapes and Ctrl+T takes control.
    fn default() -> Self {
        Self {
            escape_prefix: Some(ControlKey(0x1D)),
            take_control_key: Some(ControlKey(0x14)),
        }
    }
}
