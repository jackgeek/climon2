//! Running a program outside the test's job. cargo puts every test in a job
//! that refuses breakaway, so a `c2 exec` started from a test could never
//! leave it and would always exit 125 (docs/windows-lifecycle.md, "Starting
//! the daemon"). On the `windows-2025` runner the runner itself, from
//! `Runner.Listener` down, is in another job that refuses breakaway too, so
//! no process the test can reach as a parent is outside every job.
//!
//! The one documented way out is WMI: `Win32_Process.Create` starts a
//! process that is not in its caller's job, and `CREATE_BREAKAWAY_FROM_JOB`
//! takes it out of the WMI provider host's. The harness uses it to start the
//! test executable again, detached, as a relay: the test binary declares the
//! relay with [`outside_relay!`](crate::outside_relay). The relay reads a
//! request file, starts the program with the request's arguments,
//! environment, working directory and job shapes (outer job first), its
//! standard output and error going to files, waits for it, and writes its
//! exit status beside them.
//!
//! This is test code: it must pass its own tests in the Windows Test job and
//! is outside CRAP and mutation (docs/windows-quality.md).

use super::{conpty::wide, host::command_line, job::JobShape};
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    process::Command,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_OBJECT_0},
    System::Threading::{
        CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
        DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE,
        InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess,
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_INFORMATION,
        PROCESS_TERMINATE, STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess,
        UpdateProcThreadAttribute, WaitForSingleObject,
    },
};

/// The name of the test function [`outside_relay!`](crate::outside_relay)
/// declares, which the relay is started as.
pub const RELAY: &str = "climon2_outside_relay";

/// Declares the relay that [`Outside`] starts through WMI in a test binary
/// that uses it. Run as an ordinary test it does nothing.
#[macro_export]
macro_rules! outside_relay {
    () => {
        #[test]
        fn climon2_outside_relay() {
            $crate::relay_outside();
        }
    };
}

/// Set in a test run again by [`rerun_outside`].
const RERUN: &str = "CLIMON2_TEST_OUTSIDE";

/// Runs the test `name` of this test binary again outside every job, through
/// the relay, which the binary must declare with
/// [`outside_relay!`](crate::outside_relay), and fails unless it passes.
/// True when it ran there, so the caller returns; false in that second run,
/// where the caller runs the test's body. A daemon `c2 exec` starts from
/// the body can then leave every job, as it does under a real host.
pub fn rerun_outside(name: &str, timeout: Duration) -> bool {
    if std::env::var_os(RERUN).is_some() {
        return false;
    }
    let finished = Outside::new(std::env::current_exe().expect("this test binary"))
        .args(["--exact", name, "--nocapture", "--test-threads=1"])
        .env(RERUN, "1")
        .run(timeout)
        .expect("run the test outside every job");
    println!("{}", finished.stdout);
    eprintln!("{}", finished.stderr);
    assert!(
        finished.status == 0 && finished.stdout.contains("1 passed"),
        "{name} failed outside every job with status {:#x}",
        finished.status
    );
    true
}

/// What a program run outside the test's job left: its full exit status and
/// what it wrote.
#[derive(Debug)]
pub struct Finished {
    pub status: u32,
    pub stdout: String,
    pub stderr: String,
}

/// A program to run outside the test's job.
pub struct Outside {
    argv: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    cwd: PathBuf,
    jobs: Vec<JobShape>,
}

impl Outside {
    /// `program` with this process's environment, in this process's working
    /// directory, in no job.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            argv: vec![program.as_ref().to_owned()],
            environment: std::env::vars_os().collect(),
            cwd: std::env::current_dir().expect("current directory"),
            jobs: Vec::new(),
        }
    }

    pub fn args(mut self, args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> Self {
        self.argv
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    /// Sets one variable, replacing any inherited value (names compare
    /// without case, as Windows compares them).
    pub fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        let name = name.as_ref().to_owned();
        self.environment
            .retain(|(known, _)| !known.eq_ignore_ascii_case(&name));
        self.environment.push((name, value.as_ref().to_owned()));
        self
    }

    pub fn cwd(mut self, cwd: impl AsRef<Path>) -> Self {
        self.cwd = cwd.as_ref().to_owned();
        self
    }

    /// Starts the program in one more job of `shape`, inside any added before.
    pub fn job(mut self, shape: JobShape) -> Self {
        self.jobs.push(shape);
        self
    }

    /// Runs the program to its end through the relay; after `timeout` the
    /// relay is ended and the run fails.
    pub fn run(self, timeout: Duration) -> io::Result<Finished> {
        let scratch = tempfile::tempdir()?;
        let request = scratch.path().join("request");
        fs::write(&request, self.encode())?;
        let relay = start_relay(&request)?;
        let deadline = Instant::now() + timeout;
        let status = loop {
            if let Ok(text) = fs::read_to_string(request.with_extension("status")) {
                break text.trim().parse::<u32>().map_err(io::Error::other)?;
            }
            if Instant::now() > deadline {
                terminate(relay);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the program outside the test's job did not finish",
                ));
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        Ok(Finished {
            status,
            stdout: fs::read_to_string(request.with_extension("out"))?,
            stderr: fs::read_to_string(request.with_extension("err"))?,
        })
    }

    /// NUL-separated: the job shapes, the working directory, the argument
    /// count and arguments, then name and value of each variable.
    fn encode(&self) -> Vec<u8> {
        let shapes: String = self.jobs.iter().filter_map(|shape| code(*shape)).collect();
        let mut fields = vec![
            OsString::from(shapes),
            self.cwd.clone().into_os_string(),
            OsString::from(self.argv.len().to_string()),
        ];
        fields.extend(self.argv.iter().cloned());
        for (name, value) in &self.environment {
            fields.push(name.clone());
            fields.push(value.clone());
        }
        let mut bytes = Vec::new();
        for field in fields {
            bytes.extend(field.to_string_lossy().as_bytes());
            bytes.push(0);
        }
        bytes
    }
}

fn code(shape: JobShape) -> Option<char> {
    match shape {
        JobShape::None => None,
        JobShape::AllowsBreakaway => Some('A'),
        JobShape::RefusesBreakaway => Some('R'),
    }
}

fn shape(code: char) -> JobShape {
    match code {
        'A' => JobShape::AllowsBreakaway,
        _ => JobShape::RefusesBreakaway,
    }
}

/// Starts this test executable as the relay for `request` through WMI,
/// detached and breaking away from the provider host's job; its PID.
fn start_relay(request: &Path) -> io::Result<u32> {
    let executable = std::env::current_exe()?;
    let line = command_line(&[
        executable.into_os_string(),
        "--exact".into(),
        "--nocapture".into(),
        "--test-threads=1".into(),
        RELAY.into(),
        request.as_os_str().to_owned(),
    ])?;
    let line = String::from_utf16_lossy(&line[..line.len() - 1]);
    // DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB
    let script = "$startup = New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly \
                  -Property @{CreateFlags = [uint32]16777224}; \
                  $result = Invoke-CimMethod -ClassName Win32_Process -MethodName Create \
                  -Arguments @{CommandLine = $env:CLIMON2_RELAY_LINE; \
                  CurrentDirectory = $env:CLIMON2_RELAY_DIRECTORY; \
                  ProcessStartupInformation = $startup}; \
                  \"$($result.ReturnValue) $($result.ProcessId)\"";
    let output = Command::new("powershell")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("CLIMON2_RELAY_LINE", line)
        .env(
            "CLIMON2_RELAY_DIRECTORY",
            request.parent().expect("a folder"),
        )
        .output()?;
    let reply = String::from_utf8_lossy(&output.stdout);
    match reply.split_whitespace().collect::<Vec<_>>()[..] {
        ["0", pid] => pid.parse().map_err(io::Error::other),
        _ => Err(io::Error::other(format!(
            "Win32_Process.Create did not start the relay: {reply} {}",
            String::from_utf8_lossy(&output.stderr)
        ))),
    }
}

fn terminate(pid: u32) {
    // Opening by PID has no preconditions.
    let process = unsafe { OpenProcess(PROCESS_TERMINATE, 0, pid) };
    if !process.is_null() {
        // A new handle, open with PROCESS_TERMINATE, closed here.
        let process = unsafe { OwnedHandle::from_raw_handle(process) };
        unsafe { TerminateProcess(process.as_raw_handle(), 1) };
    }
}

/// The relay's body, run when this test executable was started as the
/// relay (see the module documentation); otherwise it does nothing.
pub fn relay_outside() {
    let arguments: Vec<String> = std::env::args().collect();
    let Some(position) = arguments.iter().position(|argument| argument == RELAY) else {
        return;
    };
    let Some(request) = arguments.get(position + 1).map(PathBuf::from) else {
        return;
    };
    let status = relay(&request).unwrap_or_else(|error| {
        let _ = fs::write(request.with_extension("err"), error.to_string());
        u32::MAX
    });
    let written = request.with_extension("written");
    fs::write(&written, status.to_string()).expect("write the status");
    fs::rename(&written, request.with_extension("status")).expect("publish the status");
}

fn relay(request: &Path) -> io::Result<u32> {
    let bytes = fs::read(request)?;
    let text = String::from_utf8_lossy(&bytes);
    let mut fields = text.split('\0');
    let mut next = || {
        fields
            .next()
            .map(OsString::from)
            .ok_or_else(|| io::Error::other("short request"))
    };
    let shapes = next()?.to_string_lossy().into_owned();
    let cwd = PathBuf::from(next()?);
    let count: usize = next()?
        .to_string_lossy()
        .parse()
        .map_err(io::Error::other)?;
    let argv = (0..count).map(|_| next()).collect::<io::Result<Vec<_>>>()?;
    let mut environment = Vec::new();
    while let (Ok(name), Ok(value)) = (next(), next()) {
        if name.is_empty() {
            break;
        }
        environment.push((name, value));
    }
    let jobs: Vec<OwnedHandle> = shapes
        .chars()
        .map(|code| {
            shape(code)
                .create()
                .map(|job| job.expect("a job shape with a job"))
        })
        .collect::<io::Result<_>>()?;
    let streams = [
        inheritable(File::open("NUL")?)?,
        inheritable(File::create(request.with_extension("out"))?)?,
        inheritable(File::create(request.with_extension("err"))?)?,
    ];
    let process = create(&argv, &environment, &cwd, &streams, &jobs)?;
    drop(streams);
    finish(&process)
}

fn inheritable(file: File) -> io::Result<File> {
    // The file's handle is open.
    if unsafe {
        SetHandleInformation(
            file.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(file)
}

fn create(
    argv: &[OsString],
    environment: &[(OsString, OsString)],
    cwd: &Path,
    streams: &[File; 3],
    jobs: &[OwnedHandle],
) -> io::Result<OwnedHandle> {
    let handles: [HANDLE; 3] = streams.each_ref().map(AsRawHandle::as_raw_handle);
    let job_raws: Vec<HANDLE> = jobs.iter().map(AsRawHandle::as_raw_handle).collect();
    let mut attributes = Attributes::new(1 + u32::from(!jobs.is_empty()))?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        handles.as_ptr().cast(),
        size_of_val(&handles),
    )?;
    if !jobs.is_empty() {
        attributes.update(
            PROC_THREAD_ATTRIBUTE_JOB_LIST,
            job_raws.as_ptr().cast(),
            size_of_val(job_raws.as_slice()),
        )?;
    }
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    let [input, output, error] = handles;
    startup.StartupInfo.hStdInput = input;
    startup.StartupInfo.hStdOutput = output;
    startup.StartupInfo.hStdError = error;
    startup.lpAttributeList = attributes.list();
    let mut line = command_line(argv)?;
    let environment = environment_block(environment);
    let directory = wide(cwd.as_os_str());
    let mut information = PROCESS_INFORMATION::default();
    // Every pointer is valid for the call; the handle list names open
    // inheritable handles.
    let created = unsafe {
        CreateProcessW(
            ptr::null(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            1,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT | DETACHED_PROCESS,
            environment.as_ptr().cast(),
            directory.as_ptr(),
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
            OwnedHandle::from_raw_handle(information.hProcess),
            OwnedHandle::from_raw_handle(information.hThread),
        )
    };
    Ok(process)
}

/// Waits for `process`; its exit status.
fn finish(process: &OwnedHandle) -> io::Result<u32> {
    // The handle is open with every access.
    unsafe {
        if WaitForSingleObject(process.as_raw_handle(), INFINITE) != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        let mut status = 0;
        if GetExitCodeProcess(process.as_raw_handle(), &mut status) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(status)
    }
}

fn environment_block(variables: &[(OsString, OsString)]) -> Vec<u16> {
    let mut block = Vec::new();
    for (name, value) in variables {
        block.extend(wide(
            &[name.as_os_str(), OsStr::new("="), value].join(OsStr::new("")),
        ));
    }
    block.extend([0, 0]);
    block
}

/// A process thread attribute list of a fixed number of attributes.
struct Attributes {
    buffer: Vec<u64>,
}

impl Attributes {
    fn new(count: u32) -> io::Result<Self> {
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
