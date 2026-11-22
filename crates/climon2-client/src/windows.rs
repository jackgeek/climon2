//! The client on Windows so far: kill mode runs as on Unix (#318); ordinary
//! attach needs the console rules (docs/windows-lifecycle.md, "Client
//! console"), which arrive with #321.
use crate::embedded;
use climon2_core::{keys::ClientSettings, metadata::SessionMetadata};
use std::io;

/// Refused, before any connection, until the client has its console rules.
pub(crate) fn attach(
    _metadata: &SessionMetadata,
    _session: &embedded::Session,
    _settings: ClientSettings,
) -> io::Result<u32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "attach is not supported on Windows yet",
    ))
}
