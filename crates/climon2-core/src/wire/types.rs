//! Shared wire types. These are wire shapes, not C ABI declarations.
use crate::{ContractError, json::Object};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    num::{NonZeroU32, NonZeroU64},
    str::FromStr,
};
use time::{PrimitiveDateTime, format_description};

/// A canonical decimal string from 1 through 18446744073709551615, with no
/// sign or leading zero. It never wraps or repeats.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Counter(NonZeroU64);

impl Counter {
    pub fn new(value: u64) -> Option<Self> {
        NonZeroU64::new(value).map(Self)
    }

    pub fn get(self) -> u64 {
        self.0.get()
    }
}

impl FromStr for Counter {
    type Err = ContractError;

    /// `u64::from_str` alone would accept a sign and leading zeros.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let canonical = !text.starts_with('0') && text.bytes().all(|byte| byte.is_ascii_digit());
        canonical
            .then(|| text.parse::<NonZeroU64>().ok())
            .flatten()
            .map(Self)
            .ok_or_else(|| ContractError("invalid counter".into()))
    }
}

impl TryFrom<String> for Counter {
    type Error = ContractError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Counter> for String {
    fn from(counter: Counter) -> Self {
        counter.to_string()
    }
}

impl fmt::Display for Counter {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The control period and restoration ID that a record names together: the
/// authority a request claims or a server record carries. On the wire they
/// stay two members, `period` and `restoration_id`, beside the record's
/// others, or the same two header counters in a binary record.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct AuthorityIds {
    pub period: Counter,
    pub restoration_id: Counter,
}

/// A terminal size in positive columns and rows; representable is not supported.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(from = "Object<GeometryFields>")]
pub struct Geometry {
    pub cols: NonZeroU32,
    pub rows: NonZeroU32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GeometryFields {
    cols: NonZeroU32,
    rows: NonZeroU32,
}

impl From<Object<GeometryFields>> for Geometry {
    fn from(Object(fields): Object<GeometryFields>) -> Self {
        Self {
            cols: fields.cols,
            rows: fields.rows,
        }
    }
}

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` on a real calendar date, with seconds 00
/// through 59. It records when; it is not an ordering or an identity.
///
/// It is deliberately not the frozen metadata's `Timestamp`. That
/// `created_at` contract is RFC 3339 UTC with milliseconds, which also admits
/// a leap second (`23:59:60`) and a `t` or space separator, and readers of
/// every release's metadata must keep accepting them. The wire fixes this
/// one exact form.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Utc(String);

const UTC: &str = "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z";

impl Utc {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for Utc {
    type Err = ContractError;

    /// The exact format rejects other separators, precisions and leap seconds,
    /// which RFC 3339 parsing would accept; `[year]` alone would accept a sign.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let format = format_description::parse_borrowed::<2>(UTC).expect("a valid format");
        let unsigned = text.starts_with(|first: char| first.is_ascii_digit());
        PrimitiveDateTime::parse(text, &format)
            .ok()
            .filter(|_| unsigned)
            .map(|_| Self(text.to_owned()))
            .ok_or_else(|| ContractError("invalid UTC time".into()))
    }
}

impl TryFrom<String> for Utc {
    type Error = ContractError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        text.parse()
    }
}

impl From<Utc> for String {
    fn from(utc: Utc) -> Self {
        utc.0
    }
}
