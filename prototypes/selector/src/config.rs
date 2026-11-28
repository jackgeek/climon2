use serde::{
    Deserialize, Deserializer,
    de::{Error as _, MapAccess, Visitor},
};
use serde_json_lenient::Value;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub bell: bool,
    pub show_key_hints: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bell: true,
            show_key_hints: true,
        }
    }
}

impl Settings {
    pub fn parse(bytes: &[u8]) -> Result<Self, serde_json_lenient::Error> {
        serde_json_lenient::from_slice::<Value>(bytes)?;
        serde_json_lenient::from_slice::<File>(bytes).map(|file| file.0)
    }
}

struct File(Settings);

impl<'de> Deserialize<'de> for File {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(FileVisitor).map(Self)
    }
}

struct FileVisitor;
impl<'de> Visitor<'de> for FileVisitor {
    type Value = Settings;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Settings, A::Error> {
        let mut monitor = None;
        while let Some(key) = map.next_key::<String>()? {
            if key == "monitor" {
                if monitor.is_some() {
                    return Err(A::Error::duplicate_field("monitor"));
                }
                monitor = Some(map.next_value::<Settings>()?);
            } else {
                map.next_value::<Value>()?;
            }
        }
        Ok(monitor.unwrap_or_default())
    }
}

impl<'de> Deserialize<'de> for Settings {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(MonitorVisitor)
    }
}

struct MonitorVisitor;
impl<'de> Visitor<'de> for MonitorVisitor {
    type Value = Settings;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a monitor JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Settings, A::Error> {
        let mut bell = None;
        let mut hints = None;
        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "bell" => {
                    if bell.is_some() {
                        return Err(A::Error::duplicate_field("bell"));
                    }
                    bell = Some(map.next_value::<bool>()?);
                }
                "show_key_hints" => {
                    if hints.is_some() {
                        return Err(A::Error::duplicate_field("show_key_hints"));
                    }
                    hints = Some(map.next_value::<bool>()?);
                }
                _ => {
                    map.next_value::<Value>()?;
                }
            }
        }
        let defaults = Settings::default();
        Ok(Settings {
            bell: bell.unwrap_or(defaults.bell),
            show_key_hints: hints.unwrap_or(defaults.show_key_hints),
        })
    }
}
