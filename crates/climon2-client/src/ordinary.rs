//! Ordinary attach: the terminal, the attachment and the raw bridge.
use crate::{attachment, bridge, embedded};
use climon2_core::{keys::ClientSettings, metadata::SessionMetadata};
use climon2_platform::{home, terminal};
use std::{fs::File, io};

/// Ordinary attach, through this release's client library; a termination
/// signal before the bridge starts wins over any outcome.
pub(crate) fn attach(
    metadata: &SessionMetadata,
    session: &embedded::Session,
    settings: ClientSettings,
) -> io::Result<u32> {
    let state = Terminal::new()?;
    let result = connection(&state, metadata, session);
    if let Some(signal) = state.signals.termination_signal() {
        return Ok(128 + signal);
    }
    raw_bridge(result?, state, metadata, settings)
}

/// The attachment, once the session admitted it, before raw mode.
fn connection(
    state: &Terminal,
    metadata: &SessionMetadata,
    session: &embedded::Session,
) -> io::Result<bridge::Opened> {
    tracing::debug!("opening the attachment");
    let size = terminal::size(&state.input)?;
    let attachment = attachment::Attachment::open(metadata, session, home::named()?.root(), size)?;
    let backlog = attachment.opened(&state.signals)?;
    tracing::debug!("client attached");
    Ok(bridge::Opened {
        attachment,
        backlog,
        size,
    })
}

struct Terminal {
    input: File,
    signals: terminal::Signals,
}

impl Terminal {
    fn new() -> io::Result<Self> {
        terminal::require_ttys()?;
        Ok(Self {
            input: terminal::input()?,
            signals: terminal::Signals::client()?,
        })
    }
}

fn raw_bridge(
    opened: bridge::Opened,
    state: Terminal,
    metadata: &SessionMetadata,
    settings: ClientSettings,
) -> io::Result<u32> {
    let raw = terminal::RawModeGuard::new(&state.input)?;
    #[cfg(feature = "test-hooks")]
    client_fault();
    let result = bridge::run(
        opened,
        state.input,
        state.signals,
        &metadata.session_id,
        settings,
        lone_escape(),
    )?;
    drop(raw);
    Ok(exit_code(result, metadata))
}

/// How long the bridge holds a lone Escape (`terminal::LONE_ESCAPE`); with
/// `test-hooks`, `CLIMON2_CLIENT_LONE_ESCAPE_MS` sets it, so the Unix suites
/// test the timed release Windows uses.
fn lone_escape() -> Option<std::time::Duration> {
    #[cfg(feature = "test-hooks")]
    if let Some(milliseconds) = std::env::var("CLIMON2_CLIENT_LONE_ESCAPE_MS")
        .ok()
        .and_then(|value| value.parse().ok())
    {
        return Some(std::time::Duration::from_millis(milliseconds));
    }
    terminal::LONE_ESCAPE
}

#[cfg(feature = "test-hooks")]
fn client_fault() {
    if std::env::var_os("CLIMON2_CLIENT_FAULT").as_deref() == Some(std::ffi::OsStr::new("panic")) {
        panic!("CLIMON2_CLIENT_FAULT_PANIC");
    }
}

/// Raw mode is already restored, so the detach notice lands on a cooked terminal.
fn exit_code(result: bridge::BridgeExit, metadata: &SessionMetadata) -> u32 {
    match result {
        bridge::BridgeExit::Status(code) => code,
        bridge::BridgeExit::Detached => {
            eprintln!(
                "climon2: detached from session {}; reattach with: c2 attach {}",
                metadata.session_id, metadata.session_id
            );
            0
        }
    }
}
