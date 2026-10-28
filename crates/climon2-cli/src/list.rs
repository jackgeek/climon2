use climon2_core::{
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

pub fn run(home: &Path, json: bool) -> io::Result<()> {
    let sessions = discover(home)?;
    let output = if json {
        format!("{}\n", serde_json::to_string(&sessions)?)
    } else {
        table(&sessions, presentation::columns())?
    };
    io::stdout().lock().write_all(output.as_bytes())
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
    .collect::<Vec<_>>();
    live.sort_by(|first, second| second.created_at.cmp(&first.created_at));
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

fn hello(stream: SocketStream, deadline: Instant, metadata: &SessionMetadata) -> io::Result<bool> {
    match exchange_hello(stream, deadline)? {
        ServerMessage::Hello(reply) => Ok(reply.is_live_session(&metadata.session_id)),
        _ => Ok(false),
    }
}

/// Nonblocking I/O under one absolute deadline; never socket timeouts (Darwin EINVAL after FIN).
fn exchange_hello(stream: SocketStream, deadline: Instant) -> io::Result<ServerMessage> {
    stream.set_nonblocking(true)?;
    let mut stream = DeadlineStream { stream, deadline };
    send_hello(&mut stream)?;
    receive_hello(&mut stream)
}

fn send_hello(stream: &mut DeadlineStream) -> io::Result<()> {
    write_frame(stream, &ClientMessage::Hello { protocol: 1 }.to_frame()?)
}

fn receive_hello(stream: &mut DeadlineStream) -> io::Result<ServerMessage> {
    ServerMessage::from_frame(read_frame(stream)?)
}

fn inspect(home: &Path, metadata: SessionMetadata) -> Option<SessionMetadata> {
    let deadline = Instant::now() + HELLO_TIMEOUT;
    match connection::connect(Path::new(metadata.socket_path.as_str()), HELLO_TIMEOUT) {
        Ok(stream) => match hello(stream, deadline, &metadata) {
            Ok(true) => Some(metadata),
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
    match process::is_same_process(metadata.daemon_pid, metadata.daemon_start_time) {
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
    let result = remove_if_present(&path)
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

pub fn table(sessions: &[SessionMetadata], width: usize) -> io::Result<String> {
    if sessions.is_empty() {
        return Ok("No climon2 sessions running.\n".to_owned());
    }
    let command_width = sessions
        .iter()
        .map(|session| escaped(&session.command).width())
        .max()
        .unwrap_or(0)
        .max("COMMAND".len());
    let mut output = format!(
        "{:<36}  {:<19}  {:<command_width$}  ARGS\n",
        "SESSION", "CREATED", "COMMAND"
    );
    for metadata in sessions {
        let command = escaped(&metadata.command);
        let padding = command_width.saturating_sub(command.width());
        let prefix = format!(
            "{}  {}  {}{}  ",
            metadata.session_id,
            local_created(metadata)?,
            command,
            " ".repeat(padding)
        );
        let arguments = metadata
            .args
            .iter()
            .map(|arg| quoted(arg))
            .collect::<Vec<_>>()
            .join(" ");
        output.push_str(&prefix);
        output.push_str(&truncate(&arguments, width.saturating_sub(prefix.width())));
        output.push('\n');
    }
    Ok(output)
}
