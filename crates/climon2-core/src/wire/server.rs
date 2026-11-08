//! Server messages: session daemon to client, types `0x40` through `0x4d`.
//! Session metadata flows to every selected family regardless of control.
use super::{
    Available, CommandResult, Counter, Error, FatalCode, Frame, Progress, Response, Status,
    record::{self, ControlRecord, Decoder, nullable},
};
use crate::ContractError;
use serde::{Deserialize, Serialize};

/// Fatal error `0x4d`: a best-effort report before the connection fails.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FatalError(pub Error);

/// Initial metadata `0x41`: the atomic report after a successful Open. Each
/// family's availability is explicit, never a fabricated empty value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "InitialMetadataFields")]
pub struct InitialMetadata {
    title: Available<Option<String>>,
    status: Available<Status>,
    progress: Available<Progress>,
    command_result: Option<CommandResult>,
}

impl InitialMetadata {
    /// A final status and a known command result must agree.
    pub fn new(
        title: Available<Option<String>>,
        status: Available<Status>,
        progress: Available<Progress>,
        command_result: Option<CommandResult>,
    ) -> Option<Self> {
        agrees(&status, command_result.as_ref()).then_some(Self {
            title,
            status,
            progress,
            command_result,
        })
    }

    /// An available `None` is an unset title, distinct from an empty one.
    pub fn title(&self) -> &Available<Option<String>> {
        &self.title
    }

    pub fn status(&self) -> &Available<Status> {
        &self.status
    }

    pub fn progress(&self) -> &Available<Progress> {
        &self.progress
    }

    pub fn command_result(&self) -> Option<&CommandResult> {
        self.command_result.as_ref()
    }
}

fn agrees(status: &Available<Status>, result: Option<&CommandResult>) -> bool {
    match (status, result) {
        (Available::Available(status), Some(result)) if status.state().is_final() => {
            status.state() == result.final_state()
        }
        _ => true,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InitialMetadataFields {
    title: Available<Option<String>>,
    status: Available<Status>,
    progress: Available<Progress>,
    #[serde(deserialize_with = "nullable")]
    command_result: Option<CommandResult>,
}

impl TryFrom<InitialMetadataFields> for InitialMetadata {
    type Error = ContractError;

    fn try_from(fields: InitialMetadataFields) -> Result<Self, Self::Error> {
        Self::new(
            fields.title,
            fields.status,
            fields.progress,
            fields.command_result,
        )
        .ok_or_else(|| ContractError("final status and command result disagree".into()))
    }
}

/// Title changed `0x42`: `None` is unset, distinct from an empty title. The
/// title is inert text, never a host-title instruction.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TitleChanged {
    #[serde(deserialize_with = "nullable")]
    pub title: Option<String>,
}

/// Status changed `0x43`: never the final transition, which only Command
/// final carries.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "StatusChangedFields")]
pub struct StatusChanged {
    status: Status,
}

impl StatusChanged {
    pub fn new(status: Status) -> Option<Self> {
        (!status.state().is_final()).then_some(Self { status })
    }

    pub fn status(&self) -> &Status {
        &self.status
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusChangedFields {
    status: Status,
}

impl TryFrom<StatusChangedFields> for StatusChanged {
    type Error = ContractError;

    fn try_from(fields: StatusChangedFields) -> Result<Self, Self::Error> {
        Self::new(fields.status)
            .ok_or_else(|| ContractError("Command final carries final status".into()))
    }
}

/// Notification `0x44`: a normalized, inert notification event. The episode
/// is attention identity; the source ID is not.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Notification {
    pub source: NotificationSource,
    #[serde(deserialize_with = "nullable")]
    pub source_id: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub title: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub body: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub episode_id: Option<Counter>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSource {
    Osc9,
    Osc99,
    Osc777,
}

/// Progress changed `0x45`: the retained normalized progress.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgressChanged {
    pub progress: Progress,
}

/// Command final `0x4c`: the one authoritative final result. Its status is
/// the agreeing final status when status is selected, otherwise null.
#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CommandFinalFields")]
pub struct CommandFinal {
    command_result: CommandResult,
    status: Option<Status>,
}

impl CommandFinal {
    pub fn new(command_result: CommandResult, status: Option<Status>) -> Option<Self> {
        let agrees = status
            .as_ref()
            .is_none_or(|status| status.state() == command_result.final_state());
        agrees.then_some(Self {
            command_result,
            status,
        })
    }

    pub fn command_result(&self) -> &CommandResult {
        &self.command_result
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandFinalFields {
    command_result: CommandResult,
    #[serde(deserialize_with = "nullable")]
    status: Option<Status>,
}

impl TryFrom<CommandFinalFields> for CommandFinal {
    type Error = ContractError;

    fn try_from(fields: CommandFinalFields) -> Result<Self, Self::Error> {
        Self::new(fields.command_result, fields.status)
            .ok_or_else(|| ContractError("status must be the final status of the result".into()))
    }
}

impl ControlRecord for InitialMetadata {
    const CODE: u8 = 0x41;
}

impl ControlRecord for TitleChanged {
    const CODE: u8 = 0x42;
}

impl ControlRecord for StatusChanged {
    const CODE: u8 = 0x43;
}

impl ControlRecord for Notification {
    const CODE: u8 = 0x44;
}

impl ControlRecord for ProgressChanged {
    const CODE: u8 = 0x45;
}

impl ControlRecord for CommandFinal {
    const CODE: u8 = 0x4c;
}

impl ControlRecord for FatalError {
    const CODE: u8 = 0x4d;
}

/// Every server message the client decodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServerMessage {
    Response(Response),
    InitialMetadata(InitialMetadata),
    TitleChanged(TitleChanged),
    StatusChanged(StatusChanged),
    Notification(Notification),
    ProgressChanged(ProgressChanged),
    CommandFinal(CommandFinal),
    FatalError(FatalError),
}

const DECODERS: [(u8, Decoder<ServerMessage>); 8] = [
    (Response::CODE, |body| {
        record::decode(body).map(ServerMessage::Response)
    }),
    (InitialMetadata::CODE, |body| {
        record::decode(body).map(ServerMessage::InitialMetadata)
    }),
    (TitleChanged::CODE, |body| {
        record::decode(body).map(ServerMessage::TitleChanged)
    }),
    (StatusChanged::CODE, |body| {
        record::decode(body).map(ServerMessage::StatusChanged)
    }),
    (Notification::CODE, |body| {
        record::decode(body).map(ServerMessage::Notification)
    }),
    (ProgressChanged::CODE, |body| {
        record::decode(body).map(ServerMessage::ProgressChanged)
    }),
    (CommandFinal::CODE, |body| {
        record::decode(body).map(ServerMessage::CommandFinal)
    }),
    (FatalError::CODE, |body| {
        record::decode(body).map(ServerMessage::FatalError)
    }),
];

impl ServerMessage {
    /// Decode a frame the client received.
    pub fn from_frame(frame: &Frame) -> Result<Self, FatalCode> {
        record::dispatch(frame, &DECODERS, record::REQUEST_TYPES)
    }
}
