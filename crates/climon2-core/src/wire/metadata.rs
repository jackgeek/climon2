//! Session metadata shapes: status, progress, command results and
//! availability.
use super::{Counter, Utc, record::nullable};
use crate::{ContractError, json::Object};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, value},
};
use std::num::NonZeroU32;

/// A raw normal exit or a source-host signal. Exit 143 is not signal 15, and
/// a signal number is never a local command.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    from = "Object<CommandResultFields>"
)]
pub enum CommandResult {
    Exit {
        code: u32,
    },
    Signal {
        number: NonZeroU32,
        name: Option<String>,
    },
}

impl CommandResult {
    /// Normal exit zero completes; every other exit and every signal fails.
    pub fn final_state(&self) -> StatusState {
        match self {
            Self::Exit { code: 0 } => StatusState::Completed,
            _ => StatusState::Failed,
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum CommandResultFields {
    Exit {
        code: u32,
    },
    Signal {
        number: NonZeroU32,
        #[serde(deserialize_with = "nullable")]
        name: Option<String>,
    },
}

impl From<Object<CommandResultFields>> for CommandResult {
    fn from(Object(fields): Object<CommandResultFields>) -> Self {
        match fields {
            CommandResultFields::Exit { code } => Self::Exit { code },
            CommandResultFields::Signal { number, name } => Self::Signal { number, name },
        }
    }
}

/// The effective session status a session daemon reports.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusState {
    Idle,
    Running,
    NeedsAttention,
    Acknowledged,
    Completed,
    Failed,
}

impl StatusState {
    /// Only the attention states carry an episode.
    pub fn is_attention(self) -> bool {
        matches!(self, Self::NeedsAttention | Self::Acknowledged)
    }

    /// The final states arrive only with Command final.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Completed | Self::Failed)
    }
}

/// Effective status, its stored change time, and the episode that exactly the
/// attention states carry.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Object<StatusFields>")]
pub struct Status {
    state: StatusState,
    changed_at: Utc,
    #[serde(rename = "episode_id")]
    episode: Option<Counter>,
}

impl Status {
    pub fn new(state: StatusState, changed_at: Utc, episode: Option<Counter>) -> Option<Self> {
        (state.is_attention() == episode.is_some()).then_some(Self {
            state,
            changed_at,
            episode,
        })
    }

    pub fn state(&self) -> StatusState {
        self.state
    }

    pub fn changed_at(&self) -> &Utc {
        &self.changed_at
    }

    pub fn episode(&self) -> Option<Counter> {
        self.episode
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusFields {
    state: StatusState,
    changed_at: Utc,
    #[serde(deserialize_with = "nullable")]
    episode_id: Option<Counter>,
}

impl TryFrom<Object<StatusFields>> for Status {
    type Error = ContractError;

    fn try_from(Object(fields): Object<StatusFields>) -> Result<Self, Self::Error> {
        Self::new(fields.state, fields.changed_at, fields.episode_id)
            .ok_or_else(|| ContractError("an episode belongs only to attention".into()))
    }
}

/// Normalized progress state.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressState {
    None,
    Normal,
    Indeterminate,
    Error,
    Paused,
}

impl ProgressState {
    fn permits(self, percent: Option<Percent>) -> bool {
        match self {
            Self::Normal => percent.is_some(),
            Self::None | Self::Indeterminate => percent.is_none(),
            Self::Error | Self::Paused => true,
        }
    }
}

/// The current normalized progress: normal requires a percent, none and
/// indeterminate forbid one, error and paused allow either.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Object<ProgressFields>")]
pub struct Progress {
    state: ProgressState,
    percent: Option<Percent>,
}

impl Progress {
    pub fn new(state: ProgressState, percent: Option<Percent>) -> Option<Self> {
        state.permits(percent).then_some(Self { state, percent })
    }

    pub fn state(&self) -> ProgressState {
        self.state
    }

    pub fn percent(&self) -> Option<Percent> {
        self.percent
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProgressFields {
    state: ProgressState,
    #[serde(deserialize_with = "nullable")]
    percent: Option<Percent>,
}

impl TryFrom<Object<ProgressFields>> for Progress {
    type Error = ContractError;

    fn try_from(Object(fields): Object<ProgressFields>) -> Result<Self, Self::Error> {
        Self::new(fields.state, fields.percent)
            .ok_or_else(|| ContractError("percent does not match the progress state".into()))
    }
}

/// An integer percentage from 0 through 100.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Percent(u8);

impl Percent {
    pub fn new(value: u8) -> Option<Self> {
        (value <= 100).then_some(Self(value))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Percent {
    type Error = ContractError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value).ok_or_else(|| ContractError("percent above 100".into()))
    }
}

impl From<Percent> for u8 {
    fn from(percent: Percent) -> Self {
        percent.0
    }
}

/// Whether a family can supply a value, distinct from the value being unset
/// or empty. Unavailable always carries null.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum Available<T> {
    Unavailable,
    Available(T),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "T: Deserialize<'de>"))]
struct AvailableFields<T> {
    available: bool,
    #[serde(deserialize_with = "nullable")]
    value: Option<T>,
}

impl<'de, T: Deserialize<'de>> AvailableFields<T> {
    /// An available null is whatever `T` makes of null: an unset title is
    /// valid, while status and progress require a value.
    fn availability(self) -> Option<Available<T>> {
        match (self.available, self.value) {
            (false, None) => Some(Available::Unavailable),
            (true, Some(value)) => Some(Available::Available(value)),
            (true, None) => T::deserialize(value::UnitDeserializer::<value::Error>::new())
                .ok()
                .map(Available::Available),
            (false, Some(_)) => None,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Available<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Object(fields) = Object::<AvailableFields<T>>::deserialize(deserializer)?;
        fields
            .availability()
            .ok_or_else(|| D::Error::custom("availability and value disagree"))
    }
}

#[derive(Serialize)]
struct AvailableMembers<'a, T> {
    available: bool,
    value: Option<&'a T>,
}

impl<T: Serialize> Serialize for Available<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (available, value) = match self {
            Self::Unavailable => (false, None),
            Self::Available(value) => (true, Some(value)),
        };
        AvailableMembers { available, value }.serialize(serializer)
    }
}
