//! Response `0x40`: one correlated outcome for each received request. The
//! codec checks each outcome's error and the results it may carry. Whether a
//! result belongs to the request's operation is the connection's to check: a
//! stateless codec does not know which request a request ID names.
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

/// The operation failed, or whether it ran is unknown. It may expose what is
/// known of the operation instead of hiding partial effects, but never a
/// successful Kill confirmation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Failure {
    error: Error,
    known: Option<KnownResult>,
}

/// What a failed or indeterminate outcome knows of its operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KnownResult {
    /// The operation's known result, in its success shape.
    Operation(Success),
    /// What a Kill knows of termination: cleanup is incomplete or unknown.
    Termination(PartialTermination),
}

impl Failure {
    /// `None` for a confirmed termination, which only a Kill success reports.
    pub fn new(error: Error, known: Option<KnownResult>) -> Option<Self> {
        let confirmed = matches!(
            known,
            Some(KnownResult::Operation(Success::Termination { .. }))
        );
        (!confirmed).then_some(Self { error, known })
    }

    pub fn error(&self) -> &Error {
        &self.error
    }

    pub fn known(&self) -> Option<&KnownResult> {
        self.known.as_ref()
    }
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

/// The results only a non-success outcome carries, as written on the wire:
/// a rejection's `known_final` and a failed or indeterminate Kill's partial
/// `termination`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum NonSuccessResult {
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
    fn non_success(self, error: Error, result: Option<ResultBody>) -> Option<Outcome> {
        match self {
            Self::Ok => None,
            Self::Rejected => Rejection::decoded(error, result).map(Outcome::Rejected),
            Self::Failed => Failure::decoded(error, result).map(Outcome::Failed),
            Self::Indeterminate => Failure::decoded(error, result).map(Outcome::Indeterminate),
        }
    }
}

impl Rejection {
    /// A rejection carries at most `known_final`.
    fn decoded(error: Error, result: Option<ResultBody>) -> Option<Self> {
        let known_final = match result {
            None => None,
            Some(ResultBody::NonSuccess(Object(NonSuccessResult::KnownFinal {
                command_result,
            }))) => Some(command_result),
            Some(_) => return None,
        };
        Some(Self { error, known_final })
    }
}

impl Failure {
    /// A failure carries at most a known result; never `known_final`.
    fn decoded(error: Error, result: Option<ResultBody>) -> Option<Self> {
        let known = match result {
            None => None,
            Some(ResultBody::Success(Object(success))) => Some(KnownResult::Operation(success)),
            Some(ResultBody::NonSuccess(Object(NonSuccessResult::Termination(termination)))) => {
                Some(KnownResult::Termination(termination))
            }
            Some(ResultBody::NonSuccess(Object(NonSuccessResult::KnownFinal { .. }))) => {
                return None;
            }
        };
        Self::new(error, known)
    }
}

/// Success results and non-success results share the `kind` space; each
/// side decodes only from an object.
#[derive(Deserialize)]
#[serde(untagged)]
enum ResultBody {
    Success(Object<Success>),
    NonSuccess(Object<NonSuccessResult>),
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
            Some(error) => fields.outcome.non_success(error, fields.result),
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

#[derive(Serialize)]
#[serde(untagged)]
enum ResultMember<'a> {
    Success(&'a Success),
    NonSuccess(NonSuccessResult),
}

#[derive(Serialize)]
struct ResponseMembers<'a> {
    request_id: Counter,
    outcome: OutcomeName,
    result: Option<ResultMember<'a>>,
    error: Option<&'a Error>,
}

impl Response {
    fn members(&self) -> ResponseMembers<'_> {
        let request_id = self.request_id;
        match &self.outcome {
            Outcome::Ok(success) => ResponseMembers {
                request_id,
                outcome: OutcomeName::Ok,
                result: Some(ResultMember::Success(success)),
                error: None,
            },
            Outcome::Rejected(rejection) => rejection.members(request_id),
            Outcome::Failed(failure) => failure.members(request_id, OutcomeName::Failed),
            Outcome::Indeterminate(failure) => {
                failure.members(request_id, OutcomeName::Indeterminate)
            }
        }
    }
}

impl Rejection {
    fn members(&self, request_id: Counter) -> ResponseMembers<'_> {
        let known_final = self.known_final.clone().map(|command_result| {
            ResultMember::NonSuccess(NonSuccessResult::KnownFinal { command_result })
        });
        ResponseMembers {
            request_id,
            outcome: OutcomeName::Rejected,
            result: known_final,
            error: Some(&self.error),
        }
    }
}

impl Failure {
    fn members(&self, request_id: Counter, outcome: OutcomeName) -> ResponseMembers<'_> {
        ResponseMembers {
            request_id,
            outcome,
            result: self.known.as_ref().map(KnownResult::member),
            error: Some(&self.error),
        }
    }
}

impl KnownResult {
    fn member(&self) -> ResultMember<'_> {
        match self {
            Self::Operation(success) => ResultMember::Success(success),
            Self::Termination(termination) => {
                ResultMember::NonSuccess(NonSuccessResult::Termination(termination.clone()))
            }
        }
    }
}

impl Serialize for Response {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.members().serialize(serializer)
    }
}

impl ControlRecord for Response {
    const CODE: u8 = 0x40;
}
