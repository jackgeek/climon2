//! [Daemon wire revision 1](../../../../docs/protocol/daemon-client/core-v1.md):
//! the frozen post-hello codec that the session daemon and the client library
//! share. It is platform-neutral and stateless. It frames, encodes and strictly
//! validates records. Roles, ordering, authority and request-ID sequencing
//! belong to the connection that owns them.
mod catalogue;
mod frame;

pub use catalogue::FatalCode;
pub use frame::{Frame, FrameLimit, WireError, read_frame, write_frame};
