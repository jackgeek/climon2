//! Response `0x40`: one correlated outcome for each received request.
use super::{
    CommandResult, Counter, Error, Families, Guid,
    record::{ControlRecord, nullable},
};
use crate::{ContractError, json::Object};
use serde::{Deserialize, Serialize, Serializer};
use std::num::NonZeroU32;

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(try_from = "ResponseFields")]
pub struct Response {
    pub request_id: Counter,
    pub outcome: Outcome,
}

/// `rejected` guarantees the operation did not run; `failed` and
/// `indeterminate` do not. A lost response is not a rejection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Outcome {
    Ok(Success),
    Rejected(Failure),
    Failed(Failure),
    Indeterminate(Failure),
}

/// A non-success always has an error and at most a known result, never a
/// successful confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    pub error: Error,
    pub known: Option<Known>,
}

/// The success result of each operation, named by its `kind`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Success {
    Opened {
        connection_id: Guid,
        selected_capabilities: Families,
    },
    /// Equal to the whole Input payload; nothing is accepted partially.
    InputAccepted {
        accepted_bytes: NonZeroU32,
    },
    ResizeAccepted {},
    Acquired {
        changed: bool,
        period: Counter,
        restoration_id: Counter,
    },
    Ready {
        changed: bool,
        period: Counter,
        restoration_id: Counter,
    },
    Viewing {
        changed: bool,
        viewing: bool,
    },
    Acknowledgement {
        changed: bool,
    },
    Left {
        attachment_detached: bool,
    },
    /// Confirmed termination: a command result and complete daemon cleanup.
    Termination {
        command_result: CommandResult,
        cleanup: Cleanup,
    },
    Focus {
        changed: bool,
        focused: bool,
        accepted_bytes: FocusBytes,
    },
}

/// The only cleanup a successful Kill reports.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cleanup {
    Complete,
}

/// What Focus accepted for forwarding: no sequence, or the complete
/// three-byte focus report.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub enum FocusBytes {
    None,
    Sequence,
}

impl TryFrom<u32> for FocusBytes {
    type Error = ContractError;

    fn try_from(bytes: u32) -> Result<Self, Self::Error> {
        match bytes {
            0 => Ok(Self::None),
            3 => Ok(Self::Sequence),
            _ => Err(ContractError("focus accepts 0 or 3 bytes".into())),
        }
    }
}

impl From<FocusBytes> for u32 {
    fn from(bytes: FocusBytes) -> Self {
        match bytes {
            FocusBytes::None => 0,
            FocusBytes::Sequence => 3,
        }
    }
}

/// A result a non-success may expose instead of hiding known effects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Known {
    /// An initial role rejection with an already known command result.
    KnownFinal { command_result: CommandResult },
    /// A failed or indeterminate Kill: never a successful confirmation.
    Termination {
        #[serde(deserialize_with = "nullable")]
        command_result: Option<CommandResult>,
        cleanup: PartialCleanup,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialCleanup {
    Incomplete,
    Unknown,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum OutcomeName {
    Ok,
    Rejected,
    Failed,
    Indeterminate,
}

impl OutcomeName {
    fn failure(self, failure: Failure) -> Option<Outcome> {
        match self {
            Self::Ok => None,
            Self::Rejected => Some(Outcome::Rejected(failure)),
            Self::Failed => Some(Outcome::Failed(failure)),
            Self::Indeterminate => Some(Outcome::Indeterminate(failure)),
        }
    }
}

/// Success results and known results share the `kind` space; each side
/// decodes only from an object.
#[derive(Deserialize)]
#[serde(untagged)]
enum ResultBody {
    Success(Object<Success>),
    Known(Object<Known>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseFields {
    request_id: Counter,
    outcome: OutcomeName,
    #[serde(deserialize_with = "nullable")]
    result: Option<ResultBody>,
    #[serde(deserialize_with = "nullable")]
    error: Option<Error>,
}

impl TryFrom<ResponseFields> for Response {
    type Error = ContractError;

    fn try_from(fields: ResponseFields) -> Result<Self, Self::Error> {
        let outcome = match fields.error {
            None => success(fields.outcome, fields.result),
            Some(error) => failure(fields.outcome, error, fields.result),
        };
        outcome
            .map(|outcome| Self {
                request_id: fields.request_id,
                outcome,
            })
            .ok_or_else(|| ContractError("outcome, result and error disagree".into()))
    }
}

fn success(name: OutcomeName, result: Option<ResultBody>) -> Option<Outcome> {
    match (name, result) {
        (OutcomeName::Ok, Some(ResultBody::Success(Object(success)))) => Some(Outcome::Ok(success)),
        _ => None,
    }
}

fn failure(name: OutcomeName, error: Error, result: Option<ResultBody>) -> Option<Outcome> {
    let known = match result {
        None => None,
        Some(ResultBody::Known(Object(known))) => Some(known),
        Some(ResultBody::Success(_)) => return None,
    };
    name.failure(Failure { error, known })
}

#[derive(Serialize)]
#[serde(untagged)]
enum ResultMember<'a> {
    Success(&'a Success),
    Known(&'a Known),
}

#[derive(Serialize)]
struct ResponseMembers<'a> {
    request_id: Counter,
    outcome: OutcomeName,
    result: Option<ResultMember<'a>>,
    error: Option<&'a Error>,
}

type Members<'a> = (OutcomeName, Option<ResultMember<'a>>, Option<&'a Error>);

impl Outcome {
    fn members(&self) -> Members<'_> {
        match self {
            Self::Ok(success) => (OutcomeName::Ok, Some(ResultMember::Success(success)), None),
            Self::Rejected(failure) => failure.members(OutcomeName::Rejected),
            Self::Failed(failure) => failure.members(OutcomeName::Failed),
            Self::Indeterminate(failure) => failure.members(OutcomeName::Indeterminate),
        }
    }
}

impl Failure {
    fn members(&self, name: OutcomeName) -> Members<'_> {
        let known = self.known.as_ref().map(ResultMember::Known);
        (name, known, Some(&self.error))
    }
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (outcome, result, error) = self.outcome.members();
        ResponseMembers {
            request_id: self.request_id,
            outcome,
            result,
            error,
        }
        .serialize(serializer)
    }
}

impl ControlRecord for Response {
    const CODE: u8 = 0x40;
    const REQUEST_ID: bool = true;
}
