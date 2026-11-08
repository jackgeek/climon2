//! Strict control records: one complete UTF-8 JSON object with no unknown,
//! duplicate or undocumented members, where every listed member is required,
//! even when it is null. Types whose wire form is an object decode only from
//! an object (`json::Object`); derived structs would also take an array.
use super::{Counter, FatalCode, Frame};
use crate::json::Object;
use serde::{Deserialize, Deserializer, Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::ops::RangeInclusive;

/// Server types, sent only by the session daemon.
pub(crate) const SERVER_TYPES: RangeInclusive<u8> = 0x40..=0x4d;

/// A JSON control record and its frame type.
pub trait ControlRecord: Serialize + DeserializeOwned {
    const CODE: u8;
    /// Records with a `request_id` report a malformed one as `invalid_request_id`.
    const REQUEST_ID: bool = false;

    fn to_frame(&self) -> Frame {
        let body = serde_json::to_vec(self).expect("control records always serialize");
        Frame::new(Self::CODE, body)
    }
}

/// Syntax is checked first, so broken JSON is never reported as a record error.
pub(crate) fn decode<T: ControlRecord>(body: &[u8]) -> Result<T, FatalCode> {
    let value: Value = serde_json::from_slice(body).map_err(|_| FatalCode::InvalidJson)?;
    if T::REQUEST_ID {
        request_id(&value)?;
    }
    serde_json::from_slice::<Object<T>>(body)
        .map(|Object(record)| record)
        .map_err(|_| FatalCode::InvalidRecord)
}

fn request_id(record: &Value) -> Result<(), FatalCode> {
    match record.get("request_id").map(Counter::deserialize) {
        Some(Err(_)) => Err(FatalCode::InvalidRequestId),
        _ => Ok(()),
    }
}

pub(crate) type Decoder<M> = fn(&[u8]) -> Result<M, FatalCode>;

/// A type documented only for the other direction is `wrong_direction`; any
/// other type without a decoder is `unknown_message_type`.
pub(crate) fn dispatch<M>(
    frame: &Frame,
    decoders: &[(u8, Decoder<M>)],
    opposite: RangeInclusive<u8>,
) -> Result<M, FatalCode> {
    match decoders.iter().find(|(code, _)| *code == frame.code()) {
        Some((_, decode)) => decode(frame.body()),
        None if opposite.contains(&frame.code()) => Err(FatalCode::WrongDirection),
        None => Err(FatalCode::UnknownMessageType),
    }
}

/// A nullable member that must still be present: plain `Option` would let
/// serde default a missing member to `None`.
pub(crate) fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}
