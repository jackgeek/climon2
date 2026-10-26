use crate::{absolute, launch_error, nonempty, real_exe, session::Config, Result};
use socket2::Socket;
use std::{env, ffi::c_void, fs::{self, File}, io::{self, Read, Write}, mem::{size_of, zeroed}, os::windows::{ffi::OsStrExt, io::{AsRawSocket, FromRawHandle}, process::CommandExt}, path::{Path, PathBuf}, process::{Command, ExitStatus}, ptr::{null, null_mut}};
use windows_sys::Win32::{Foundation::*, Security::{*, Authorization::*}, Storage::FileSystem::*, System::{Console::*, JobObjects::*, Threading::*, IO::*, Memory::*}, Networking::WinSock::*};

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> { s.as_ref().encode_wide().chain(Some(0)).collect() }
fn last() -> String { io::Error::last_os_error().to_string() }
struct Handle(HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Drop for Handle { fn drop(&mut self) { unsafe { CloseHandle(self.0); } } }
fn handle(h: HANDLE) -> Result<Handle> { if h.is_null() || h == INVALID_HANDLE_VALUE { Err(last()) } else { Ok(Handle(h)) } }
unsafe extern "system" fn ignore_interrupt(kind: u32) -> BOOL { if kind == CTRL_C_EVENT { 1 } else { 0 } }
fn ignore_ctrl_c() -> Result<()> {
    unsafe {
        // The NULL-handler ignore flag is inherited; a registered handler is not.
        if SetConsoleCtrlHandler(None, 0) == 0 || SetConsoleCtrlHandler(Some(ignore_interrupt), 1) == 0 { return Err(last()); }
    }
    Ok(())
}
fn job(breakaway: bool) -> Result<Handle> {
    unsafe {
        let job = handle(CreateJobObjectW(null(), null()))?;
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | if breakaway { JOB_OBJECT_LIMIT_BREAKAWAY_OK } else { 0 };
        if SetInformationJobObject(job.0, JobObjectExtendedLimitInformation, (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32) == 0 { return Err(last()); }
        Ok(job)
    }
}
fn quote(value: &str) -> String {
    let mut output = String::from("\"");
    let mut slashes = 0;
    for c in value.chars() {
        if c == '\\' { slashes += 1; continue; }
        if c == '"' { output.extend(std::iter::repeat('\\').take(slashes * 2 + 1)); }
        else { output.extend(std::iter::repeat('\\').take(slashes)); }
        output.push(c);
        slashes = 0;
    }
    output.extend(std::iter::repeat('\\').take(slashes * 2));
    output.push('"');
    output
}
unsafe fn create_process(path: &Path, args: &[String], startup: *const STARTUPINFOW, flags: u32, inherit: bool, job: &Handle) -> Result<Handle> {
    let application = wide(path);
    let text = std::iter::once(path.to_string_lossy().into_owned()).chain(args.iter().cloned()).map(|s| quote(&s)).collect::<Vec<_>>().join(" ");
    let mut line = wide(text);
    let mut info: PROCESS_INFORMATION = zeroed();
    if CreateProcessW(application.as_ptr(), line.as_mut_ptr(), null(), null(), inherit as i32, flags | CREATE_SUSPENDED, null(), null(), startup, &mut info) == 0 { return Err(launch_error(path, last())); }
    let process = handle(info.hProcess)?;
    let thread = handle(info.hThread)?;
    if AssignProcessToJobObject(job.0, process.0) == 0 {
        TerminateProcess(process.0, 125);
        WaitForSingleObject(process.0, INFINITE);
        return Err(launch_error(path, last()));
    }
    if ResumeThread(thread.0) == u32::MAX {
        TerminateProcess(process.0, 125);
        WaitForSingleObject(process.0, INFINITE);
        return Err(launch_error(path, last()));
    }
    Ok(process)
}
pub fn launch_cli(_command: Command, path: &Path) -> Result<i32> {
    ignore_ctrl_c()?;
    let job = job(true)?;
    let args = env::args().skip(1).collect::<Vec<_>>();
    unsafe {
        let mut startup: STARTUPINFOW = zeroed();
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        startup.dwFlags = STARTF_USESTDHANDLES;
        startup.hStdInput = GetStdHandle(STD_INPUT_HANDLE);
        startup.hStdOutput = GetStdHandle(STD_OUTPUT_HANDLE);
        startup.hStdError = GetStdHandle(STD_ERROR_HANDLE);
        let process = create_process(path, &args, &startup, 0, true, &job)?;
        if WaitForSingleObject(process.0, INFINITE) != WAIT_OBJECT_0 { return Err(last()); }
        let mut code = 0;
        if GetExitCodeProcess(process.0, &mut code) == 0 { return Err(last()); }
        Ok(code as i32)
    }
}
pub fn prepare_daemon(command: &mut Command) -> Result<()> {
    let mut flags = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    unsafe {
        let mut in_job = 0;
        if IsProcessInJob(GetCurrentProcess(), null_mut(), &mut in_job) == 0 { return Err(last()); }
        if in_job != 0 {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = zeroed();
            if QueryInformationJobObject(null_mut(), JobObjectExtendedLimitInformation, (&mut info as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(), size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32, null_mut()) == 0 { return Err(last()); }
            if info.BasicLimitInformation.LimitFlags & JOB_OBJECT_LIMIT_BREAKAWAY_OK != 0 { flags |= CREATE_BREAKAWAY_FROM_JOB; }
        }
    }
    command.creation_flags(flags);
    Ok(())
}
pub fn daemon_init() -> Result<()> { Ok(()) }
pub fn status(status: ExitStatus) -> i32 { status.code().unwrap_or(125) }
pub fn check_executable(path: &Path) -> Result<()> {
    if !path.is_file() { return Err(launch_error(path, "executable is missing")); }
    // Validate the PE signature before starting a session with an unusable companion.
    let mut file = File::open(path).map_err(|e| launch_error(path, e))?;
    let mut signature = [0; 2];
    if file.read_exact(&mut signature).is_err() || signature != *b"MZ" { return Err(launch_error(path, "not an executable")); }
    Ok(())
}
pub fn resolve(command: &str, args: &[String]) -> Result<PathBuf> {
    let supplied = Path::new(command);
    let target = if supplied.components().count() > 1 || command.contains('/') || command.contains('\\') || supplied.is_absolute() {
        absolute(supplied)?
    } else {
        let extensions = env::var("PATHEXT").unwrap_or_default();
        let mut found = None;
        if let Some(path) = env::var_os("PATH") {
            'directories: for directory in env::split_paths(&path) {
                if supplied.extension().is_some() {
                    let candidate = directory.join(command);
                    if candidate.is_file() { found = Some(absolute(candidate)?); break; }
                } else {
                    for extension in extensions.split(';').filter(|e| !e.is_empty()) {
                        let candidate = directory.join(format!("{command}{extension}"));
                        if candidate.is_file() { found = Some(absolute(candidate)?); break 'directories; }
                    }
                }
            }
        }
        found.ok_or_else(|| launch_error(command, "not found in PATH"))?
    };
    let extension = target.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    if extension == "cmd" || extension == "bat" {
        if target.to_string_lossy().contains(['\r', '\n', '"', '%']) || args.iter().any(|arg| arg.contains(['\r', '\n', '"', '%'])) { return Err(launch_error(&target, "unsafe batch argument")); }
        if nonempty("ComSpec").is_none() { return Err(launch_error(&target, "ComSpec is unset or empty")); }
    }
    if extension == "cmd" || extension == "bat" {
        if !target.is_file() { return Err(launch_error(&target, "batch file is missing")); }
    } else { check_executable(&target)?; }
    Ok(target)
}

fn token_sid(process: HANDLE) -> Result<Vec<u8>> {
    unsafe {
        let mut raw = null_mut();
        if OpenProcessToken(process, TOKEN_QUERY, &mut raw) == 0 { return Err(last()); }
        let token = handle(raw)?;
        let mut bytes = 0;
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut bytes);
        let mut buffer = vec![0usize; (bytes as usize + size_of::<usize>() - 1) / size_of::<usize>()];
        if GetTokenInformation(token.0, TokenUser, buffer.as_mut_ptr().cast(), bytes, &mut bytes) == 0 { return Err(last()); }
        let sid = (*(buffer.as_ptr() as *const TOKEN_USER)).User.Sid;
        let length = GetLengthSid(sid);
        let mut result = vec![0; length as usize];
        if CopySid(length, result.as_mut_ptr().cast(), sid) == 0 { return Err(last()); }
        Ok(result)
    }
}
fn own_sid() -> Result<Vec<u8>> { token_sid(unsafe { GetCurrentProcess() }) }
struct Descriptor(*mut c_void);
impl Drop for Descriptor { fn drop(&mut self) { unsafe { LocalFree(self.0); } } }
fn descriptor(inherit: bool) -> Result<Descriptor> {
    let sid = own_sid()?;
    unsafe {
        let mut text = null_mut();
        if ConvertSidToStringSidW(sid.as_ptr() as *mut _, &mut text) == 0 { return Err(last()); }
        let mut n = 0;
        while *text.add(n) != 0 { n += 1; }
        let sid_text = String::from_utf16_lossy(std::slice::from_raw_parts(text, n));
        LocalFree(text.cast());
        let sddl = wide(format!("D:P(A;{};FA;;;{})", if inherit { "OICI" } else { "" }, sid_text));
        let mut descriptor = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, null_mut()) == 0 { return Err(last()); }
        Ok(Descriptor(descriptor))
    }
}
fn attrs(descriptor: &Descriptor) -> SECURITY_ATTRIBUTES { SECURITY_ATTRIBUTES { nLength: size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: descriptor.0, bInheritHandle: 0 } }
fn open_security(path: &Path, access: u32) -> Result<Handle> {
    unsafe { handle(CreateFileW(wide(path).as_ptr(), access, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, null(), OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, null_mut())) }
}
fn is_private(path: &Path) -> Result<bool> {
    let file = open_security(path, READ_CONTROL)?;
    let sid = own_sid()?;
    unsafe {
        let (mut owner, mut dacl, mut sd) = (null_mut(), null_mut(), null_mut());
        let code = GetSecurityInfo(file.0, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, &mut owner, null_mut(), &mut dacl, null_mut(), &mut sd);
        if code != 0 { return Err(io::Error::from_raw_os_error(code as i32).to_string()); }
        let _descriptor = Descriptor(sd);
        if owner.is_null() || EqualSid(owner, sid.as_ptr() as *mut _) == 0 || dacl.is_null() || (*dacl).AceCount == 0 { return Ok(false); }
        for index in 0..(*dacl).AceCount as u32 {
            let mut ace = null_mut();
            if GetAce(dacl, index, &mut ace) == 0 { return Err(last()); }
            let header = &*(ace as *const ACE_HEADER);
            if header.AceType != 0 || EqualSid((ace as *mut u8).add(8).cast(), sid.as_ptr() as *mut _) == 0 { return Ok(false); }
        }
        Ok(true)
    }
}
pub fn runtime_dir() -> Result<PathBuf> {
    let base = nonempty("LOCALAPPDATA").ok_or("runtime directory: LOCALAPPDATA is unset or empty")?;
    let path = absolute(PathBuf::from(base).join("climon2").join("run"))?;
    if !path.exists() {
        fs::create_dir_all(path.parent().unwrap()).map_err(|e| format!("runtime directory {}: {e}", path.display()))?;
        let descriptor = descriptor(true)?;
        let sa = attrs(&descriptor);
        if unsafe { CreateDirectoryW(wide(&path).as_ptr(), &sa) } == 0 && !path.exists() { return Err(format!("runtime directory {}: {}", path.display(), last())); }
    }
    if !path.is_dir() || !is_private(&path).map_err(|e| format!("unsafe runtime directory {}: {e}", path.display()))? { return Err(format!("unsafe runtime directory: {}", path.display())); }
    Ok(path)
}
pub fn check_directory_access(path: &Path) -> io::Result<()> {
    unsafe {
        let raw = CreateFileW(wide(path).as_ptr(), FILE_LIST_DIRECTORY | READ_CONTROL, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, null(), OPEN_EXISTING, FILE_FLAG_BACKUP_SEMANTICS, null_mut());
        if raw == INVALID_HANDLE_VALUE { return Err(io::Error::last_os_error()); }
        let directory = Handle(raw);
        let mut sd = null_mut();
        let code = GetSecurityInfo(directory.0, SE_FILE_OBJECT, OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, null_mut(), null_mut(), null_mut(), null_mut(), &mut sd);
        if code != 0 { return Err(io::Error::from_raw_os_error(code as i32)); }
        let descriptor = Descriptor(sd);
        let mut raw = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut raw) == 0 { return Err(io::Error::last_os_error()); }
        let process_token = Handle(raw);
        let mut raw = null_mut();
        if DuplicateToken(process_token.0, SecurityImpersonation, &mut raw) == 0 { return Err(io::Error::last_os_error()); }
        let token = Handle(raw);
        let mut mapping = GENERIC_MAPPING { GenericRead: FILE_GENERIC_READ, GenericWrite: FILE_GENERIC_WRITE, GenericExecute: FILE_GENERIC_EXECUTE, GenericAll: FILE_ALL_ACCESS };
        let mut privileges = [0usize; 128];
        let mut bytes = size_of::<[usize; 128]>() as u32;
        let (mut granted, mut allowed) = (0, 0);
        // Directory opens/enumeration can bypass list denial under backup semantics.
        // Check the DACL explicitly, including for an empty directory.
        if AccessCheck(descriptor.0, token.0, FILE_LIST_DIRECTORY, &mut mapping, privileges.as_mut_ptr().cast(), &mut bytes, &mut granted, &mut allowed) == 0 { return Err(io::Error::last_os_error()); }
        if allowed == 0 { return Err(io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32)); }
        Ok(())
    }
}
struct ThreadToken { previous: Option<Handle>, active: bool }
impl ThreadToken {
    fn restore(&mut self) -> io::Result<()> {
        if !self.active { return Ok(()); }
        let token = self.previous.as_ref().map_or(null_mut(), |h| h.0);
        if unsafe { SetThreadToken(null(), token) } == 0 { return Err(io::Error::last_os_error()); }
        self.active = false;
        Ok(())
    }
}
impl Drop for ThreadToken {
    fn drop(&mut self) {
        if let Err(error) = self.restore() { eprintln!("climon2: cannot restore thread identity: {error}"); }
    }
}
pub fn bind_private(socket: &Socket, address: &socket2::SockAddr) -> io::Result<()> {
    // Set the creating thread's default DACL: AF_UNIX reparse points cannot
    // reliably be reopened with WRITE_DAC after bind.
    unsafe {
        let descriptor = descriptor(false).map_err(io::Error::other)?;
        let (mut present, mut defaulted, mut dacl) = (0, 0, null_mut());
        if GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) == 0 { return Err(io::Error::last_os_error()); }
        let mut raw = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_DUPLICATE | TOKEN_QUERY, &mut raw) == 0 { return Err(io::Error::last_os_error()); }
        let process_token = Handle(raw);
        let mut raw = null_mut();
        if DuplicateTokenEx(process_token.0, TOKEN_IMPERSONATE | TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, null(), SecurityImpersonation, TokenImpersonation, &mut raw) == 0 { return Err(io::Error::last_os_error()); }
        let token = Handle(raw);
        let default_dacl = TOKEN_DEFAULT_DACL { DefaultDacl: dacl };
        if SetTokenInformation(token.0, TokenDefaultDacl, (&default_dacl as *const TOKEN_DEFAULT_DACL).cast(), size_of::<TOKEN_DEFAULT_DACL>() as u32) == 0 { return Err(io::Error::last_os_error()); }
        let mut raw = null_mut();
        let previous = if OpenThreadToken(GetCurrentThread(), TOKEN_IMPERSONATE, 1, &mut raw) != 0 { Some(Handle(raw)) }
            else if GetLastError() == ERROR_NO_TOKEN { None }
            else { return Err(io::Error::last_os_error()); };
        if SetThreadToken(null(), token.0) == 0 { return Err(io::Error::last_os_error()); }
        let mut guard = ThreadToken { previous, active: true };
        let result = socket.bind(address);
        guard.restore()?;
        result
    }
}
pub fn private_socket(path: &Path) -> Result<()> {
    if !is_private(path).map_err(|e| format!("socket path {}: {e}", path.display()))? {
        return Err(format!("socket path {}: unsafe socket permissions", path.display()));
    }
    Ok(())
}
pub fn private_file(path: &Path) -> io::Result<File> {
    let descriptor = descriptor(false).map_err(io::Error::other)?;
    let sa = attrs(&descriptor);
    unsafe {
        let raw = CreateFileW(wide(path).as_ptr(), GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_DELETE, &sa, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, null_mut());
        if raw == INVALID_HANDLE_VALUE { return Err(io::Error::last_os_error()); }
        Ok(File::from_raw_handle(raw))
    }
}
fn peer_pid(socket: &Socket) -> Result<u32> {
    unsafe {
        let (mut pid, mut used) = (0u32, 0u32);
        if WSAIoctl(socket.as_raw_socket() as usize, 0x58000100, null(), 0, (&mut pid as *mut u32).cast(), 4, &mut used, null_mut(), None) != 0 {
            return Err(format!("peer identity verification unavailable: Winsock {}", WSAGetLastError()));
        }
        // Windows AF_UNIX can write the PID without updating the byte count.
        // A zero-initialized output must still contain a real, queryable PID;
        // peer() checks its token SID and verify_transport() checks both ends.
        if (used != 0 && used != 4) || pid == 0 {
            return Err(format!("peer identity verification unavailable: invalid peer PID response ({used} bytes, PID {pid})"));
        }
        Ok(pid)
    }
}
pub fn peer(socket: &Socket) -> bool {
    let checked = (|| -> Result<bool> {
        let pid = peer_pid(socket)?;
        let process = unsafe { handle(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid)) }?;
        Ok(token_sid(process.0)? == own_sid()?)
    })();
    checked.unwrap_or(false)
}
pub fn verify_transport() -> Result<()> {
    let path = runtime_dir()?.join(format!("{}.sock", uuid::Uuid::new_v4().simple()));
    let result = (|| -> Result<()> {
        let listener = crate::protocol::listener(&path).map_err(|e| format!("socket path {}: {e}", path.display()))?;
        let sender = crate::protocol::Stream::connect(&path, std::time::Instant::now() + std::time::Duration::from_secs(2)).map_err(|e| e.to_string())?;
        let (receiver, _) = listener.accept().map_err(|e| e.to_string())?;
        if peer_pid(&receiver)? != std::process::id() || peer_pid(&sender.0)? != std::process::id() || !peer(&receiver) { return Err("peer identity verification unavailable".into()); }
        private_socket(&path)?;
        Ok(())
    })();
    match fs::remove_file(&path) { Ok(()) => (), Err(e) if e.kind() == io::ErrorKind::NotFound => (), Err(e) => return Err(format!("socket path {}: {e}", path.display())) }
    result
}

pub struct TerminalGuard { input: HANDLE, output: HANDLE, input_mode: u32, output_mode: u32 }
impl TerminalGuard {
    pub fn new() -> Result<Self> {
        unsafe {
            let (input, output) = (GetStdHandle(STD_INPUT_HANDLE), GetStdHandle(STD_OUTPUT_HANDLE));
            let (mut input_mode, mut output_mode) = (0, 0);
            if GetConsoleMode(input, &mut input_mode) == 0 || GetConsoleMode(output, &mut output_mode) == 0 { return Err(last()); }
            Ok(Self { input, output, input_mode, output_mode })
        }
    }
    pub fn raw(&self) -> Result<()> {
        unsafe {
            if SetConsoleMode(self.input, (self.input_mode & !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT)) | ENABLE_VIRTUAL_TERMINAL_INPUT) == 0
                || SetConsoleMode(self.output, self.output_mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN) == 0 { return Err(last()); }
        }
        Ok(())
    }
}
impl Drop for TerminalGuard { fn drop(&mut self) { unsafe { SetConsoleMode(self.input, self.input_mode); SetConsoleMode(self.output, self.output_mode); } } }
pub fn size() -> (u16, u16) {
    unsafe {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = zeroed();
        if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) == 0 { return (80, 24); }
        ((info.srWindow.Right - info.srWindow.Left + 1) as u16, (info.srWindow.Bottom - info.srWindow.Top + 1) as u16)
    }
}
pub fn process_start() -> Result<String> {
    unsafe {
        let (mut created, mut exited, mut kernel, mut user): (FILETIME, FILETIME, FILETIME, FILETIME) = zeroed();
        if GetProcessTimes(GetCurrentProcess(), &mut created, &mut exited, &mut kernel, &mut user) == 0 { return Err(last()); }
        let ticks = ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64;
        let instant = chrono::DateTime::from_timestamp((ticks / 10_000_000) as i64 - 11_644_473_600, ((ticks % 10_000_000) * 100) as u32).ok_or("invalid process start time")?;
        Ok(instant.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
    }
}
pub fn local_stamp(timestamp: i64) -> Result<String> {
    chrono::DateTime::from_timestamp(timestamp, 0).map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string()).ok_or_else(|| "cannot convert local timestamp".into())
}

pub fn is_pty_eof(error: &io::Error) -> bool { error.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) }
pub fn relay_output(output: &mut dyn Write, bytes: &[u8], pending: &mut Vec<u8>) -> io::Result<()> {
    pending.extend_from_slice(bytes);
    let complete = match std::str::from_utf8(pending) {
        Ok(_) => pending.len(),
        Err(error) if error.error_len().is_none() => error.valid_up_to(),
        Err(error) => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    };
    output.write_all(&pending[..complete])?;
    pending.drain(..complete);
    output.flush()
}
struct Console(HPCON);
impl Drop for Console { fn drop(&mut self) { unsafe { ClosePseudoConsole(self.0); } } }
unsafe impl Send for Console {}
unsafe impl Sync for Console {}
pub struct Pty {
    pub reader: Option<Box<dyn Read + Send>>,
    pub writer: Box<dyn Write + Send>,
    process: Handle,
    job: Handle,
    console: std::sync::Mutex<Option<Console>>,
}
fn pipe() -> Result<(File, File)> {
    #[link(name = "kernel32")]
    extern "system" { fn CreatePipe(read: *mut HANDLE, write: *mut HANDLE, sa: *const SECURITY_ATTRIBUTES, size: u32) -> BOOL; }
    unsafe {
        let (mut read, mut write) = (null_mut(), null_mut());
        if CreatePipe(&mut read, &mut write, null(), 0) == 0 { return Err(last()); }
        Ok((File::from_raw_handle(read), File::from_raw_handle(write)))
    }
}
struct BufferedOutput {
    receiver: std::sync::mpsc::Receiver<io::Result<Vec<u8>>>,
    current: io::Cursor<Vec<u8>>,
}
impl Read for BufferedOutput {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() { return Ok(0); }
        loop {
            let count = self.current.read(buffer)?;
            if count != 0 { return Ok(count); }
            match self.receiver.recv() {
                Ok(Ok(bytes)) => self.current = io::Cursor::new(bytes),
                Ok(Err(error)) => return Err(error),
                Err(_) => return Ok(0),
            }
        }
    }
}
fn drain_output(mut pipe: File) -> Box<dyn Read + Send> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(64);
    std::thread::spawn(move || {
        let mut buffer = [0; 16384];
        loop {
            match pipe.read(&mut buffer) {
                Ok(0) => return,
                Ok(count) => { if sender.send(Ok(buffer[..count].to_vec())).is_err() { return; } }
                Err(error) => { let _ = sender.send(Err(error)); return; }
            }
        }
    });
    Box::new(BufferedOutput { receiver, current: io::Cursor::new(Vec::new()) })
}
impl Pty {
    pub fn spawn(config: &Config) -> Result<Self> {
        use std::os::windows::io::AsRawHandle;
        let (input_read, input_write) = pipe()?;
        let (output_read, output_write) = pipe()?;
        // ConPTY can write while creating or attaching a process, before startup replies.
        let output_read = drain_output(output_read);
        unsafe {
            let mut raw = 0;
            let hr = CreatePseudoConsole(COORD { X: config.cols as i16, Y: config.rows as i16 }, input_read.as_raw_handle(), output_write.as_raw_handle(), 0, &mut raw);
            if hr < 0 { return Err(format!("cannot create ConPTY: {hr:#x}")); }
            let console = Console(raw);
            let mut bytes = 0;
            InitializeProcThreadAttributeList(null_mut(), 2, 0, &mut bytes);
            let mut storage = vec![0usize; (bytes + size_of::<usize>() - 1) / size_of::<usize>()];
            let list = storage.as_mut_ptr().cast();
            if InitializeProcThreadAttributeList(list, 2, 0, &mut bytes) == 0 { return Err(last()); }
            struct Attributes(LPPROC_THREAD_ATTRIBUTE_LIST);
            impl Drop for Attributes { fn drop(&mut self) { unsafe { DeleteProcThreadAttributeList(self.0); } } }
            let list = Attributes(list);
            if UpdateProcThreadAttribute(list.0, 0, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize, raw as *const c_void, size_of::<HPCON>(), null_mut(), null()) == 0 { return Err(last()); }
            let mut startup: STARTUPINFOEXW = zeroed();
            startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
            startup.lpAttributeList = list.0;
            let job = job(false)?;
            let (startup_read, startup_write) = pipe()?;
            if SetHandleInformation(startup_write.as_raw_handle(), HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) == 0 { return Err(last()); }
            // Do not leak the daemon's CLI control pipes into the console helper.
            let inherited = [startup_write.as_raw_handle()];
            if UpdateProcThreadAttribute(list.0, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize, inherited.as_ptr().cast(), size_of::<HANDLE>(), null_mut(), null()) == 0 { return Err(last()); }
            let args = vec!["--run-command".to_owned(), serde_json::to_string(config).map_err(|e| e.to_string())?, (startup_write.as_raw_handle() as usize).to_string()];
            let path = real_exe()?;
            let process = create_process(&path, &args, &startup.StartupInfo, EXTENDED_STARTUPINFO_PRESENT, true, &job)?;
            drop(startup_write);
            drop(input_read);
            drop(output_write);
            let pty = Self { reader: Some(output_read), writer: Box::new(input_write), process, job, console: std::sync::Mutex::new(Some(console)) };
            use std::io::BufRead;
            let mut reply = String::new();
            std::io::BufReader::new(startup_read).read_line(&mut reply).map_err(|e| launch_error(&config.target, e))?;
            let reply: serde_json::Value = serde_json::from_str(&reply).map_err(|e| launch_error(&config.target, e))?;
            if let Some(error) = reply["error"].as_str() { return Err(error.to_owned()); }
            if reply["ready"] != true { return Err(launch_error(&config.target, "invalid command startup response")); }
            Ok(pty)
        }
    }
    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        let console = self.console.lock().unwrap();
        let Some(console) = console.as_ref() else { return Ok(()); };
        let hr = unsafe { ResizePseudoConsole(console.0, COORD { X: cols as i16, Y: rows as i16 }) };
        if hr < 0 { Err(io::Error::other(format!("cannot resize ConPTY: {hr:#x}"))) } else { Ok(()) }
    }
    pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
        unsafe {
            match WaitForSingleObject(self.process.0, 0) {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    if GetExitCodeProcess(self.process.0, &mut code) == 0 { return Err(io::Error::last_os_error()); }
                    Ok(Some(code as i32))
                }
                _ => Err(io::Error::last_os_error()),
            }
        }
    }
    pub fn terminate(&self) {
        unsafe { TerminateJobObject(self.job.0, 125); }
        drop(self.console.lock().unwrap().take());
    }
}
impl Drop for Pty { fn drop(&mut self) { self.terminate(); unsafe { WaitForSingleObject(self.process.0, INFINITE); } } }
fn console_file(name: &str) -> Result<File> {
    unsafe {
        let raw = CreateFileW(wide(name).as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE, null(), OPEN_EXISTING, 0, null_mut());
        if raw == INVALID_HANDLE_VALUE { return Err(last()); }
        Ok(File::from_raw_handle(raw))
    }
}
pub fn run_command(args: &[String]) -> Result<i32> {
    let raw: usize = args.get(2).ok_or("invalid command startup pipe")?.parse().map_err(|_| "invalid command startup pipe")?;
    let mut startup = unsafe { File::from_raw_handle(raw as *mut c_void) };
    let result = start_command(args, raw);
    match result {
        Ok(mut child) => {
            writeln!(startup, "{}", serde_json::json!({"ready":true})).map_err(|e| e.to_string())?;
            drop(startup);
            child.wait().map(status).map_err(|e| e.to_string())
        }
        Err(error) => {
            writeln!(startup, "{}", serde_json::json!({"error":error})).map_err(|e| e.to_string())?;
            Ok(125)
        }
    }
}
fn start_command(args: &[String], startup: usize) -> Result<std::process::Child> {
    if unsafe { SetHandleInformation(startup as HANDLE, HANDLE_FLAG_INHERIT, 0) } == 0 { return Err(last()); }
    ignore_ctrl_c()?;
    let config: Config = serde_json::from_str(args.get(1).ok_or("invalid command startup")?).map_err(|e| e.to_string())?;
    let extension = config.target.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
    let mut command = if extension == "cmd" || extension == "bat" {
        let comspec = nonempty("ComSpec").ok_or_else(|| launch_error(&config.target, "ComSpec is unset or empty"))?;
        let mut command = Command::new(comspec);
        let line = std::iter::once(config.target.to_string_lossy().into_owned()).chain(config.args.iter().cloned()).map(|s| quote(&s)).collect::<Vec<_>>().join(" ");
        command.args(["/d", "/v:off", "/s", "/c"]).raw_arg(format!("\"{line}\""));
        command
    } else {
        let mut command = Command::new(&config.target);
        command.args(&config.args);
        command
    };
    command.current_dir(&config.cwd).env("CLIMON2_SESSION_ID", &config.session_id).env("CLIMON2_SESSION_DEPTH", config.depth.to_string());
    if env::var_os("TERM").is_none() { command.env("TERM", "xterm-256color"); }
    // A ConPTY association does not replace inherited redirected standard handles.
    let input = console_file("CONIN$").map_err(|e| launch_error(&config.target, e))?;
    let output = console_file("CONOUT$").map_err(|e| launch_error(&config.target, e))?;
    let error = output.try_clone().map_err(|e| launch_error(&config.target, e))?;
    command.stdin(input).stdout(output).stderr(error);
    command.spawn().map_err(|e| launch_error(&config.target, e))
}
