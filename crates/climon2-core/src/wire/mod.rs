//! [Daemon wire revision 1](../../../../docs/protocol/daemon-client/core-v1.md):
//! the frozen post-hello codec that the session daemon and the client library
//! share. It is platform-neutral and stateless. It frames, encodes and strictly
//! validates records. Roles, ordering, authority, request-ID sequencing and
//! whether a Response's result belongs to its request's operation belong to
//! the connection that owns them.
//!
//! Extend a direction by adding its record type, a `ControlRecord` impl and
//! one decoder-table entry; binary records implement `to_frame` themselves.
mod binary;
mod catalogue;
mod families;
mod frame;
mod metadata;
mod record;
mod request;
mod response;
mod server;
mod types;

/// Shares the session ID's GUID syntax and validator.
pub use crate::identity::Guid;
pub use binary::{Display, Input};
pub use catalogue::{Error, ErrorCode, FatalCode, OperationalCode};
pub use families::{Advertisement, Families, Family, FrameLimits, Requested};
pub use frame::{Frame, FrameLimit, WireError, read_frame, write_frame};
pub use metadata::{
    Available, CommandResult, Percent, Progress, ProgressState, Status, StatusState,
};
pub use record::{ControlRecord, defines};
pub use request::{
    Acknowledge, Acquire, Focus, Kill, Leave, Mode, Open, Ready, Request, Resize, SetViewing,
};
pub use response::{
    Cleanup, Failure, FocusBytes, KnownResult, Outcome, PartialCleanup, PartialTermination,
    Rejection, Response, Success,
};
pub use server::{
    ActualGeometry, CommandFinal, ControlGranted, ControlRevoked, FatalError, GrantReason,
    InitialMetadata, Notification, NotificationSource, ProgressChanged, RestorationBegin,
    RestorationComplete, RevocationReason, ServerMessage, StatusChanged, TitleChanged,
};
pub use types::{Counter, Geometry, Utc};
