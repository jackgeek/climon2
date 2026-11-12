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
    Rejected(Rejection),
    Failed(Failure),
    Indeterminate(Failure),
}

/// The operation did not run. An initial role rejection may carry the
/// command result the session daemon already knows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Rejection {
    pub error: Error,
    pub known_final: Option<CommandResult>,
}

/// The operation failed, or whether it ran is unknown. A Kill may expose what
/// it knows of termination, never a successful confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    pub error: Error,
    pub termination: Option<PartialTermination>,
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

/// A Kill that did not confirm termination: any command result it knows, and
/// cleanup that is incomplete or unknown.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartialTermination {
    #[serde(deserialize_with = "nullable")]
    pub command_result: Option<CommandResult>,
    pub cleanup: PartialCleanup,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartialCleanup {
    Incomplete,
    Unknown,
}

/// The results a non-success may carry, as written on the wire. Only a
/// rejection carries `known_final`; only a failed or indeterminate outcome
/// carries partial `termination`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Known {
    KnownFinal { command_result: CommandResult },
    Termination(PartialTermination),
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
    fn failure(self, error: Error, known: Option<Known>) -> Option<Outcome> {
        match self {
            Self::Ok => None,
            Self::Rejected => Rejection::decoded(error, known).map(Outcome::Rejected),
            Self::Failed => Failure::decoded(error, known).map(Outcome::Failed),
            Self::Indeterminate => Failure::decoded(error, known).map(Outcome::Indeterminate),
        }
    }
}

impl Rejection {
    fn decoded(error: Error, known: Option<Known>) -> Option<Self> {
        let known_final = match known {
            None => None,
            Some(Known::KnownFinal { command_result }) => Some(command_result),
            Some(Known::Termination(_)) => return None,
        };
        Some(Self { error, known_final })
    }
}

impl Failure {
    fn decoded(error: Error, known: Option<Known>) -> Option<Self> {
        let termination = match known {
            None => None,
            Some(Known::Termination(termination)) => Some(termination),
            Some(Known::KnownFinal { .. }) => return None,
        };
        Some(Self { error, termination })
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
    name.failure(error, known)
}

#[derive(Serialize)]
#[serde(untagged)]
enum ResultMember<'a> {
    Success(&'a Success),
    Known(Known),
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
            Self::Rejected(rejection) => rejection.members(),
            Self::Failed(failure) => failure.members(OutcomeName::Failed),
            Self::Indeterminate(failure) => failure.members(OutcomeName::Indeterminate),
        }
    }
}

impl Rejection {
    fn members(&self) -> Members<'_> {
        let known = self
            .known_final
            .clone()
            .map(|command_result| ResultMember::Known(Known::KnownFinal { command_result }));
        (OutcomeName::Rejected, known, Some(&self.error))
    }
}

impl Failure {
    fn members(&self, name: OutcomeName) -> Members<'_> {
        let known = self.termination.clone().map(Known::Termination);
        (name, known.map(ResultMember::Known), Some(&self.error))
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
}
