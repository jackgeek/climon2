use climon2_core::{
    hello::{HelloExtensions, SessionState},
    metadata::{SessionMetadata, metadata_session_id},
    protocol::{ClientMessage, HELLO_TIMEOUT, ServerMessage, read_frame, write_frame},
};
use climon2_platform::{connection, presentation, process, runtime::SocketStream};
use std::{
    fs, io,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

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

fn candidates(home: &Path) -> io::Result<Vec<PathBuf>> {
    match fs::read_dir(home.join("sessions")) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn read_metadata(path: &Path) -> Option<SessionMetadata> {
    if path.extension().is_none_or(|extension| extension != "pty") {
        return None;
    }
    let result = metadata_session_id(path)
        .map_err(io::Error::other)
        .and_then(|_| fs::read(path))
        .and_then(|bytes| SessionMetadata::from_json(path, &bytes).map_err(io::Error::other));
    match result {
        Ok(metadata) => Some(metadata),
        Err(_) => {
            tracing::warn!("invalid session metadata");
            None
        }
    }
}

pub fn discover(home: &Path) -> io::Result<Vec<SessionMetadata>> {
    discover_live(home).map(|sessions| {
        sessions
            .into_iter()
            .map(|session| session.metadata)
            .collect()
    })
}

#[derive(Clone, Debug)]
pub struct LiveSession {
    pub metadata: SessionMetadata,
    pub extensions: HelloExtensions,
}

impl LiveSession {
    /// Shown only when `state-v1` is advertised with a valid snapshot.
    pub fn state(&self) -> Option<SessionState> {
        let capabilities = self.extensions.capabilities.valid()?;
        let state = self.extensions.state.valid().copied();
        state.filter(|_| capabilities.supports_state())
    }
}

pub fn discover_live(home: &Path) -> io::Result<Vec<LiveSession>> {
    let metadata: Vec<_> = candidates(home)?
        .iter()
        .filter_map(|path| read_metadata(path))
        .collect();
    let mut live = std::thread::scope(|scope| {
        let workers: Vec<_> = metadata
            .into_iter()
            .map(|metadata| scope.spawn(move || inspect(home, metadata)))
            .collect();
        workers
            .into_iter()
            .map(|worker| {
                worker
                    .join()
                    .map_err(|_| io::Error::other("session probe panicked"))
            })
            .collect::<io::Result<Vec<_>>>()
    })?
    .into_iter()
    .flatten()
    .inspect(warn_if_malformed)
    .collect::<Vec<_>>();
    live.sort_by(|first, second| second.metadata.created_at.cmp(&first.metadata.created_at));
    Ok(live)
}

struct DeadlineStream {
    stream: SocketStream,
    deadline: Instant,
}

impl DeadlineStream {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "session probe timed out"))
    }

    fn wait_for(&self, interest: connection::Interest) -> io::Result<()> {
        connection::wait(&self.stream, interest, self.remaining()?)
    }

    fn transfer(
        &mut self,
        interest: connection::Interest,
        mut operation: impl FnMut(&mut SocketStream) -> io::Result<usize>,
    ) -> io::Result<usize> {
        loop {
            self.remaining()?;
            let result = operation(&mut self.stream);
            if !would_block(&result) {
                return result;
            }
            self.wait_for(interest)?;
        }
    }
}

fn would_block(result: &io::Result<usize>) -> bool {
    result
        .as_ref()
        .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.transfer(connection::Interest::Read, |stream| stream.read(bytes))
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(connection::Interest::Write, |stream| stream.write(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

fn hello(
    stream: SocketStream,
    deadline: Instant,
    metadata: &SessionMetadata,
) -> io::Result<Option<HelloExtensions>> {
    let (message, extensions) = exchange_hello(stream, deadline)?;
    match message {
        ServerMessage::Hello(reply) if reply.is_live_session(&metadata.session_id) => {
            Ok(Some(extensions))
        }
        _ => Ok(None),
    }
}

/// Nonblocking I/O under one absolute deadline; never socket timeouts (Darwin EINVAL after FIN).
fn exchange_hello(
    stream: SocketStream,
    deadline: Instant,
) -> io::Result<(ServerMessage, HelloExtensions)> {
    stream.set_nonblocking(true)?;
    let mut stream = DeadlineStream { stream, deadline };
    send_hello(&mut stream)?;
    receive_hello(&mut stream)
}

fn send_hello(stream: &mut DeadlineStream) -> io::Result<()> {
    write_frame(stream, &ClientMessage::Hello { protocol: 1 }.to_frame()?)
}

fn receive_hello(stream: &mut DeadlineStream) -> io::Result<(ServerMessage, HelloExtensions)> {
    let frame = read_frame(stream)?;
    let extensions = HelloExtensions::from_body(frame.body());
    Ok((ServerMessage::from_frame(frame)?, extensions))
}

fn inspect(home: &Path, metadata: SessionMetadata) -> Option<LiveSession> {
    let deadline = Instant::now() + HELLO_TIMEOUT;
    match connection::connect(Path::new(metadata.socket_path.as_str()), HELLO_TIMEOUT) {
        Ok(stream) => match hello(stream, deadline, &metadata) {
            Ok(Some(extensions)) => Some(LiveSession {
                metadata,
                extensions,
            }),
            result => {
                tracing::debug!(responded = result.is_ok(), "session probe did not match");
                None
            }
        },
        Err(error) => {
            inspect_failure(home, &metadata, &error);
            None
        }
    }
}

/// Logged on the caller's thread so its subscriber records it.
fn warn_if_malformed(session: &LiveSession) {
    let extensions = &session.extensions;
    if extensions.capabilities.is_malformed() || extensions.state.is_malformed() {
        tracing::warn!(session = %session.metadata.session_id, "malformed hello extensions");
    }
}

fn inspect_failure(home: &Path, metadata: &SessionMetadata, error: &io::Error) {
    if matches!(
        error.raw_os_error(),
        Some(libc::ENOENT | libc::ECONNREFUSED)
    ) {
        cleanup_if_dead(home, metadata);
    } else {
        tracing::debug!(errno = error.raw_os_error(), "session connection failed");
    }
}

/// Only a proven dead daemon identity makes a refused session stale.
fn cleanup_if_dead(home: &Path, metadata: &SessionMetadata) {
    match process::is_same_daemon(metadata) {
        Ok(false) => cleanup_stale(home, metadata),
        Ok(true) => tracing::debug!("refused connection belongs to a live daemon"),
        Err(error) => {
            tracing::warn!(errno = error.raw_os_error(), "daemon identity is uncertain")
        }
    }
}

fn cleanup_stale(home: &Path, metadata: &SessionMetadata) {
    let path = home
        .join("sessions")
        .join(format!("{}.pty", metadata.session_id));
    let result =
        climon2_platform::runtime::validate_stale_socket(Path::new(metadata.socket_path.as_str()))
            .and_then(|()| remove_if_present(&path))
            .and_then(|()| remove_if_present(Path::new(metadata.socket_path.as_str())));
    if let Err(error) = result {
        tracing::warn!(errno = error.raw_os_error(), "stale session cleanup failed");
    }
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
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
        let cells = character.width().unwrap_or(0);
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
