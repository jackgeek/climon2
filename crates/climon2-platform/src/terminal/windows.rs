//! The client's console on Windows (#321; ADR 0003, "Client console";
//! docs/for-humans/windows/lifecycle.md, "Client console").
//!
//! `exec` and `attach` need console handles for stdin and stdout. Raw mode
//! turns off processed, line and echo input and quick edit, turns on VT
//! input, window input and extended flags, adds VT processing and no
//! automatic newline to output, and sets both code pages to UTF-8; every exit
//! path, a panic included, puts all four back.
//!
//! While a client bridges the console, one thread reads its input and one
//! writes its output, each through a bounded channel, and each signals a
//! manual-reset event the client's wait watches (ADR 0003, "Multiplexing").
//! Only key-down records become input: their characters are UTF-16, sent on
//! as UTF-8. A window-size record is a resize. Output is cut only between
//! whole UTF-8 characters.
//!
//! Console control events: Ctrl+C is a termination, as SIGINT is on Unix;
//! Ctrl+Break does nothing yet (#326); close, logoff and shutdown are a
//! hang-up, which ends the client with 129, and the handler holds the process
//! until the client has ended itself.
use crate::{
    notifier::Notifier,
    waiting::{self, Watch},
};
use climon2_core::{protocol::TerminalSize, utf8};
use std::{
    fs::File,
    io::{self, Read, Write},
    num::NonZeroU32,
    os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle},
    sync::{
        Arc, Mutex, Once,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::{
    Win32::{
        Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::{
            Console::{
                CONSOLE_SCREEN_BUFFER_INFO, CTRL_BREAK_EVENT, CTRL_C_EVENT,
                DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT, ENABLE_EXTENDED_FLAGS,
                ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_QUICK_EDIT_MODE,
                ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
                ENABLE_WINDOW_INPUT, GetConsoleCP, GetConsoleMode, GetConsoleOutputCP,
                GetConsoleScreenBufferInfo, INPUT_RECORD, KEY_EVENT, ReadConsoleInputW,
                SetConsoleCP, SetConsoleCtrlHandler, SetConsoleMode, SetConsoleOutputCP,
                WINDOW_BUFFER_SIZE_EVENT,
            },
            Threading::WaitForSingleObject,
        },
    },
    core::BOOL,
};

/// The UTF-8 code page.
const UTF8: u32 = 65001;
/// The Unix signal numbers whose termination statuses Windows events take:
/// 129 for a hang-up and 130 for an interrupt.
const HANGUP: u32 = 1;
const INTERRUPT: u32 = 2;
/// The most input records one read takes.
const RECORDS: usize = 128;
/// How long the input thread waits for input before it looks at its stop
/// flag again.
const INPUT_POLL: Duration = Duration::from_millis(50);
/// The most output one write queues, and how many writes the queue holds:
/// together the 64 KiB of output the bridge lets wait on Unix.
const OUTPUT_CHUNK: usize = 16 * 1024;
const QUEUED: usize = 4;
/// How long the end of a bridge waits for queued output to reach the console.
const DRAIN: Duration = Duration::from_secs(5);

pub fn require_ttys() -> io::Result<()> {
    if console(&io::stdin()) && console(&io::stdout()) {
        return Ok(());
    }
    Err(io::Error::other("c2 exec needs a terminal"))
}

fn console(handle: &impl AsHandle) -> bool {
    mode(handle.as_handle()).is_ok()
}

fn mode(handle: BorrowedHandle<'_>) -> io::Result<u32> {
    let mut mode = 0u32;
    // SAFETY: a borrowed handle; the call writes one u32.
    check(unsafe { GetConsoleMode(raw(handle), &mut mode) })?;
    Ok(mode)
}

fn set_mode(handle: BorrowedHandle<'_>, mode: u32) -> io::Result<()> {
    // SAFETY: a borrowed handle.
    check(unsafe { SetConsoleMode(raw(handle), mode) })
}

fn raw(handle: BorrowedHandle<'_>) -> HANDLE {
    handle.as_raw_handle()
}

fn check(result: BOOL) -> io::Result<()> {
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// An owned duplicate of the console's input; closing it never closes the
/// process's stdin.
pub fn input() -> io::Result<File> {
    let stdin = io::stdin();
    mode(stdin.as_handle())?;
    Ok(File::from(stdin.as_handle().try_clone_to_owned()?))
}

/// The console window's size. Only a console's output reports it, so this
/// asks stdout, whichever console handle it is given.
pub fn size(_terminal: &File) -> io::Result<TerminalSize> {
    window_size()
}

fn window_size() -> io::Result<TerminalSize> {
    let mut info = CONSOLE_SCREEN_BUFFER_INFO::default();
    // SAFETY: stdout's handle; the call writes one CONSOLE_SCREEN_BUFFER_INFO.
    check(unsafe { GetConsoleScreenBufferInfo(raw(io::stdout().as_handle()), &mut info) })?;
    let window = info.srWindow;
    Ok(TerminalSize {
        cols: extent(window.Left, window.Right),
        rows: extent(window.Top, window.Bottom),
    }
    .initial())
}

/// The cells from `first` to `last`, both included.
fn extent(first: i16, last: i16) -> u16 {
    u16::try_from(i32::from(last) - i32::from(first) + 1).unwrap_or_default()
}

/// Input mode for raw input: no processed, line or echo input and no quick
/// edit; VT input, window input and the extended flags quick edit needs.
fn raw_input(mode: u32) -> u32 {
    let cooked =
        ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_QUICK_EDIT_MODE;
    (mode & !cooked) | ENABLE_VIRTUAL_TERMINAL_INPUT | ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS
}

/// Output mode for raw output: VT processing, and no newline after the
/// last column.
fn raw_output(mode: u32) -> u32 {
    mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN
}

/// What raw mode changed, and owned duplicates of the handles it changed.
struct SavedConsole {
    input: File,
    output: File,
    input_mode: u32,
    output_mode: u32,
    input_page: u32,
    output_page: u32,
}

impl SavedConsole {
    fn save(terminal: &File) -> io::Result<Self> {
        let output = File::from(io::stdout().as_handle().try_clone_to_owned()?);
        Ok(Self {
            input_mode: mode(terminal.as_handle())?,
            output_mode: mode(output.as_handle())?,
            input: terminal.try_clone()?,
            output,
            // SAFETY (both): no arguments; they only read the console.
            input_page: unsafe { GetConsoleCP() },
            output_page: unsafe { GetConsoleOutputCP() },
        })
    }

    fn enter_raw(&self) -> io::Result<()> {
        set_mode(self.input.as_handle(), raw_input(self.input_mode))?;
        set_mode(self.output.as_handle(), raw_output(self.output_mode))?;
        // SAFETY (both): code pages only.
        check(unsafe { SetConsoleCP(UTF8) })?;
        check(unsafe { SetConsoleOutputCP(UTF8) })
    }

    /// Best effort: a drop or the panic hook restores, with no one to report
    /// a failure to.
    fn restore(&self) {
        let _ = set_mode(self.input.as_handle(), self.input_mode);
        let _ = set_mode(self.output.as_handle(), self.output_mode);
        // SAFETY (both): code pages only.
        unsafe {
            SetConsoleCP(self.input_page);
            SetConsoleOutputCP(self.output_page);
        }
    }
}

static ACTIVE: Mutex<Option<Arc<SavedConsole>>> = Mutex::new(None);
static PANIC_HOOK: Once = Once::new();

fn install_panic_hook() {
    PANIC_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            if let Ok(active) = ACTIVE.lock()
                && let Some(saved) = active.as_ref()
            {
                saved.restore();
            }
            previous(panic);
        }));
    });
}

pub struct RawModeGuard(Arc<SavedConsole>);

impl RawModeGuard {
    /// Enter only after Attached. Restores the console's modes and code
    /// pages, never its screen; a raw mode entered only in part is undone.
    pub fn new(terminal: &File) -> io::Result<Self> {
        install_panic_hook();
        let mut active = ACTIVE
            .lock()
            .map_err(|_| io::Error::other("terminal lock poisoned"))?;
        if active.is_some() {
            return Err(io::Error::other("raw terminal already owned"));
        }
        let saved = Arc::new(SavedConsole::save(terminal)?);
        saved.enter_raw().inspect_err(|_| saved.restore())?;
        *active = Some(saved.clone());
        Ok(Self(saved))
    }
}

impl Drop for RawModeGuard {
    fn drop(&mut self) {
        self.0.restore();
        if let Ok(mut active) = ACTIVE.lock() {
            *active = None;
        }
    }
}

/// Set by the input thread on a window-size record.
static RESIZED: AtomicBool = AtomicBool::new(false);
/// The Unix signal number of the last console control event that ends a
/// client; zero before any.
static TERMINATION: AtomicU32 = AtomicU32::new(0);

/// The console control events a process has heard of. Only the client
/// listens; the daemon is detached and has no console.
pub struct Signals;

impl Signals {
    pub fn daemon() -> io::Result<Self> {
        Ok(Self)
    }

    /// Listens for console control events with a handler routine, never the
    /// ignore flag, which the client's children would inherit.
    pub fn client() -> io::Result<Self> {
        // SAFETY: a handler routine that lives as long as the process.
        check(unsafe { SetConsoleCtrlHandler(Some(console_event), 1) })?;
        Ok(Self)
    }

    /// Whether the console was resized since the last look.
    pub fn take_resize(&self) -> bool {
        RESIZED.swap(false, Ordering::SeqCst)
    }

    /// 1 after close, logoff or shutdown and 2 after Ctrl+C, as Unix reports
    /// SIGHUP and SIGINT; never set in the daemon.
    pub fn termination_signal(&self) -> Option<u32> {
        NonZeroU32::new(TERMINATION.load(Ordering::SeqCst)).map(NonZeroU32::get)
    }
}

/// The signal a console control event stands for; none for Ctrl+Break.
fn signal(event: u32) -> Option<u32> {
    match event {
        CTRL_C_EVENT => Some(INTERRUPT),
        CTRL_BREAK_EVENT => None,
        _ => Some(HANGUP),
    }
}

/// Records the event. After a hang-up the system ends the process as soon
/// as this returns, so it never does: the client sees the hang-up, restores
/// the console and exits 129 first.
unsafe extern "system" fn console_event(event: u32) -> BOOL {
    if let Some(signal) = signal(event) {
        TERMINATION.store(signal, Ordering::SeqCst);
        if signal == HANGUP {
            thread::sleep(Duration::MAX);
        }
    }
    1
}

/// The console's input while the monitor reads it (#327): the client's
/// bridge, whose input thread also sees resizes. The monitor writes its
/// output itself, so the bridge's output thread stays idle.
pub type Input = Bridged;

/// The console while a client bridges it: input read and output written
/// without blocking, each with the wait slot that is ready when it can make
/// progress. Reading 0 bytes means the console hung up.
pub struct Bridged {
    keys: Receiver<Vec<u8>>,
    /// Received input not yet read.
    unread: Vec<u8>,
    /// Signalled while input may wait.
    typed: Arc<Notifier>,
    screen: SyncSender<Vec<u8>>,
    /// Signalled while the output queue may have room.
    room: Arc<Notifier>,
    /// The start of a character whose rest has not been written yet.
    carry: Vec<u8>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
}

impl Bridged {
    /// Starts the threads for the console's input, `terminal`, and for
    /// stdout.
    pub fn new(terminal: File) -> io::Result<Self> {
        let output = File::from(io::stdout().as_handle().try_clone_to_owned()?);
        let (typed, room) = (Arc::new(Notifier::new()?), Arc::new(Notifier::new()?));
        let stop = Arc::new(AtomicBool::new(false));
        let (keys, received) = mpsc::sync_channel(QUEUED);
        let reader = Reader {
            console: terminal,
            keys,
            typed: typed.clone(),
            stop: stop.clone(),
        };
        let reader = thread::spawn(move || reader.run());
        let (screen, chunks) = mpsc::sync_channel(QUEUED);
        let space = room.clone();
        let writer = thread::spawn(move || write_console(output, &chunks, &space));
        room.set(true);
        Ok(Self {
            keys: received,
            unread: Vec::new(),
            typed,
            screen,
            room,
            carry: Vec::new(),
            stop,
            reader: Some(reader),
            writer: Some(writer),
        })
    }

    pub fn watch_input(&self) -> Watch<'_> {
        self.typed.watch()
    }

    pub fn watch_output(&self) -> Watch<'_> {
        self.room.watch()
    }

    pub fn size(&self) -> io::Result<TerminalSize> {
        window_size()
    }

    /// Queues `chunk`; a full queue is `WouldBlock`.
    fn queue(&self, chunk: Vec<u8>) -> io::Result<()> {
        // Cleared before the send, so the output thread's next taking sets it
        // again. Spelled `bool::default()`, false: leaving it set would only
        // make the waiter spin until the thread takes a chunk, an equivalent
        // mutant.
        self.room.set(bool::default());
        self.screen
            .try_send(chunk)
            .map_err(|_| io::Error::from(io::ErrorKind::WouldBlock))?;
        self.room.set(true);
        Ok(())
    }

    /// Takes the next chunk of input; `None` when there is none yet, and an
    /// empty chunk once the console hung up.
    fn receive(&mut self) -> Option<Vec<u8>> {
        // Cleared before the look, so the input thread's next send sets it
        // again. Spelled `bool::default()`, false: leaving it set would only
        // make the waiter spin, an equivalent mutant.
        self.typed.set(bool::default());
        match self.keys.try_recv() {
            Ok(chunk) => {
                // More may wait behind it.
                self.typed.set(true);
                Some(chunk)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Vec::new()),
        }
    }

    /// Queues the start of a character that never got its rest, closes the
    /// queue and waits up to `DRAIN` for the output thread to write it all.
    fn drain(&mut self) {
        let carry = std::mem::take(&mut self.carry);
        let _ = self.queue(carry);
        self.screen = mpsc::sync_channel(QUEUED).0;
        let writer = self.writer.take().expect("one drain");
        // A thread's handle is signalled once it has ended.
        let _ = waiting::wait(&mut [Watch::event(writer.as_handle())], DRAIN);
    }
}

impl Read for Bridged {
    /// `WouldBlock` while no input waits; 0 bytes once the console hung up.
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.unread.is_empty() {
            self.unread = self.receive().ok_or(io::ErrorKind::WouldBlock)?;
        }
        let count = bytes.len().min(self.unread.len());
        bytes[..count].copy_from_slice(&self.unread[..count]);
        self.unread.drain(..count);
        Ok(count)
    }
}

impl Write for Bridged {
    /// Takes up to one chunk, queueing it up to the last whole character;
    /// the start of a character waits for its rest.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let taken = bytes.len().min(OUTPUT_CHUNK);
        let mut chunk = [self.carry.as_slice(), &bytes[..taken]].concat();
        let rest = chunk.split_off(utf8::whole_characters(&chunk));
        self.queue(chunk)?;
        self.carry = rest;
        Ok(taken)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for Bridged {
    /// Stops reading, then gives queued output its chance to be written.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Unblocks a reader waiting for room in the queue.
        self.keys = mpsc::sync_channel(QUEUED).1;
        let _ = self.reader.take().expect("one drop").join();
        self.drain();
    }
}

/// The input thread's state.
struct Reader {
    console: File,
    keys: SyncSender<Vec<u8>>,
    typed: Arc<Notifier>,
    stop: Arc<AtomicBool>,
}

impl Reader {
    /// Reads until told to stop or nothing receives. A console that fails is
    /// a hang-up, which the bridge reads as 0 bytes once this thread ends.
    fn run(self) {
        let mut decoder = Keys::default();
        while !self.stop.load(Ordering::SeqCst) {
            let Ok(records) = self.records() else {
                self.typed.set(true);
                return;
            };
            if self.forward(&records, &mut decoder).is_err() {
                return;
            }
        }
    }

    fn forward(&self, records: &[INPUT_RECORD], decoder: &mut Keys) -> Result<(), ()> {
        RESIZED.fetch_or(resized(records), Ordering::SeqCst);
        let bytes = decoder.feed(records);
        if !bytes.is_empty() {
            self.keys.send(bytes).map_err(drop)?;
            self.typed.set(true);
        }
        Ok(())
    }

    /// The records waiting, after waiting up to `INPUT_POLL` for any.
    fn records(&self) -> io::Result<Vec<INPUT_RECORD>> {
        let handle = raw(self.console.as_handle());
        let milliseconds = u32::try_from(INPUT_POLL.as_millis()).unwrap_or_default();
        // SAFETY: the console's input handle, which this thread owns.
        match unsafe { WaitForSingleObject(handle, milliseconds) } {
            WAIT_OBJECT_0 => read_records(handle),
            WAIT_TIMEOUT => Ok(Vec::new()),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

fn read_records(handle: HANDLE) -> io::Result<Vec<INPUT_RECORD>> {
    let mut records = vec![INPUT_RECORD::default(); RECORDS];
    let mut count = 0u32;
    // SAFETY: room for `RECORDS` records; the call writes `count` of them.
    check(unsafe { ReadConsoleInputW(handle, records.as_mut_ptr(), RECORDS as u32, &mut count) })?;
    records.truncate(count as usize);
    Ok(records)
}

/// Whether any record is a window-size record.
fn resized(records: &[INPUT_RECORD]) -> bool {
    records
        .iter()
        .any(|record| u32::from(record.EventType) == WINDOW_BUFFER_SIZE_EVENT)
}

/// The characters a key-down record types, once per repeat; none for any
/// other record.
fn typed(record: &INPUT_RECORD) -> Vec<u16> {
    if u32::from(record.EventType) != KEY_EVENT {
        return Vec::new();
    }
    // SAFETY: a key record's active member is its key event, whose character
    // is UTF-16 under ReadConsoleInputW.
    let (key, unit) = unsafe {
        (
            record.Event.KeyEvent,
            record.Event.KeyEvent.uChar.UnicodeChar,
        )
    };
    let count = if key.bKeyDown != 0 && unit != 0 {
        key.wRepeatCount
    } else {
        0
    };
    vec![unit; usize::from(count)]
}

/// Turns key records into UTF-8, keeping a high surrogate that ends one read
/// for the low surrogate that starts the next.
#[derive(Default)]
struct Keys {
    high: Option<u16>,
}

impl Keys {
    fn feed(&mut self, records: &[INPUT_RECORD]) -> Vec<u8> {
        let mut units: Vec<u16> = self.high.take().into_iter().collect();
        units.extend(records.iter().flat_map(typed));
        if units
            .last()
            .is_some_and(|unit| (0xD800..0xDC00).contains(unit))
        {
            self.high = units.pop();
        }
        String::from_utf16_lossy(&units).into_bytes()
    }
}

/// The output thread: writes each queued chunk, in full, until the queue
/// closes. Taking a chunk makes room. Once the console fails, chunks are
/// only taken: a failed console has hung up, which its input reports.
fn write_console(mut console: File, chunks: &Receiver<Vec<u8>>, room: &Notifier) {
    let mut working = true;
    for chunk in chunks {
        room.set(true);
        working = working && console.write_all(&chunk).is_ok();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Console::{CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT};

    fn key(unit: u16, down: bool, repeat: u16) -> INPUT_RECORD {
        let mut record = INPUT_RECORD {
            EventType: KEY_EVENT as u16,
            ..INPUT_RECORD::default()
        };
        record.Event.KeyEvent.bKeyDown = BOOL::from(down);
        record.Event.KeyEvent.wRepeatCount = repeat;
        record.Event.KeyEvent.uChar.UnicodeChar = unit;
        record
    }

    fn resize() -> INPUT_RECORD {
        INPUT_RECORD {
            EventType: WINDOW_BUFFER_SIZE_EVENT as u16,
            ..INPUT_RECORD::default()
        }
    }

    #[test]
    fn only_key_down_characters_are_typed_once_per_repeat() {
        let mut keys = Keys::default();
        let records = [
            key(u16::from(b'a'), true, 1),
            key(u16::from(b'a'), false, 1),
            key(u16::from(b'b'), true, 3),
            // A modifier alone types nothing.
            key(0, true, 1),
            resize(),
            key(0x03, true, 1),
            key(0x03, false, 1),
        ];
        assert_eq!(keys.feed(&records), b"abbb\x03");
    }

    #[test]
    fn a_surrogate_pair_split_across_reads_is_one_character() {
        let mut keys = Keys::default();
        assert_eq!(
            keys.feed(&[key(u16::from(b'x'), true, 1), key(0xD83D, true, 1)]),
            b"x"
        );
        assert_eq!(keys.feed(&[key(0xDE00, true, 1)]), "\u{1F600}".as_bytes());
        // A lone surrogate is a replacement character, not held for good.
        assert_eq!(keys.feed(&[key(0xDE00, true, 1)]), "\u{FFFD}".as_bytes());
        assert_eq!(keys.feed(&[key(0xD83D, true, 1)]), b"");
        assert_eq!(
            keys.feed(&[key(u16::from(b'y'), true, 1)]),
            "\u{FFFD}y".as_bytes()
        );
        assert_eq!(keys.feed(&[key(0xDBFF, true, 1)]), b"");
        assert_eq!(keys.feed(&[key(0xDFFF, true, 1)]), "\u{10FFFF}".as_bytes());
        assert_eq!(keys.feed(&[key(0xDC00, true, 1)]), "\u{FFFD}".as_bytes());
    }

    #[test]
    fn a_window_size_record_is_a_resize() {
        assert!(resized(&[key(u16::from(b'a'), true, 1), resize()]));
        assert!(!resized(&[key(u16::from(b'a'), true, 1)]));
    }

    #[test]
    fn raw_modes_change_only_the_agreed_flags() {
        // Insert mode and mouse input are left alone; quick edit goes.
        let cooked = 0x1F7;
        assert_eq!(raw_input(cooked), 0x3B8);
        assert_eq!(raw_input(0), 0x288);
        // Processed output and wrapping stay.
        assert_eq!(raw_output(0x3), 0xF);
        assert_eq!(raw_output(0), 0xC);
    }

    #[test]
    fn an_extent_counts_both_ends() {
        assert_eq!(extent(0, 79), 80);
        assert_eq!(extent(5, 5), 1);
        assert_eq!(extent(10, 9), 0);
        assert_eq!(extent(0, i16::MAX), 32768);
        assert_eq!(extent(i16::MIN, i16::MAX), 0);
    }

    #[test]
    fn a_resize_is_taken_once() {
        RESIZED.store(true, Ordering::SeqCst);
        assert!(Signals.take_resize());
        assert!(!Signals.take_resize());
    }

    #[test]
    fn console_events_stand_for_unix_signals() {
        assert_eq!(signal(CTRL_C_EVENT), Some(2));
        assert_eq!(signal(CTRL_BREAK_EVENT), None);
        assert_eq!(signal(CTRL_CLOSE_EVENT), Some(1));
        assert_eq!(signal(CTRL_LOGOFF_EVENT), Some(1));
    }
}
