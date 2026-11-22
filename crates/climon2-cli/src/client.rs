//! What `attach` and `kill` share: choosing one live session, and the client
//! of that session's own version that carries the subcommand out.
use crate::{Error, list::LiveSession};
use climon2_core::{
    layout::Invocation,
    metadata::SessionMetadata,
    selector::Selector,
    startup_envelope::{ENVIRONMENT_VARIABLE, StartupEnvelope},
};
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

pub(crate) fn select<'a>(
    sessions: &'a [LiveSession],
    selector: &Selector,
) -> Result<&'a LiveSession, Error> {
    selector
        .select(sessions, |session| &session.metadata.session_id)
        .map_err(|error| Error::new(125, error))
}

/// The client of the session's own version, never this CLI's. The envelope always
/// overwrites an inherited one.
pub(crate) fn client(
    home: &Path,
    metadata: &SessionMetadata,
    envelope: StartupEnvelope,
) -> Result<Command, Error> {
    let invocation = client_invocation(home, metadata)?;
    let mut command = Command::new(installed(&invocation.program, metadata)?);
    command
        .args(invocation.args)
        .env(ENVIRONMENT_VARIABLE, envelope.to_json())
        .env("CLIMON2_HOME", home);
    Ok(command)
}

fn installed(client: &Path, metadata: &SessionMetadata) -> Result<PathBuf, Error> {
    client.canonicalize().map_err(|_| {
        Error::new(
            125,
            format!(
                "session {} needs climon2 {}, which is not installed",
                metadata.session_id, metadata.climon2_version
            ),
        )
    })
}

fn client_invocation(home: &Path, metadata: &SessionMetadata) -> io::Result<Invocation> {
    let layout = crate::executable_layout()?;
    let path = home
        .join("sessions")
        .join(format!("{}.pty", metadata.session_id));
    layout
        .client_invocation(&metadata.climon2_version, &path)
        .map_err(io::Error::other)
}
