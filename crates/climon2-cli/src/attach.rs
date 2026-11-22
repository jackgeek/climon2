use crate::{
    Error,
    client::{client, select},
    list::{self, LiveSession},
};
use climon2_core::{
    config::{AttachConfig, ConfigSource},
    layout::HomeLayout,
    metadata::SessionMetadata,
    selector::Selector,
    startup_envelope::StartupEnvelope,
};
use climon2_platform::terminal;
use std::{ffi::OsStr, io, os::unix::process::CommandExt, path::Path, process::Command};

/// Returns only on failure: success replaces this process with the client.
pub fn run(home: &Path, selector: Selector) -> Result<u32, Error> {
    Err(Error::from(prepare(home, &selector)?.exec()))
}

fn prepare(home: &Path, selector: &Selector) -> Result<Command, Error> {
    let sessions = list::discover_live(home)?;
    let session = select(&sessions, selector)?;
    admit(session)?;
    client_command(home, &session.metadata)
}

fn admit(session: &LiveSession) -> Result<(), Error> {
    require_attach(session)?;
    reject_self_attach(&session.metadata.session_id)?;
    require_terminal()
}

fn require_attach(session: &LiveSession) -> Result<(), Error> {
    let capabilities = session.extensions.capabilities.valid();
    capabilities
        .is_some_and(|capabilities| capabilities.supports_attach())
        .then_some(())
        .ok_or_else(|| {
            Error::new(
                125,
                format!(
                    "session {} does not support attach and client startup",
                    session.metadata.session_id
                ),
            )
        })
}

fn client_command(home: &Path, metadata: &SessionMetadata) -> Result<Command, Error> {
    let settings = attach_config(home)?.client;
    client(home, metadata, StartupEnvelope::Attach(settings))
}

fn attach_config(home: &Path) -> Result<AttachConfig, Error> {
    let layout = HomeLayout::at(home).map_err(io::Error::other)?;
    ConfigSource::read(&layout)
        .and_then(|source| source.attach())
        .map_err(|error| Error::new(125, error))
}

fn reject_self_attach(id: &climon2_core::SessionId) -> Result<(), Error> {
    if std::env::var_os("CLIMON2_SESSION_ID").as_deref() == Some(OsStr::new(id.as_str())) {
        return Err(Error::new(
            125,
            format!("cannot attach to the current session {}", id),
        ));
    }
    Ok(())
}

fn require_terminal() -> Result<(), Error> {
    terminal::require_ttys().map_err(|_| Error::new(125, "c2 attach needs a terminal"))
}
