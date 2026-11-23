//! The session's terminal on Windows (#317; ADR 0003, "Windows I/O and
//! transport"; docs/for-humans/windows/lifecycle.md, "Starting the command" and
//! "Session job and hang-up").
//!
//! The daemon hosts its command in a pseudoconsole of the pinned, bundled
//! ConPTY: `conpty.dll` loaded by full path from the daemon's own folder,
//! beside `OpenConsole.exe`. The command, resolved and quoted by
//! [`crate::command`], starts with one `CreateProcessW`
//! whose attribute list holds the pseudoconsole and the session job, with
//! handle inheritance off and null standard handles, after the daemon set the
//! session's variables in its own environment and cleared any inherited
//! "ignore Ctrl+C". The pseudoconsole is released at once, so its output pipe
//! breaks when the last process leaves it. The command's end is learnt from
//! the leader's process handle, never from the job.
//!
//! One thread per blocking handle: one reads the output pipe and one writes
//! the input pipe, each through a bounded channel, so a full channel behaves
//! as a full Unix PTY buffer does.
//!
//! The pseudoconsole starts its output with its own negotiation with a host
//! (#325, `WIN-08`). The output thread drops it: a session's clients, and the
//! terminal model that restores their screens, see only the command's own
//! output. The input thread answers its device attributes request first
//! (#328), because the pseudoconsole takes the first answer it reads as its
//! host's, and would otherwise take the one meant for the command.
use crate::command::Launch;
use std::{
    cell::RefCell,
    ffi::OsStr,
    io::{self, PipeReader, PipeWriter, Write},
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    },
    path::Path,
    ptr,
    sync::mpsc::{self, Receiver, SyncSender, TrySendError},
    time::Duration,
};
use windows_sys::{
    Win32::{
        Foundation::{ERROR_BROKEN_PIPE, FALSE, HANDLE, STATUS_CONTROL_C_EXIT, WAIT_OBJECT_0},
        Storage::FileSystem::ReadFile,
        System::{
            Console::{COORD, SetConsoleCtrlHandler},
            Environment::SetEnvironmentVariableW,
            JobObjects::{
                CreateJobObjectW, JOB_OBJECT_LIMIT_BREAKAWAY_OK,
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
            },
            LibraryLoader::{
                GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32,
                LoadLibraryExW,
            },
            Threading::{
                CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
                GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
                LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
                STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
            },
        },
    },
    core::BOOL,
};

/// The most output one read takes, and the most input one write queues.
const CHUNK: usize = 32 * 1024;
/// Chunks a channel holds before its writer waits (output) or input is
/// refused as `WouldBlock`.
const QUEUED: usize = 4;
/// `ConptyCreatePseudoConsole` flags: none, so the pseudoconsole neither
/// asks for the cursor position nor changes how it measures widths.
const NO_FLAGS: u32 = 0;

/// A read failing because the last process left the pseudoconsole
/// (`ERROR_BROKEN_PIPE`): the end of the command's output.
pub fn output_ended(error: &io::Error) -> bool {
    error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32)
}

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut isize) -> i32;
type ResizeFn = unsafe extern "system" fn(isize, COORD) -> i32;
type ReleaseFn = unsafe extern "system" fn(isize) -> i32;
type CloseFn = unsafe extern "system" fn(isize);
/// What `GetProcAddress` returns for any export.
type Export = unsafe extern "system" fn() -> isize;

const EXPORTS: [&[u8]; 4] = [
    b"ConptyCreatePseudoConsole\0",
    b"ConptyResizePseudoConsole\0",
    b"ConptyReleasePseudoConsole\0",
    b"ConptyClosePseudoConsole\0",
];

/// The bundled ConPTY's pseudoconsole calls. The library is loaded once and
/// never freed.
#[derive(Clone, Copy)]
pub struct ConPty {
    create: CreateFn,
    resize: ResizeFn,
    release: ReleaseFn,
    close: CloseFn,
}

impl ConPty {
    /// Loads `conpty.dll` from `folder` by full path, its own imports limited
    /// to that folder and System32. `OpenConsole.exe` must be beside it:
    /// without it `conpty.dll` would fall back silently to the inbox console
    /// host, which ADR 0007 forbids.
    pub fn load(folder: &Path) -> io::Result<Self> {
        let console = folder.join("OpenConsole.exe");
        if !console.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{} is missing", console.display()),
            ));
        }
        library(&folder.join("conpty.dll")).and_then(Self::exports)
    }

    fn exports(module: RawHandle) -> io::Result<Self> {
        let exports: Vec<Export> = EXPORTS
            .iter()
            .map(|name| export(module, name))
            .collect::<io::Result<_>>()?;
        let [create, resize, release, close] = <[Export; 4]>::try_from(exports)
            .unwrap_or_else(|_| unreachable!("one export per name"));
        // Each export has the documented signature of the type it becomes.
        unsafe {
            Ok(Self {
                create: std::mem::transmute::<Export, CreateFn>(create),
                resize: std::mem::transmute::<Export, ResizeFn>(resize),
                release: std::mem::transmute::<Export, ReleaseFn>(release),
                close: std::mem::transmute::<Export, CloseFn>(close),
            })
        }
    }
}

fn library(path: &Path) -> io::Result<RawHandle> {
    let path = wide(path.as_os_str());
    // The path is NUL-terminated and absolute.
    let module = unsafe {
        LoadLibraryExW(
            path.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return Err(io::Error::last_os_error());
    }
    Ok(module)
}

fn export(module: RawHandle, name: &[u8]) -> io::Result<Export> {
    // `name` is a NUL-terminated export name of the loaded `module`.
    unsafe { GetProcAddress(module, name.as_ptr()) }.ok_or_else(io::Error::last_os_error)
}

/// A pseudoconsole and the session job, ready for the command.
pub struct Opened {
    console: Console,
    input: PipeWriter,
    output: PipeReader,
}

/// Creates the session job, which allows breakaway, so a nested `c2 exec`
/// can start its own daemon outside it, and kills whatever is left in it
/// when its last handle closes; never `SILENT_BREAKAWAY_OK`. Then the
/// pseudoconsole, `rows` by `cols`.
pub fn open(conpty: ConPty, rows: u16, cols: u16) -> io::Result<Opened> {
    let job = session_job()?;
    let ((console_input, input), (output, console_output)) = pipes()?;
    let mut handle = isize::default();
    // Both pipe ends are open; the pseudoconsole keeps its own copies, so
    // ours close when this returns. The call writes one HPCON.
    hresult(unsafe {
        (conpty.create)(
            size(rows, cols),
            console_input.as_raw_handle(),
            console_output.as_raw_handle(),
            NO_FLAGS,
            &mut handle,
        )
    })?;
    Ok(Opened {
        console: Console {
            conpty,
            handle: Some(handle),
            job,
        },
        input,
        output,
    })
}

/// The pseudoconsole's input pipe, then its output pipe.
type Pipes = ((PipeReader, PipeWriter), (PipeReader, PipeWriter));

fn pipes() -> io::Result<Pipes> {
    Ok((io::pipe()?, io::pipe()?))
}

fn session_job() -> io::Result<OwnedHandle> {
    // An anonymous job with default security.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // `raw` is a new job handle that nothing else owns.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags =
        JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // The call reads one JOBOBJECT_EXTENDED_LIMIT_INFORMATION of the size given.
    check(unsafe {
        SetInformationJobObject(
            raw,
            JobObjectExtendedLimitInformation,
            (&raw const limits).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    })?;
    Ok(job)
}

impl Opened {
    /// Starts `launch` with one `CreateProcessW` in `cwd`, on the
    /// pseudoconsole and in the session job from its first instruction, then
    /// releases the pseudoconsole. `environment` is set in this process's own
    /// environment first, which the command inherits.
    pub fn start(
        self,
        launch: &Launch,
        cwd: &Path,
        environment: &[(&str, &str)],
    ) -> io::Result<Hosted> {
        prepare(environment)?;
        let (leader, pid) = self.console.spawn(launch, cwd)?;
        self.console.release()?;
        Ok(Hosted::new(self, leader, pid))
    }
}

/// Sets the session's variables in this process, then clears any "ignore
/// Ctrl+C" this process inherited, which the command would inherit in turn.
/// A process without a console may still clear it.
fn prepare(environment: &[(&str, &str)]) -> io::Result<()> {
    for (name, value) in environment {
        let (name, value) = (wide(OsStr::new(name)), wide(OsStr::new(value)));
        // Both strings are NUL-terminated.
        check(unsafe { SetEnvironmentVariableW(name.as_ptr(), value.as_ptr()) })?;
    }
    // Clearing the flag reads nothing.
    check(unsafe { SetConsoleCtrlHandler(None, FALSE) })
}

/// The pseudoconsole, until closed, and the session job.
struct Console {
    conpty: ConPty,
    handle: Option<isize>,
    job: OwnedHandle,
}

impl Console {
    /// Handle inheritance is off and there is no handle list (Windows refuses
    /// an empty one); null standard handles make the command take the
    /// pseudoconsole's.
    fn spawn(&self, launch: &Launch, cwd: &Path) -> io::Result<(OwnedHandle, u32)> {
        let job = self.job.as_raw_handle();
        let console = self.handle.unwrap_or_default();
        let mut attributes = Attributes::new(2)?;
        // This attribute's value is the HPCON itself, not a pointer to it.
        attributes.update(
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            console as *const std::ffi::c_void,
            size_of::<isize>(),
        )?;
        attributes.update(
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
            (&raw const job).cast(),
            size_of::<HANDLE>(),
        )?;
        create(&mut attributes, launch, cwd)
    }

    fn release(&self) -> io::Result<()> {
        let handle = self.handle.unwrap_or_default();
        // The pseudoconsole is open until `close`.
        hresult(unsafe { (self.conpty.release)(handle) })
    }

    /// A closed pseudoconsole has nothing left to resize.
    fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        self.handle.map_or(Ok(()), |handle| {
            // As in `release`.
            hresult(unsafe { (self.conpty.resize)(handle, size(rows, cols)) })
        })
    }

    /// Every process on the pseudoconsole gets `CTRL_CLOSE_EVENT`.
    fn close(&mut self) {
        if let Some(handle) = self.handle.take() {
            // As in `release`; the handle is not used again.
            unsafe { (self.conpty.close)(handle) }
        }
    }

    fn terminate(&self) -> io::Result<()> {
        // The job handle is open.
        check(unsafe { TerminateJobObject(self.job.as_raw_handle(), STATUS_CONTROL_C_EXIT as u32) })
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        self.close();
    }
}

fn create(
    attributes: &mut Attributes,
    launch: &Launch,
    cwd: &Path,
) -> io::Result<(OwnedHandle, u32)> {
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes.list();
    let mut line = launch.line();
    let directory = wide(cwd.as_os_str());
    let mut information = PROCESS_INFORMATION::default();
    // A NUL-terminated application path, a mutable NUL-terminated command
    // line, a NUL-terminated directory, the inherited environment and the
    // extended startup information.
    check(unsafe {
        CreateProcessW(
            launch.application().as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            FALSE,
            EXTENDED_STARTUPINFO_PRESENT,
            ptr::null(),
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut information,
        )
    })?;
    let (process, _thread, pid) = started(information);
    Ok((process, pid))
}

/// The new process's handle, its first thread's and its PID.
pub(crate) fn started(information: PROCESS_INFORMATION) -> (OwnedHandle, OwnedHandle, u32) {
    // Both handles are new and owned by nothing else.
    unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess),
            OwnedHandle::from_raw_handle(information.hThread),
            information.dwProcessId,
        )
    }
}

/// The command running in the pseudoconsole.
pub struct Hosted {
    console: Console,
    leader: OwnedHandle,
    pid: u32,
    output: Output,
    input: SyncSender<Vec<u8>>,
}

impl Hosted {
    fn new(opened: Opened, leader: OwnedHandle, pid: u32) -> Self {
        let (chunks, output) = mpsc::sync_channel(QUEUED);
        let source = opened.output;
        std::thread::spawn(move || pump_output(&source, &chunks));
        let (input, queued) = mpsc::sync_channel(QUEUED);
        let mut sink = opened.input;
        std::thread::spawn(move || pump_input(&mut sink, &queued));
        Self {
            console: opened.console,
            leader,
            pid,
            output: Output {
                chunks: output,
                pending: RefCell::default(),
            },
            input,
        }
    }

    /// The leader: the process started with the command.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Queues at most one chunk of `bytes` for the command's input; a full
    /// queue is `WouldBlock`, and input after the pseudoconsole ended is a
    /// broken pipe.
    pub fn write(&self, bytes: &[u8]) -> io::Result<usize> {
        let count = bytes.len().min(CHUNK);
        match self.input.try_send(bytes[..count].to_vec()) {
            Ok(()) => Ok(count),
            Err(TrySendError::Full(_)) => Err(io::ErrorKind::WouldBlock.into()),
            Err(TrySendError::Disconnected(_)) => {
                Err(io::Error::from_raw_os_error(ERROR_BROKEN_PIPE as i32))
            }
        }
    }

    /// Output already read from the pseudoconsole; `WouldBlock` when there is
    /// none yet, and a broken pipe (see [`output_ended`]) once it ended.
    pub fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        self.output.read(bytes)
    }

    /// Waits at most `timeout` for output, or its end, without taking it.
    pub fn wait_readable(&self, timeout: Duration) {
        self.output.wait(timeout);
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        self.console.resize(rows, cols)
    }

    /// The leader's exit status, in full, once it has exited.
    pub fn try_wait(&self) -> io::Result<Option<u32>> {
        // The handle is open with SYNCHRONIZE; a zero timeout only looks.
        if unsafe { WaitForSingleObject(self.leader.as_raw_handle(), 0u32) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        exit_code(&self.leader).map(Some)
    }

    /// Hang-up's first step: closing the pseudoconsole.
    pub fn close(&mut self) {
        self.console.close();
    }

    /// Hang-up's last steps, after the grace: ends whatever is left in the
    /// session job with `STATUS_CONTROL_C_EXIT`, then the leader's status as
    /// it stands.
    pub fn end(&mut self) -> io::Result<u32> {
        self.finish()?;
        // The handle is open with SYNCHRONIZE; the job has ended the leader.
        unsafe { WaitForSingleObject(self.leader.as_raw_handle(), INFINITE) };
        exit_code(&self.leader)
    }

    /// After the leader exited: closes the pseudoconsole and ends whatever is
    /// left in the session job.
    pub fn finish(&mut self) -> io::Result<()> {
        self.close();
        self.console.terminate()
    }
}

fn exit_code(process: &OwnedHandle) -> io::Result<u32> {
    let mut code = u32::default();
    // The handle is open with PROCESS_QUERY_LIMITED_INFORMATION.
    check(unsafe { GetExitCodeProcess(process.as_raw_handle(), &mut code) })?;
    Ok(code)
}

/// The output thread: forwards each read, the last being the error that
/// ended the output, until that or until nothing receives.
fn pump_output(source: &PipeReader, chunks: &SyncSender<io::Result<Vec<u8>>>) {
    let mut buffer = vec![u8::default(); CHUNK];
    let mut start_up = StartUp::default();
    while forward(
        read(source, &mut buffer).map(|count| start_up.filter(&buffer[..count])),
        chunks,
    )
    .is_some()
    {}
}

/// The sequences a pseudoconsole starts its output with: it asks its host
/// for the window position report and the device attributes, and turns on
/// focus reporting and win32-input-mode.
const NEGOTIATION: [&[u8]; 4] = [b"\x1b[1t", b"\x1b[c", b"\x1b[?1004h", b"\x1b[?9001h"];

/// The answer to the negotiation's device attributes request, the first thing
/// the input thread writes. The pseudoconsole takes the first answer it reads
/// as its host's; without this one it would take the answer to the command's
/// own first request, which the command would then never get. It is the
/// answer the session's terminal model gives a command.
const DEVICE_ATTRIBUTES: &[u8] = b"\x1b[?6c";

/// Drops the negotiation from the start of the output. Output that may
/// still be the start of a negotiation sequence is held for the next read;
/// from the first other byte on, everything passes unchanged.
#[derive(Default)]
struct StartUp {
    /// Output held while it may still be the start of a sequence.
    held: Vec<u8>,
    /// Whether the command's own output began.
    began: bool,
}

impl StartUp {
    fn filter(&mut self, read: &[u8]) -> Vec<u8> {
        if self.began {
            return read.to_vec();
        }
        let output = [self.held.as_slice(), read].concat();
        let rest = after_negotiation(&output);
        if NEGOTIATION
            .iter()
            .any(|sequence| sequence.starts_with(rest))
        {
            self.held = rest.to_vec();
            return Vec::new();
        }
        self.began = true;
        rest.to_vec()
    }
}

/// `output` after the whole negotiation sequences it starts with.
fn after_negotiation(output: &[u8]) -> &[u8] {
    NEGOTIATION
        .iter()
        .find_map(|sequence| output.strip_prefix(*sequence))
        .map_or(output, after_negotiation)
}

/// Sends `chunk`; `None` once nothing receives or the chunk was the error
/// that ended the output.
fn forward(chunk: io::Result<Vec<u8>>, chunks: &SyncSender<io::Result<Vec<u8>>>) -> Option<()> {
    let more = chunk.is_ok();
    chunks.send(chunk).ok().filter(|()| more)
}

/// Reads with `ReadFile` itself: std reports a broken pipe as end of file.
fn read(source: &PipeReader, buffer: &mut [u8]) -> io::Result<usize> {
    let mut count = u32::default();
    // The buffer holds its length in bytes; the read is synchronous.
    check(unsafe {
        ReadFile(
            source.as_raw_handle(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut count,
            ptr::null_mut(),
        )
    })?;
    Ok(count as usize)
}

/// The input thread: writes each queued chunk until the queue closes or the
/// pseudoconsole stops reading.
fn pump_input(sink: &mut PipeWriter, queued: &Receiver<Vec<u8>>) {
    for bytes in std::iter::once(DEVICE_ATTRIBUTES.to_vec()).chain(queued) {
        if sink.write_all(&bytes).is_err() {
            return;
        }
    }
}

/// The output side, used by one thread: the chunk being read from, and the
/// raw error that ended the output.
struct Output {
    chunks: Receiver<io::Result<Vec<u8>>>,
    pending: RefCell<Pending>,
}

impl Output {
    fn read(&self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut pending = self.pending.borrow_mut();
        if pending.bytes.is_empty() {
            pending.receive(self.chunks.try_recv().ok());
        }
        pending.read(bytes)
    }

    fn wait(&self, timeout: Duration) {
        let mut pending = self.pending.borrow_mut();
        if pending.bytes.is_empty() {
            pending.receive(self.chunks.recv_timeout(timeout).ok());
        }
    }
}

#[derive(Default)]
struct Pending {
    bytes: Vec<u8>,
    ended: Option<i32>,
}

impl Pending {
    fn receive(&mut self, chunk: Option<io::Result<Vec<u8>>>) {
        match chunk {
            Some(Ok(bytes)) => self.bytes = bytes,
            Some(Err(error)) => self.ended = error.raw_os_error(),
            None => {}
        }
    }

    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if self.bytes.is_empty() {
            return Err(self.ended.map_or_else(
                || io::ErrorKind::WouldBlock.into(),
                io::Error::from_raw_os_error,
            ));
        }
        let count = bytes.len().min(self.bytes.len());
        bytes[..count].copy_from_slice(&self.bytes[..count]);
        self.bytes.drain(..count);
        Ok(count)
    }
}

/// A process thread attribute list of a fixed number of attributes.
pub(crate) struct Attributes {
    buffer: Vec<u64>,
}

impl Attributes {
    pub(crate) fn new(count: u32) -> io::Result<Self> {
        let mut size = usize::default();
        // Asks for the size only; this call fails by design.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0u32, &mut size) };
        let mut attributes = Self {
            buffer: vec![u64::default(); size.div_ceil(8)],
        };
        // The buffer holds `size` bytes, aligned for the list.
        check(unsafe {
            InitializeProcThreadAttributeList(attributes.list(), count, 0u32, &mut size)
        })?;
        Ok(attributes)
    }

    pub(crate) fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_mut_ptr().cast()
    }

    /// `value` must point at `size` bytes that outlive every use of the list.
    pub(crate) fn update(
        &mut self,
        attribute: u32,
        value: *const std::ffi::c_void,
        size: usize,
    ) -> io::Result<()> {
        // As the caller guarantees.
        check(unsafe {
            UpdateProcThreadAttribute(
                self.list(),
                0u32,
                attribute as usize,
                value,
                size,
                ptr::null_mut(),
                ptr::null(),
            )
        })
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // The list was initialized in `new`.
        unsafe { DeleteProcThreadAttributeList(self.list()) };
    }
}

/// The command line for `argv`, NUL-terminated, each argument quoted as the
/// Microsoft C runtime parses it.
pub(crate) fn command_line(argv: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Vec<u16> {
    let mut line = Vec::new();
    for argument in argv {
        if !line.is_empty() {
            line.push(u16::from(b' '));
        }
        quote(argument.as_ref(), &mut line);
    }
    // The terminating NUL; spelled with its type so it is never mutated.
    line.push(0u16);
    line
}

const QUOTE: u16 = b'"' as u16;
const BACKSLASH: u16 = b'\\' as u16;

fn quote(argument: &OsStr, line: &mut Vec<u16>) {
    let units: Vec<u16> = argument.encode_wide().collect();
    if plain(&units) {
        line.extend(units);
        return;
    }
    line.push(QUOTE);
    let mut backslashes = 0;
    for unit in units {
        backslashes = escaped(unit, backslashes, line);
    }
    line.extend(std::iter::repeat_n(BACKSLASH, 2 * backslashes));
    line.push(QUOTE);
}

/// Whether `units` needs no quotes: it is not empty and holds no space, tab,
/// newline, vertical tab or quote.
fn plain(units: &[u16]) -> bool {
    !units.is_empty()
        && !units
            .iter()
            .any(|&unit| matches!(unit, 0x20 | 0x09 | 0x0A | 0x0B | 0x22))
}

/// Adds `unit` inside quotes after `backslashes` held back; the backslashes
/// now held back. Backslashes before a quote double, and the quote is escaped.
fn escaped(unit: u16, backslashes: usize, line: &mut Vec<u16>) -> usize {
    if unit == BACKSLASH {
        return backslashes + 1;
    }
    let count = if unit == QUOTE {
        2 * backslashes + 1
    } else {
        backslashes
    };
    line.extend(std::iter::repeat_n(BACKSLASH, count));
    line.push(unit);
    0
}

/// `text` as a NUL-terminated UTF-16 string.
pub(crate) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0u16)).collect()
}

fn size(rows: u16, cols: u16) -> COORD {
    COORD {
        X: cols as i16,
        Y: rows as i16,
    }
}

fn hresult(result: i32) -> io::Result<()> {
    if result < 0 {
        return Err(io::Error::other(format!("HRESULT {:#010x}", result as u32)));
    }
    Ok(())
}

pub(crate) fn check(result: BOOL) -> io::Result<()> {
    if result == FALSE {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(argv: &[&str]) -> String {
        let mut line = command_line(argv);
        assert_eq!(line.pop(), Some(0));
        String::from_utf16(&line).unwrap()
    }

    #[test]
    fn arguments_are_quoted_as_the_c_runtime_parses_them() {
        assert_eq!(line(&["cmd.exe", "/d"]), "cmd.exe /d");
        assert_eq!(
            line(&[r"C:\Program Files\x.exe", ""]),
            r#""C:\Program Files\x.exe" """#
        );
        assert_eq!(line(&["a", r#"say "hi""#]), r#"a "say \"hi\"""#);
        assert_eq!(
            line(&["a", "tab\there", "nl\nx", "vt\x0bx"]),
            "a \"tab\there\" \"nl\nx\" \"vt\x0bx\""
        );
        assert_eq!(line(&["a", r#"\"q"#, r#"\\"r"#]), r#"a "\\\"q" "\\\\\"r""#);
        assert_eq!(
            line(&["a", r"dir\ x\", r"y z\\"]),
            r#"a "dir\ x\\" "y z\\\\""#
        );
        assert_eq!(line(&[r"plain\path"]), r"plain\path");
        assert_eq!(line(&[] as &[&str]), "");
    }

    /// Each read through one filter, and what it passed of each.
    fn filtered(reads: &[&[u8]]) -> Vec<Vec<u8>> {
        let mut start_up = StartUp::default();
        reads.iter().map(|read| start_up.filter(read)).collect()
    }

    #[test]
    fn the_start_up_negotiation_is_dropped_and_the_output_after_it_kept() {
        let negotiation = b"\x1b[1t\x1b[c\x1b[?1004h\x1b[?9001h";
        assert_eq!(
            filtered(&[&[negotiation.as_slice(), b"\x1b[?7lhi"].concat()]),
            [b"\x1b[?7lhi".to_vec()]
        );
        // A sequence split across reads is held until it is whole.
        let one_by_one: Vec<&[u8]> = negotiation.chunks(1).collect();
        let passed = filtered(&[one_by_one.as_slice(), &[b"x"]].concat());
        assert!(passed[..negotiation.len()].iter().all(Vec::is_empty));
        assert_eq!(passed[negotiation.len()], b"x");
        // The start of another sequence is held, then passed whole.
        assert_eq!(
            filtered(&[b"\x1b[c\x1b[", b"?25l", b"\x1b[c"]),
            [Vec::new(), b"\x1b[?25l".to_vec(), b"\x1b[c".to_vec()]
        );
        // Once the command's output began, a negotiation sequence is its own.
        assert_eq!(
            filtered(&[b"a", b"\x1b[?1004h"]),
            [b"a".to_vec(), b"\x1b[?1004h".to_vec()]
        );
    }

    #[test]
    fn only_a_broken_pipe_ends_the_output() {
        assert!(output_ended(&io::Error::from_raw_os_error(
            ERROR_BROKEN_PIPE as i32
        )));
        assert!(!output_ended(&io::ErrorKind::WouldBlock.into()));
        assert!(!output_ended(&io::Error::from_raw_os_error(
            ERROR_BROKEN_PIPE as i32 + 1
        )));
    }

    #[test]
    fn a_failed_call_is_the_last_error_and_a_negative_hresult_fails() {
        assert!(check(1).is_ok());
        assert!(check(FALSE).is_err());
        assert!(hresult(0).is_ok());
        assert!(hresult(1).is_ok());
        assert_eq!(
            hresult(-2147024894).unwrap_err().to_string(),
            "HRESULT 0x80070002"
        );
    }
}
