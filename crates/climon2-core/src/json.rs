use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, MapAccess, Visitor, value::MapAccessDeserializer},
};
use std::{fmt, marker::PhantomData};

pub(crate) fn from_object<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    let first = bytes.iter().find(|byte| !byte.is_ascii_whitespace());
    if first != Some(&b'{') {
        return Err(serde::de::Error::custom("expected a JSON object"));
    }
    serde_json::from_slice(bytes)
}

/// A JSON object, well-formed throughout. serde skips unknown values leniently
/// (it accepts `[1,]`), so a strict syntax pass comes first.
pub(crate) fn strict_object<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_slice::<serde_json::Value>(bytes)?;
    serde_json::from_slice::<Object<T>>(bytes).map(|Object(value)| value)
}

/// A known object accepted only as a JSON object. Derived structs would also
/// accept a positional array, which is the wrong type for a named object.
pub(crate) struct Object<T>(pub(crate) T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Object<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer
            .deserialize_map(ObjectVisitor(PhantomData))
            .map(Self)
    }
}

struct ObjectVisitor<T>(PhantomData<T>);

impl<'de, T: Deserialize<'de>> Visitor<'de> for ObjectVisitor<T> {
    type Value = T;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<T, A::Error> {
        T::deserialize(MapAccessDeserializer::new(map))
    }
}

/// The three states of an optional JSON field. Use with `#[serde(default)]`.
#[derive(Default)]
pub(crate) enum Field<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Field<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::deserialize(deserializer).map(|value| value.map_or(Self::Null, Self::Value))
    }
}
