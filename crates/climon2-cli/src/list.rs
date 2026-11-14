use climon2_core::{layout::HomeLayout, metadata::SessionMetadata};
use climon2_discovery::{Cleanup, MetadataLimits, Options, Outcome, StaleMetadata, Uncertainty};
use climon2_platform::presentation;
use std::{io, io::Write, path::Path};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub use climon2_discovery::LiveSession;

/// Never reads the configuration and never needs a version's client.
pub fn run(home: &Path, as_json: bool) -> io::Result<()> {
    let sessions = discover_live(home)?;
    let output = if as_json {
        json(&sessions)?
    } else {
        table(&sessions, presentation::columns())?
    };
    io::stdout().lock().write_all(output.as_bytes())
}

/// The metadata objects, unchanged, each with the advertised `state` or `null`.
pub fn json(sessions: &[LiveSession]) -> io::Result<String> {
    let entries = sessions
        .iter()
        .map(json_entry)
        .collect::<io::Result<Vec<_>>>()?;
    Ok(format!("{}\n", serde_json::to_string(&entries)?))
}

fn json_entry(session: &LiveSession) -> io::Result<serde_json::Value> {
    let mut entry = serde_json::to_value(&session.metadata)?;
    entry["state"] = session.state().map_or(serde_json::Value::Null, |state| {
        serde_json::json!({
            "lifecycle": state.lifecycle().name(),
            "attachments": state.attachments(),
            "controlled": state.controlled(),
        })
    });
    Ok(entry)
}

pub fn discover(home: &Path) -> io::Result<Vec<SessionMetadata>> {
    discover_live(home).map(|sessions| {
        sessions
            .into_iter()
            .map(|session| session.metadata)
            .collect()
    })
}

/// The CLI reads metadata as it always has, whole and as deeply nested as the parser
/// accepts, and only CLI callers remove stale metadata.
const DISCOVERY: Options = Options {
    limits: MetadataLimits::UNBOUNDED,
    stale: StaleMetadata::Remove,
};

/// Diagnostics are logged here, on the caller's thread, so its subscriber records them.
pub fn discover_live(home: &Path) -> io::Result<Vec<LiveSession>> {
    let layout =
        HomeLayout::resolve(Some(home.as_os_str()), Path::new("/")).map_err(io::Error::other)?;
    let pass = climon2_discovery::discover(&layout, DISCOVERY)?;
    for _ in &pass.invalid {
        tracing::warn!("invalid session metadata");
    }
    pass.sessions.iter().for_each(log_probe);
    let live: Vec<_> = pass
        .sessions
        .into_iter()
        .filter_map(Outcome::live)
        .collect();
    live.iter().for_each(warn_if_malformed);
    Ok(live)
}

/// What each probe logged when discovery still lived in the CLI.
fn log_probe(outcome: &Outcome) {
    match outcome {
        Outcome::Uncertain(_, uncertainty) => log_uncertainty(uncertainty),
        Outcome::Stale(_, Cleanup::Failed(error)) => {
            tracing::warn!(errno = error.raw_os_error(), "stale session cleanup failed")
        }
        Outcome::Live(_) | Outcome::Stale(..) => {}
    }
}

fn log_uncertainty(uncertainty: &Uncertainty) {
    match uncertainty {
        Uncertainty::Mismatched | Uncertainty::Unanswered(_) => tracing::debug!(
            responded = matches!(uncertainty, Uncertainty::Mismatched),
            "session probe did not match"
        ),
        Uncertainty::Unreachable(error) => {
            tracing::debug!(errno = error.raw_os_error(), "session connection failed")
        }
        Uncertainty::DaemonAlive => tracing::debug!("refused connection belongs to a live daemon"),
        Uncertainty::IdentityUnknown(error) => {
            tracing::warn!(errno = error.raw_os_error(), "daemon identity is uncertain")
        }
    }
}

fn warn_if_malformed(session: &LiveSession) {
    let extensions = &session.extensions;
    if extensions.capabilities.is_malformed() || extensions.state.is_malformed() {
        tracing::warn!(session = %session.metadata.session_id, "malformed hello extensions");
    }
}

fn escaped(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| {
            if character.is_control() {
                character.escape_default().collect::<Vec<_>>()
            } else {
                vec![character]
            }
        })
        .collect()
}

fn quoted(value: &str) -> String {
    let value = escaped(value);
    if !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_@%+=:,./-".contains(character))
    {
        value
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn truncate(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut result = leading_cells(value, width - 1);
    result.push('…');
    result
}

fn leading_cells(value: &str, mut remaining: usize) -> String {
    let mut result = String::new();
    for character in value.chars() {
        let cells = character.width().unwrap_or_default();
        if cells > remaining {
            break;
        }
        result.push(character);
        remaining -= cells;
    }
    result
}

fn local_created(metadata: &SessionMetadata) -> io::Result<String> {
    presentation::local_datetime(&metadata.created_at)
}

const HEADERS: [&str; 6] = [
    "SESSION", "CREATED", "VERSION", "STATE", "CLIENTS", "COMMAND",
];

/// The padded columns before ARGS.
fn cells(session: &LiveSession) -> io::Result<[String; 6]> {
    let metadata = &session.metadata;
    let state = session.state();
    Ok([
        metadata.session_id.to_string(),
        local_created(metadata)?,
        metadata.climon2_version.to_string(),
        state.map_or("unknown".to_owned(), |state| {
            state.lifecycle().name().to_owned()
        }),
        state.map_or("-".to_owned(), |state| state.attachments().to_string()),
        escaped(&metadata.command),
    ])
}

fn widths(rows: &[[String; 6]]) -> [usize; 6] {
    let mut widths = HEADERS.map(str::len);
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.width());
        }
    }
    widths
}

/// Each cell padded to its column, then the separator before ARGS.
fn prefix(cells: &[impl AsRef<str>], widths: &[usize; 6]) -> String {
    let mut line = String::new();
    for (cell, width) in cells.iter().zip(widths) {
        let cell = cell.as_ref();
        line.push_str(cell);
        line.push_str(&" ".repeat(width - cell.width() + 2));
    }
    line
}

fn arguments(metadata: &SessionMetadata) -> String {
    metadata
        .args
        .iter()
        .map(|arg| quoted(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn table(sessions: &[LiveSession], width: usize) -> io::Result<String> {
    if sessions.is_empty() {
        return Ok("No climon2 sessions running.\n".to_owned());
    }
    let rows = sessions.iter().map(cells).collect::<io::Result<Vec<_>>>()?;
    let widths = widths(&rows);
    let mut output = prefix(&HEADERS, &widths);
    output.push_str("ARGS\n");
    for (session, row) in sessions.iter().zip(&rows) {
        let prefix = prefix(row, &widths);
        let arguments = arguments(&session.metadata);
        output.push_str(&prefix);
        output.push_str(&truncate(&arguments, width.saturating_sub(prefix.width())));
        output.push('\n');
    }
    Ok(output)
}
