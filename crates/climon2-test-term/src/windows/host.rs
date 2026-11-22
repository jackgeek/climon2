//! A program hosted in a pseudoconsole of the pinned ConPTY, under a host
//! model and a job shape.

use super::{
    conpty::{ConPty, wide},
    job::JobShape,
    keys::{self, Key, Modifiers, Win32InputMode},
};
use crate::terminal::{Feed, Options, SharedScreen, Writer, check_size, new_screen, spawn_reader};
use portable_pty::CommandBuilder;
use std::{
    ffi::OsStr,
    fs::File,
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    ptr,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{ERROR_BROKEN_PIPE, HANDLE, STILL_ACTIVE, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Console::SetConsoleCtrlHandler,
        Pipes::CreatePipe,
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
            EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
            LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
        },
    },
};

/// How a qualified host creates, feeds and closes the pseudoconsole it gives
/// its shell (docs/windows-qualification.md, "Host models").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostModel {
    /// Windows Terminal: creation flags `0x08` (grapheme width), keys in
    /// win32-input-mode once ConPTY asks for it, and close by closing the
    /// pseudoconsole. Like Windows Terminal it releases the pseudoconsole
    /// once the shell has started, so the console ends when its last
    /// process leaves.
    #[default]
    WindowsTerminal,
}

impl HostModel {
    /// The `ConptyCreatePseudoConsole` flags the real host passes.
    fn flags(self) -> u32 {
        match self {
            // PSEUDOCONSOLE_GLYPH_WIDTH_GRAPHEMES, Windows Terminal's default.
            Self::WindowsTerminal => 0x08,
        }
    }

    /// The bytes the host sends for a key, given whether ConPTY asked for
    /// win32-input-mode and whether the screen has DECCKM on.
    fn encode(self, key: Key, modifiers: Modifiers, win32: bool, cursor: bool) -> Vec<u8> {
        match self {
            Self::WindowsTerminal if win32 => keys::win32(key, modifiers),
            Self::WindowsTerminal => keys::vt(key, modifiers, cursor),
        }
    }
}

/// The host model and, separately, the job shape a terminal starts its
/// program under.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HostSettings {
    pub model: HostModel,
    pub job: JobShape,
}

/// The pseudoconsole while the host holds it.
struct Console {
    conpty: &'static ConPty,
    handle: isize,
}

impl Drop for Console {
    /// `ConptyClosePseudoConsole`: every process on the console gets
    /// `CTRL_CLOSE_EVENT`.
    fn drop(&mut self) {
        self.conpty.close(self.handle);
    }
}

pub struct Terminal {
    pub(crate) writer: Writer,
    pub(crate) screen: SharedScreen,
    reader: Option<JoinHandle<()>>,
    settings: HostSettings,
    console: Option<Console>,
    win32_input: Arc<Mutex<Win32InputMode>>,
    process: OwnedHandle,
    pid: u32,
    /// Ends every process still in it once the terminal is dropped.
    _job: Option<OwnedHandle>,
    exit: Option<u32>,
}

impl Terminal {
    /// Hosts `command` under the Windows Terminal model with no job.
    pub fn spawn(command: CommandBuilder, rows: u16, cols: u16) -> io::Result<Self> {
        Self::spawn_with(command, rows, cols, Options::default())
    }

    pub fn spawn_with(
        command: CommandBuilder,
        rows: u16,
        cols: u16,
        options: Options,
    ) -> io::Result<Self> {
        Self::spawn_hosted(command, rows, cols, options, HostSettings::default())
    }

    /// Hosts `command` in a new pseudoconsole of the pinned ConPTY, as
    /// `settings.model` does, in a job of `settings.job`'s shape.
    pub fn spawn_hosted(
        command: CommandBuilder,
        rows: u16,
        cols: u16,
        options: Options,
        settings: HostSettings,
    ) -> io::Result<Self> {
        check_size(rows, cols)?;
        let conpty = ConPty::pinned()?;
        let (console_input, input) = pipe()?;
        let (output, console_output) = pipe()?;
        let handle = conpty.create(
            rows,
            cols,
            raw(&console_input),
            raw(&console_output),
            settings.model.flags(),
        )?;
        // The pseudoconsole holds its own copies of its pipe ends.
        drop((console_input, console_output));
        let console = Console { conpty, handle };
        let job = settings.job.create()?;
        let (process, pid) = start(&command, handle, job.as_ref())?;
        conpty.release(handle)?;
        let writer: Writer = Arc::new(Mutex::new(Box::new(File::from(input))));
        let (screen, events) = new_screen(rows, cols, options);
        let win32_input = Arc::new(Mutex::new(Win32InputMode::default()));
        let tap_mode = win32_input.clone();
        let reader = spawn_reader(
            Feed {
                input: Box::new(File::from(output)),
                screen: screen.clone(),
                writer: writer.clone(),
                events,
                recent: options.recent,
                hang_up: |error| error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32),
            },
            move |bytes| tap_mode.lock().expect("input mode lock").feed(bytes),
        );
        Ok(Self {
            writer,
            screen,
            reader: Some(reader),
            settings,
            console: Some(console),
            win32_input,
            process,
            pid,
            _job: job,
            exit: None,
        })
    }

    /// The process the terminal was spawned with.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether ConPTY last asked this host for win32-input-mode.
    pub fn win32_input_mode(&self) -> bool {
        self.win32_input.lock().expect("input mode lock").on()
    }

    /// Presses and releases `key` with `modifiers` held, sent in the host
    /// model's encoding for the modes ConPTY asked for.
    pub fn press(&mut self, key: Key, modifiers: Modifiers) -> io::Result<()> {
        let win32 = self.win32_input_mode();
        let model = self.settings.model;
        self.inject(|host| {
            let cursor = host
                .mode
                .contains(alacritty_terminal::term::TermMode::APP_CURSOR);
            Ok(model.encode(key, modifiers, win32, cursor))
        })
    }

    /// Types `text` key by key, as [`Terminal::press`] sends each character.
    pub fn type_text(&mut self, text: &str) -> io::Result<()> {
        text.chars()
            .try_for_each(|c| self.press(Key::Char(c), Modifiers::NONE))
    }

    pub fn resize(&mut self, rows: u16, cols: u16) -> io::Result<()> {
        check_size(rows, cols)?;
        let mut screen = self.screen.0.lock().expect("screen lock");
        let console = self
            .console
            .as_ref()
            .ok_or_else(|| screen.failure(io::Error::other("the pseudoconsole is closed")))?;
        console
            .conpty
            .resize(console.handle, rows, cols)
            .map_err(|error| screen.failure(error))?;
        screen.resize(rows, cols);
        Ok(())
    }

    /// Closes the terminal as the host model closes its tab: for Windows
    /// Terminal, by closing the pseudoconsole, which sends every process on it
    /// `CTRL_CLOSE_EVENT`. Closing twice does nothing more.
    pub fn close(&mut self) {
        if let Some(console) = self.console.take() {
            match self.settings.model {
                HostModel::WindowsTerminal => drop(console),
            }
        }
    }

    fn poll_exit(&mut self) -> io::Result<()> {
        if self.exit.is_none() {
            let mut code = 0;
            // The process handle is open and the call writes one u32.
            if unsafe { GetExitCodeProcess(raw(&self.process), &mut code) } == 0 {
                return Err(io::Error::last_os_error());
            }
            // STILL_ACTIVE is also a status a process can exit with, so ask
            // the handle itself whether the process ended.
            if code != STILL_ACTIVE as u32 || !alive(&self.process) {
                self.exit = Some(code);
            }
        }
        Ok(())
    }

    /// Waits until the program exited and the pseudoconsole's output ended,
    /// and returns the program's full 32-bit exit status.
    pub fn wait_exit(&mut self, timeout: Duration) -> io::Result<u32> {
        let deadline = Instant::now() + timeout;
        loop {
            self.poll_exit()
                .map_err(|error| self.screen.0.lock().expect("screen lock").failure(error))?;
            let (screen, changed) = &*self.screen;
            let state = screen.lock().expect("screen lock");
            state.check()?;
            if let Some(code) = self.exit.filter(|_| state.eof) {
                return Ok(code);
            }
            if Instant::now() >= deadline {
                return Err(state.failure(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "child did not exit and end its pseudoconsole output",
                )));
            }
            let _guard = changed
                .wait_timeout(
                    state,
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_millis(5)),
                )
                .expect("screen wait");
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.close();
        let deadline = Instant::now() + Duration::from_secs(5);
        while alive(&self.process) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        // The process handle is open; ending an exited process fails harmlessly.
        if alive(&self.process) && unsafe { TerminateProcess(raw(&self.process), 1) } == 0 {
            eprintln!(
                "pseudoconsole child {} cleanup failed: {}",
                self.pid,
                io::Error::last_os_error()
            );
        }
        if let Some(reader) = self.reader.take() {
            let deadline = Instant::now() + Duration::from_secs(1);
            while !reader.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(5));
            }
            if reader.is_finished() {
                reader.join().expect("pseudoconsole reader panicked");
            } else {
                eprintln!("pseudoconsole reader did not end after the console closed");
            }
        }
    }
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    handle.as_raw_handle() as HANDLE
}

fn alive(process: &OwnedHandle) -> bool {
    // The process handle is open; a zero timeout only polls.
    match unsafe { WaitForSingleObject(raw(process), 0) } {
        WAIT_TIMEOUT => true,
        WAIT_OBJECT_0 => false,
        _ => false,
    }
}

/// A new anonymous pipe, as (read end, write end); neither is inheritable.
fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (ptr::null_mut(), ptr::null_mut());
    // Both out-pointers are valid; default security, default buffer size.
    if unsafe { CreatePipe(&mut read, &mut write, ptr::null(), 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // Both handles are new and owned by nothing else.
    unsafe {
        Ok((
            OwnedHandle::from_raw_handle(read as RawHandle),
            OwnedHandle::from_raw_handle(write as RawHandle),
        ))
    }
}

/// A process thread attribute list with the pseudoconsole and, if any, the job.
struct Attributes {
    buffer: Vec<u64>,
}

impl Attributes {
    fn new(console: isize, job: Option<&HANDLE>) -> io::Result<Self> {
        let count = 1 + u32::from(job.is_some());
        let mut size = 0;
        // Asks for the size only; this call fails by design.
        unsafe { InitializeProcThreadAttributeList(ptr::null_mut(), count, 0, &mut size) };
        let mut attributes = Self {
            buffer: vec![0; size.div_ceil(8)],
        };
        // The buffer holds `size` bytes, aligned for the list.
        if unsafe { InitializeProcThreadAttributeList(attributes.list(), count, 0, &mut size) } == 0
        {
            return Err(io::Error::last_os_error());
        }
        attributes.update(
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            // This attribute's value is the HPCON itself, not a pointer to it.
            console as *const std::ffi::c_void,
            size_of::<isize>(),
        )?;
        if let Some(job) = job {
            attributes.update(
                PROC_THREAD_ATTRIBUTE_JOB_LIST,
                ptr::from_ref(job).cast(),
                size_of::<HANDLE>(),
            )?;
        }
        Ok(attributes)
    }

    fn list(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buffer.as_mut_ptr().cast()
    }

    fn update(
        &mut self,
        attribute: u32,
        value: *const std::ffi::c_void,
        size: usize,
    ) -> io::Result<()> {
        // `value` points at `size` bytes that outlive the list's use.
        let updated = unsafe {
            UpdateProcThreadAttribute(
                self.list(),
                0,
                attribute as usize,
                value,
                size,
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if updated == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for Attributes {
    fn drop(&mut self) {
        // The list was initialized in `new`.
        unsafe { DeleteProcThreadAttributeList(self.list()) };
    }
}

/// Starts `command` on the pseudoconsole `console`, in `job` from its first
/// instruction, as Windows Terminal does: no inherited handles, standard
/// handles left null so the program takes the pseudoconsole's, and a Unicode
/// environment block.
fn start(
    command: &CommandBuilder,
    console: isize,
    job: Option<&OwnedHandle>,
) -> io::Result<(OwnedHandle, u32)> {
    let job = job.map(raw);
    let mut attributes = Attributes::new(console, job.as_ref())?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes.list();
    let mut line = command_line(command.get_argv())?;
    let environment = environment(command);
    let directory = command.get_cwd().map(|cwd| wide(cwd));
    // A host's shell handles Ctrl+C: clear any "ignore Ctrl+C" this test
    // process inherited (a new process group sets it), which the shell would
    // inherit in turn.
    unsafe { SetConsoleCtrlHandler(None, 0) };
    let mut information = PROCESS_INFORMATION::default();
    // Every pointer is valid for the call: a mutable NUL-terminated command
    // line, a double-NUL-terminated UTF-16 environment, an optional
    // NUL-terminated directory and the extended startup information.
    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr().cast(),
            directory
                .as_ref()
                .map_or(ptr::null(), |directory| directory.as_ptr()),
            &startup.StartupInfo,
            &mut information,
        )
    };
    if created == 0 {
        return Err(io::Error::last_os_error());
    }
    // Both handles are new and owned by nothing else.
    let (process, _thread) = unsafe {
        (
            OwnedHandle::from_raw_handle(information.hProcess as RawHandle),
            OwnedHandle::from_raw_handle(information.hThread as RawHandle),
        )
    };
    Ok((process, information.dwProcessId))
}

/// The command line for `argv`, each argument quoted as the Microsoft C
/// runtime parses it.
fn command_line(argv: &[std::ffi::OsString]) -> io::Result<Vec<u16>> {
    if argv.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no program to run",
        ));
    }
    let mut line = Vec::new();
    for (index, argument) in argv.iter().enumerate() {
        if index > 0 {
            line.push(u16::from(b' '));
        }
        quote(argument, &mut line);
    }
    line.push(0);
    Ok(line)
}

fn quote(argument: &OsStr, line: &mut Vec<u16>) {
    use std::os::windows::ffi::OsStrExt;
    let units: Vec<u16> = argument.encode_wide().collect();
    let plain = !units.is_empty()
        && !units
            .iter()
            .any(|&unit| matches!(unit, 0x20 | 0x09 | 0x0A | 0x0B | 0x22));
    if plain {
        line.extend(units);
        return;
    }
    line.push(u16::from(b'"'));
    let mut backslashes = 0;
    for unit in units {
        if unit == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        let doubled = if unit == u16::from(b'"') {
            2 * backslashes + 1
        } else {
            backslashes
        };
        line.extend(std::iter::repeat_n(u16::from(b'\\'), doubled));
        backslashes = 0;
        line.push(unit);
    }
    line.extend(std::iter::repeat_n(u16::from(b'\\'), 2 * backslashes));
    line.push(u16::from(b'"'));
}

/// The command's whole environment as a sorted UTF-16 block.
fn environment(command: &CommandBuilder) -> Vec<u16> {
    let mut block = Vec::new();
    for (key, value) in command.iter_full_env_as_str() {
        block.extend(format!("{key}={value}").encode_utf16());
        block.push(0);
    }
    if block.is_empty() {
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(argv: &[&str]) -> String {
        let argv: Vec<_> = argv.iter().map(std::ffi::OsString::from).collect();
        let mut line = command_line(&argv).unwrap();
        line.pop();
        String::from_utf16(&line).unwrap()
    }

    #[test]
    fn arguments_are_quoted_as_the_c_runtime_parses_them() {
        assert_eq!(line(&["pwsh.exe", "-NoLogo"]), "pwsh.exe -NoLogo");
        assert_eq!(
            line(&[r"C:\Program Files\pwsh.exe", ""]),
            r#""C:\Program Files\pwsh.exe" """#
        );
        assert_eq!(line(&["a", r#"say "hi""#]), r#"a "say \"hi\"""#);
        assert_eq!(line(&["a", r"dir\ x\"]), r#"a "dir\ x\\""#);
        assert_eq!(line(&["a", r#"\"q"#]), r#"a "\\\"q""#);
        assert!(command_line(&[]).is_err());
    }
}
