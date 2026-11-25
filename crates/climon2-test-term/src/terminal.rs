//! The emulated host terminal shared by every platform: its screen, the
//! bytes it received, its replies and the waits on them. How a program is
//! attached to it is per platform: a Unix PTY (`unix`) or a Windows
//! pseudoconsole created directly with the pinned ConPTY (`crate::windows`).

use crate::{
    cell::{Cell, Cursor},
    input::{Host, Mouse},
    signals::{HostSignal, Signals},
};
use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    index::{Column, Line},
    term::{Config, Osc52, TermMode},
    vte::ansi::Processor,
};
use std::{
    io::{self, Read, Write},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(unix)]
mod unix;

#[cfg(windows)]
pub use crate::windows::Terminal;
#[cfg(unix)]
pub use unix::{InputTerminal, Terminal, harness_shell};

/// How many of the latest output bytes a terminal keeps for
/// [`Terminal::received`]: enough for a test's whole run of monitor
/// repaints, toast animations (#115) included, so a check of everything
/// the host received (such as "the outer shell's title was the only OSC")
/// still sees its start.
const RECENT_BYTES: usize = 16 * 1024 * 1024;
/// How many of the latest output bytes a failure shows.
const FAILURE_BYTES: usize = 64 * 1024;

/// The last `bytes` of `recent`, or all of it.
fn tail(recent: &[u8], bytes: usize) -> &[u8] {
    &recent[recent.len().saturating_sub(bytes)..]
}

struct Size {
    rows: usize,
    cols: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

pub(crate) struct Events(Sender<Event>);
impl EventListener for Events {
    fn send_event(&self, event: Event) {
        self.0.send(event).expect("terminal event receiver");
    }
}

/// How the emulated host answers and how much native scrollback it keeps.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    /// Whether the host answers queries (`Event::PtyWrite`) like a real terminal.
    pub replies: bool,
    /// Native scrollback rows kept by the host.
    pub history: usize,
    /// How many of the latest output bytes the terminal keeps for
    /// [`Terminal::received`] and the byte waits; at most this many are
    /// moved when older bytes are dropped.
    pub recent: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            replies: true,
            history: 10_000,
            recent: RECENT_BYTES,
        }
    }
}

/// The host terminal's mode state, for release and restoration assertions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Modes {
    pub alternate_screen: bool,
    pub cursor_visible: bool,
    /// Any mouse tracking or SGR mouse encoding mode.
    pub mouse: bool,
    /// Button-event (press, release and drag) mouse tracking, DEC 1002.
    pub mouse_drag: bool,
    /// Any-event (all motion) mouse tracking, DEC 1003.
    pub mouse_motion: bool,
    /// SGR extended mouse encoding, DEC 1006.
    pub sgr_mouse: bool,
    pub focus: bool,
    pub bracketed_paste: bool,
    pub application_cursor: bool,
    pub application_keypad: bool,
    pub autowrap: bool,
    pub origin: bool,
    pub insert: bool,
    pub line_feed_new_line: bool,
}

impl Modes {
    /// Plan 1.7.1's documented normal-shell baseline.
    pub const BASELINE: Self = Self {
        alternate_screen: false,
        cursor_visible: true,
        mouse: false,
        mouse_drag: false,
        mouse_motion: false,
        sgr_mouse: false,
        focus: false,
        bracketed_paste: false,
        application_cursor: false,
        application_keypad: false,
        autowrap: true,
        origin: false,
        insert: false,
        line_feed_new_line: false,
    };

    pub(crate) fn of(mode: TermMode) -> Self {
        Self {
            alternate_screen: mode.contains(TermMode::ALT_SCREEN),
            cursor_visible: mode.contains(TermMode::SHOW_CURSOR),
            mouse: mode.intersects(TermMode::MOUSE_MODE | TermMode::SGR_MOUSE),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            focus: mode.contains(TermMode::FOCUS_IN_OUT),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            application_cursor: mode.contains(TermMode::APP_CURSOR),
            application_keypad: mode.contains(TermMode::APP_KEYPAD),
            autowrap: mode.contains(TermMode::LINE_WRAP),
            origin: mode.contains(TermMode::ORIGIN),
            insert: mode.contains(TermMode::INSERT),
            line_feed_new_line: mode.contains(TermMode::LINE_FEED_NEW_LINE),
        }
    }
}

pub(crate) struct Screen {
    pub(crate) term: Term<Events>,
    replies: bool,
    title: Option<String>,
    /// Queries and clipboard operations the host was asked to perform.
    requests: Vec<String>,
    signals: Signals,
    recent: Vec<u8>,
    /// How many output bytes arrived in all, kept or not.
    total: u64,
    pub(crate) eof: bool,
    /// The reader takes nothing more until this is false again
    /// ([`Terminal::stop_reading`]).
    paused: bool,
    error: Option<io::Error>,
    #[cfg_attr(not(unix), allow(dead_code))]
    pub(crate) last_output: Instant,
}

impl Screen {
    fn text(&self) -> String {
        let grid = self.term.grid();
        (0..grid.screen_lines())
            .map(|row| self.row(row as i32))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn row(&self, line: i32) -> String {
        let grid = self.term.grid();
        (0..grid.columns())
            .map(|col| grid[Line(line)][Column(col)].c)
            .collect()
    }

    fn cells(&self, line: i32) -> Vec<Cell> {
        let grid = self.term.grid();
        (0..grid.columns())
            .map(|col| Cell::of(&grid[Line(line)][Column(col)]))
            .collect()
    }

    /// Native scrollback rows, oldest first, without trailing blanks.
    fn history(&self) -> Vec<String> {
        let rows = self.term.grid().history_size() as i32;
        (1..=rows)
            .rev()
            .map(|row| self.row(-row).trim_end().to_owned())
            .collect()
    }

    fn handle(&mut self, event: Event, writer: &Writer) -> io::Result<()> {
        match event {
            Event::Title(title) => self.title = Some(title),
            Event::ResetTitle => self.title = None,
            Event::PtyWrite(reply) => {
                self.requests.push(format!("reply {reply:?}"));
                if self.replies {
                    write(writer, reply.as_bytes())?;
                }
            }
            Event::ClipboardStore(..)
            | Event::ClipboardLoad(..)
            | Event::ColorRequest(..)
            | Event::TextAreaSizeRequest(..) => self.requests.push(format!("{event:?}")),
            _ => (),
        }
        Ok(())
    }

    pub(crate) fn failure(&self, error: io::Error) -> io::Error {
        io::Error::new(
            error.kind(),
            format!(
                "{error}\nSCREEN:\n{}\nHOST SIGNALS:\n{:?}\nRECENT BYTES:\n{:?}",
                self.text(),
                self.signals.recorded(),
                String::from_utf8_lossy(tail(&self.recent, FAILURE_BYTES))
            ),
        )
    }

    pub(crate) fn check(&self) -> io::Result<()> {
        match &self.error {
            Some(error) => Err(self.failure(io::Error::new(error.kind(), error.to_string()))),
            None => Ok(()),
        }
    }

    /// Resizes the emulated screen once its program's terminal was resized.
    pub(crate) fn resize(&mut self, rows: u16, cols: u16) {
        self.term.resize(Size {
            rows: rows.into(),
            cols: cols.into(),
        });
    }
}

pub(crate) type Writer = Arc<Mutex<Box<dyn Write + Send>>>;
pub(crate) type SharedScreen = Arc<(Mutex<Screen>, Condvar)>;

/// A fresh emulated screen and the receiver of the events its parser raises.
pub(crate) fn new_screen(
    rows: u16,
    cols: u16,
    options: Options,
) -> (SharedScreen, Receiver<Event>) {
    let (sender, events) = mpsc::channel();
    let screen = Arc::new((
        Mutex::new(Screen {
            term: Term::new(
                Config {
                    scrolling_history: options.history,
                    osc52: Osc52::CopyPaste,
                    ..Config::default()
                },
                &Size {
                    rows: rows.into(),
                    cols: cols.into(),
                },
                Events(sender),
            ),
            replies: options.replies,
            title: None,
            requests: Vec::new(),
            signals: Signals::default(),
            recent: Vec::new(),
            total: 0,
            eof: false,
            paused: false,
            error: None,
            last_output: Instant::now(),
        }),
        Condvar::new(),
    ));
    (screen, events)
}

/// What [`spawn_reader`] reads from and answers through.
pub(crate) struct Feed {
    pub(crate) input: Box<dyn Read + Send>,
    pub(crate) screen: SharedScreen,
    pub(crate) writer: Writer,
    pub(crate) events: Receiver<Event>,
    /// How many of the latest bytes to keep ([`Options::recent`]).
    pub(crate) recent: usize,
    /// Which read errors mean end of output rather than a failure.
    pub(crate) hang_up: fn(&io::Error) -> bool,
}

/// Feeds everything read from the input to the screen until end of output or
/// an error. `tap` sees every chunk right after the screen parsed it.
pub(crate) fn spawn_reader(
    feed: Feed,
    mut tap: impl FnMut(&[u8]) + Send + 'static,
) -> JoinHandle<()> {
    let Feed {
        mut input,
        screen: reader_screen,
        writer: reader_writer,
        events,
        recent,
        hang_up,
    } = feed;
    thread::spawn(move || {
        let (screen, changed) = &*reader_screen;
        let mut parser: Processor = Processor::new();
        let mut buffer = [0; 4096];
        loop {
            drop(
                changed
                    .wait_while(screen.lock().expect("screen lock"), |state| state.paused)
                    .expect("screen wait"),
            );
            let result = input.read(&mut buffer);
            let mut state = screen.lock().expect("screen lock");
            match result {
                Ok(0) => state.eof = true,
                Ok(count) => {
                    parser.advance(&mut state.term, &buffer[..count]);
                    tap(&buffer[..count]);
                    state.signals.push(&buffer[..count]);
                    state.recent.extend_from_slice(&buffer[..count]);
                    state.total += count as u64;
                    state.last_output = Instant::now();
                    let excess = state.recent.len().saturating_sub(recent);
                    state.recent.drain(..excess);
                    while let Ok(event) = events.try_recv() {
                        if let Err(error) = state.handle(event, &reader_writer) {
                            state.error = Some(error);
                            break;
                        }
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if hang_up(&error) => state.eof = true,
                Err(error) => state.error = Some(error),
            }
            changed.notify_all();
            if state.eof || state.error.is_some() {
                break;
            }
        }
    })
}

/// The screen and its input, which every platform's [`Terminal`] holds.
impl Terminal {
    pub fn send(&mut self, bytes: impl AsRef<[u8]>) -> io::Result<()> {
        write(&self.writer, bytes.as_ref())
            .map_err(|error| self.screen.0.lock().expect("screen lock").failure(error))
    }

    /// Reports host focus gained (`CSI I`) or lost (`CSI O`), as a real
    /// terminal does only while the application has focus reporting
    /// (`CSI ? 1004 h`) on.
    pub fn focus(&mut self, focused: bool) -> io::Result<()> {
        self.inject(|host| host.focus(focused))
    }

    /// Reports a mouse `action` at the visible-screen cell (`row`, `column`),
    /// 0-based, in SGR encoding (`CSI < Cb ; Cx ; Cy M` or `m`, 1-based), as a
    /// real terminal does only with SGR encoding (`CSI ? 1006 h`) and a
    /// tracking mode that reports the action on.
    pub fn mouse(&mut self, action: Mouse, row: usize, column: usize) -> io::Result<()> {
        self.inject(|host| host.mouse(action, row, column))
    }

    /// Pastes `text` unchanged between `CSI 200 ~` and `CSI 201 ~`, as a real
    /// terminal does while the application has bracketed paste
    /// (`CSI ? 2004 h`) on; send unbracketed paste with [`Terminal::send`].
    pub fn paste(&mut self, text: impl AsRef<[u8]>) -> io::Result<()> {
        self.inject(|host| host.paste(text.as_ref()))
    }

    /// Sends the input the emulated host makes of an event, or nothing, failing
    /// with [`io::ErrorKind::InvalidInput`], when the host would not send it.
    pub(crate) fn inject(
        &mut self,
        encode: impl FnOnce(&Host) -> Result<Vec<u8>, String>,
    ) -> io::Result<()> {
        let bytes = {
            let screen = self.screen.0.lock().expect("screen lock");
            let host = Host {
                mode: *screen.term.mode(),
                rows: screen.term.screen_lines(),
                columns: screen.term.columns(),
            };
            encode(&host).map_err(|reason| {
                screen.failure(io::Error::new(io::ErrorKind::InvalidInput, reason))
            })?
        };
        self.send(bytes)
    }

    pub fn screen(&self) -> String {
        self.screen.0.lock().expect("screen lock").text()
    }

    /// Stops reading what the programs on the terminal write, as a host
    /// that stops reading does: once the terminal's buffer is full, their
    /// writes wait. What the reader already took is still parsed.
    pub fn stop_reading(&self) {
        self.screen.0.lock().expect("screen lock").paused = true;
    }

    /// Reads what the programs write again, from where reading stopped.
    pub fn resume_reading(&self) {
        let (screen, changed) = &*self.screen;
        screen.lock().expect("screen lock").paused = false;
        changed.notify_all();
    }

    /// The visible screen row `row` (0-based), with every column.
    pub fn row(&self, row: usize) -> String {
        self.screen.0.lock().expect("screen lock").row(row as i32)
    }

    /// The visible screen row `row` (0-based), cell by cell with every column.
    pub fn cells(&self, row: usize) -> Vec<Cell> {
        self.screen.0.lock().expect("screen lock").cells(row as i32)
    }

    pub fn history(&self) -> Vec<String> {
        self.screen.0.lock().expect("screen lock").history()
    }

    /// The host's window title; `None` before any title or after a reset.
    pub fn title(&self) -> Option<String> {
        self.screen.0.lock().expect("screen lock").title.clone()
    }

    pub fn modes(&self) -> Modes {
        Modes::of(*self.screen.0.lock().expect("screen lock").term.mode())
    }

    /// The cursor's position, visibility, shape and blink, read together.
    pub fn cursor(&self) -> Cursor {
        Cursor::of(&self.screen.0.lock().expect("screen lock").term)
    }

    /// Every query and clipboard operation the host was asked to perform.
    pub fn host_requests(&self) -> Vec<String> {
        self.screen.0.lock().expect("screen lock").requests.clone()
    }

    /// Every OSC 7501 report and bell the host received, in the order its parser
    /// acted on them.
    pub fn host_signals(&self) -> Vec<HostSignal> {
        self.screen
            .0
            .lock()
            .expect("screen lock")
            .signals
            .recorded()
            .to_vec()
    }

    /// The most recent output bytes (at most [`Options::recent`], 16 MiB by
    /// default).
    pub fn received(&self) -> Vec<u8> {
        self.screen.0.lock().expect("screen lock").recent.clone()
    }

    /// How many output bytes arrived in all: unchanged between two calls,
    /// nothing arrived between them. Cheaper than comparing
    /// [`Terminal::received`] twice.
    pub fn received_total(&self) -> u64 {
        self.screen.0.lock().expect("screen lock").total
    }

    /// At most the last `bytes` output bytes.
    pub fn received_tail(&self, bytes: usize) -> Vec<u8> {
        let state = self.screen.0.lock().expect("screen lock");
        tail(&state.recent, bytes).to_vec()
    }

    pub fn wait_for_text(&mut self, text: &str, timeout: Duration) -> io::Result<()> {
        self.wait(timeout, |screen| screen.text().contains(text))
    }

    /// Waits until `condition` holds for the visible screen text.
    pub fn wait_for_screen(
        &mut self,
        timeout: Duration,
        condition: impl Fn(&str) -> bool,
    ) -> io::Result<()> {
        self.wait(timeout, |screen| condition(&screen.text()))
    }

    /// Waits until `bytes` arrive among those received after the first
    /// `mark` ([`Terminal::received_total`] earlier), searching only those
    /// that are still kept.
    pub fn wait_for_bytes_since(
        &self,
        mark: u64,
        bytes: &[u8],
        timeout: Duration,
    ) -> io::Result<()> {
        self.wait(timeout, |screen| {
            let since = usize::try_from(screen.total.saturating_sub(mark)).unwrap_or(usize::MAX);
            let kept = &screen.recent[screen.recent.len().saturating_sub(since)..];
            bytes.is_empty() || kept.windows(bytes.len()).any(|window| window == bytes)
        })
    }

    pub fn wait_for_bytes(&mut self, bytes: &[u8], timeout: Duration) -> io::Result<()> {
        self.wait(timeout, |screen| {
            bytes.is_empty()
                || screen
                    .recent
                    .windows(bytes.len())
                    .any(|window| window == bytes)
        })
    }

    /// Waits until `condition` holds for every host signal received so far.
    pub fn wait_for_host_signals(
        &mut self,
        timeout: Duration,
        condition: impl Fn(&[HostSignal]) -> bool,
    ) -> io::Result<()> {
        self.wait(timeout, |screen| condition(screen.signals.recorded()))
    }

    fn wait(&self, timeout: Duration, condition: impl Fn(&Screen) -> bool) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        let (screen, changed) = &*self.screen;
        let mut state = screen.lock().expect("screen lock");
        loop {
            state.check()?;
            if condition(&state) {
                return Ok(());
            }
            if Instant::now() >= deadline || state.eof {
                return Err(state.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "terminal expectation not met",
                )));
            }
            (state, _) = changed
                .wait_timeout(state, deadline.saturating_duration_since(Instant::now()))
                .expect("screen wait");
        }
    }
}

pub(crate) fn write(writer: &Writer, bytes: &[u8]) -> io::Result<()> {
    let mut writer = writer.lock().expect("PTY writer lock");
    writer.write_all(bytes).and_then(|()| writer.flush())
}

/// Refuses a terminal smaller than two rows or columns.
pub(crate) fn check_size(rows: u16, cols: u16) -> io::Result<()> {
    if rows < 2 || cols < 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "PTY needs at least 2 rows and columns",
        ));
    }
    Ok(())
}
