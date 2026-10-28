//! Content-free diagnostics; callers log lifecycle facts, never session data.

use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};
use tracing_subscriber::{EnvFilter, fmt::MakeWriter};

pub const LOG_LIMIT: usize = 8 * 1024 * 1024;
const LIMIT_NOTICE: &[u8] = b"\nlog limit reached\n";

struct State<W> {
    sink: W,
    written: usize,
    capped: bool,
}

pub struct CappedWriter<W>(Arc<Mutex<State<W>>>);

impl<W> Clone for CappedWriter<W> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<W> CappedWriter<W> {
    pub fn new(sink: W) -> Self {
        Self(Arc::new(Mutex::new(State {
            sink,
            written: 0,
            capped: false,
        })))
    }
}

impl<W: Write> State<W> {
    fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let remaining = (LOG_LIMIT - LIMIT_NOTICE.len()).saturating_sub(self.written);
        let count = bytes.len().min(remaining);
        self.sink.write_all(&bytes[..count])?;
        self.written += count;
        if bytes.len() > remaining {
            self.sink.write_all(LIMIT_NOTICE)?;
            self.written += LIMIT_NOTICE.len();
            self.capped = true;
        }
        Ok(())
    }
}

impl<W: Write> Write for CappedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self
            .0
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?;
        if !state.capped {
            state.append(bytes)?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log lock poisoned"))?
            .sink
            .flush()
    }
}

impl<'a, W: Write + 'a> MakeWriter<'a> for CappedWriter<W> {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

pub fn default_filter() -> &'static str {
    if cfg!(debug_assertions) {
        "trace"
    } else {
        "info"
    }
}

pub fn subscriber<W: Write + Send + 'static>(
    writer: CappedWriter<W>,
    filter: Option<&str>,
) -> io::Result<tracing::Dispatch> {
    let filter = EnvFilter::try_new(filter.unwrap_or(default_filter()))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid CLIMON2_LOG filter"))?;
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339())
        .with_writer(writer)
        .finish();
    Ok(tracing::Dispatch::new(subscriber))
}
