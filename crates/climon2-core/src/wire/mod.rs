//! [Daemon wire revision 1](../../../../docs/protocol/daemon-client/core-v1.md):
//! the frozen post-hello codec that the session daemon and the client library
//! share. It is platform-neutral and stateless. It frames, encodes and strictly
//! validates records. Roles, ordering, authority and request-ID sequencing
//! belong to the connection that owns them.
//!
//! Extend a direction by adding its record type, a `ControlRecord` impl and
//! one decoder-table entry; binary records implement `to_frame` themselves.
mod catalogue;
mod families;
mod frame;
mod metadata;
mod record;
mod request;
mod response;
mod server;
mod types;

pub use catalogue::{Error, ErrorCode, FatalCode, OperationalCode};
pub use families::{Advertisement, Families, Family, FrameLimits, Requested};
pub use frame::{Frame, FrameLimit, WireError, read_frame, write_frame};
pub use metadata::{
    Available, CommandResult, Percent, Progress, ProgressState, Status, StatusState,
};
pub use record::ControlRecord;
pub use request::{Kill, Leave, Mode, Open, Request};
pub use response::{
    Cleanup, Failure, FocusBytes, Known, Outcome, PartialCleanup, Response, Success,
};
pub use server::{
    CommandFinal, FatalError, InitialMetadata, Notification, NotificationSource, ProgressChanged,
    ServerMessage, StatusChanged, TitleChanged,
};
pub use types::{Counter, Geometry, Guid, Utc};
