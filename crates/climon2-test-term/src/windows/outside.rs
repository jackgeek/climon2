//! Running a program outside the test's job. cargo puts every test in a job
//! that refuses breakaway, so a `c2 exec` started from a test could never
//! leave it and would always exit 125 (docs/windows-lifecycle.md, "Starting
//! the daemon"). The harness starts the program instead as a child of an
//! ancestor above the outermost cargo, through
//! `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`, optionally in jobs of the
//! harness's own shapes, outer job first. The ancestor runs as the test's
//! user: the nearest one in no job, else the process that started cargo.
//! On the CI runner every process from `Runner.Listener` down is in the
//! runner's job, so there it is the step's shell, in that job. The program
//! is detached, since that process may have no console to share, and its
//! standard streams are files.
//!
//! This is test code: it must pass its own tests in the Windows Test job and
//! is outside CRAP and mutation (docs/windows-quality.md).

use super::{conpty::wide, host::command_line, job::JobShape};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    fs::{self, File},
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    ptr,
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::{
        DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, WAIT_OBJECT_0,
    },
    Security::{GetLengthSid, GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
        JobObjects::IsProcessInJob,
        Threading::OpenProcessToken,
        Threading::{
            CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS,
            DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
            GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
            OpenProcess, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROC_THREAD_ATTRIBUTE_JOB_LIST,
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS, PROCESS_CREATE_PROCESS, PROCESS_DUP_HANDLE,
            PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, STARTF_USESTDHANDLES,
            STARTUPINFOEXW, TerminateProcess, WaitForSingleObject,
        },
    },
};

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

    /// Runs the program to its end, or ends it after `timeout`.
    pub fn run(self, timeout: Duration) -> io::Result<Finished> {
        let jobs: Vec<OwnedHandle> = self
            .jobs
            .iter()
            .map(|shape| {
                shape
                    .create()
                    .map(|job| job.expect("a job shape with a job"))
            })
            .collect::<io::Result<_>>()?;
        let parent = cargo_starter()?;
        let scratch = tempfile::tempdir()?;
        let (stdout, stderr) = (scratch.path().join("out"), scratch.path().join("err"));
        let process = {
            let streams = [
                File::open("NUL")?,
                File::create(&stdout)?,
                File::create(&stderr)?,
            ];
            let remote = Remote::duplicate(&parent, &streams)?;
            self.create(&parent, &remote, &jobs)?
        };
        let status = finish(&process, timeout)?;
        drop(jobs);
        Ok(Finished {
            status,
            stdout: fs::read_to_string(&stdout)?,
            stderr: fs::read_to_string(&stderr)?,
        })
    }

    fn create(
        &self,
        parent: &OwnedHandle,
        remote: &Remote,
        jobs: &[OwnedHandle],
    ) -> io::Result<OwnedHandle> {
        let parent_raw = parent.as_raw_handle();
        let job_raws: Vec<HANDLE> = jobs.iter().map(AsRawHandle::as_raw_handle).collect();
        let mut attributes = Attributes::new(2 + u32::from(!jobs.is_empty()))?;
        attributes.update(
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
            (&raw const parent_raw).cast(),
            size_of::<HANDLE>(),
        )?;
        attributes.update(
            PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
            remote.handles.as_ptr().cast(),
            size_of_val(&remote.handles),
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
        let [input, output, error] = remote.handles;
        startup.StartupInfo.hStdInput = input;
        startup.StartupInfo.hStdOutput = output;
        startup.StartupInfo.hStdError = error;
        startup.lpAttributeList = attributes.list();
        let mut line = command_line(&self.argv)?;
        let environment = environment(&self.environment);
        let directory = wide(self.cwd.as_os_str());
        let mut information = PROCESS_INFORMATION::default();
        // Every pointer is valid for the call; the handle list names handles
        // that are open and inheritable in `parent`.
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
}

/// Waits for `process`, ending it at `timeout`; its exit status.
fn finish(process: &OwnedHandle, timeout: Duration) -> io::Result<u32> {
    let milliseconds = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
    // The handle is open with every access.
    unsafe {
        if WaitForSingleObject(process.as_raw_handle(), milliseconds) != WAIT_OBJECT_0 {
            TerminateProcess(process.as_raw_handle(), 1);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the program outside the test's job did not finish",
            ));
        }
        let mut status = 0;
        if GetExitCodeProcess(process.as_raw_handle(), &mut status) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(status)
    }
}

/// The parent to start a program under, running as this test's user: the
/// nearest ancestor above the outermost cargo that is in no job, else the
/// process that started that cargo, though it is in a job, as the CI
/// runner's step shell is. Without either, the error lists every ancestor.
fn cargo_starter() -> io::Result<OwnedHandle> {
    let processes = processes()?;
    let ours = user(current_process()).ok_or_else(io::Error::last_os_error)?;
    let mut ancestors = Vec::new();
    let mut pid = std::process::id();
    // A reused PID can make parents form a cycle (PID 0 is its own parent).
    while let Some((parent, _)) = processes.get(&pid) {
        if *parent == pid || ancestors.iter().any(|(seen, _)| seen == parent) {
            break;
        }
        let name = processes.get(parent).map_or("?", |(_, name)| name.as_str());
        ancestors.push((*parent, name));
        pid = *parent;
    }
    let outermost_cargo = ancestors
        .iter()
        .rposition(|(_, name)| name.eq_ignore_ascii_case("cargo.exe"));
    let starter = outermost_cargo.map_or(0, |cargo| cargo + 1);
    let mut seen = Vec::new();
    let mut fallback = None;
    for (index, (candidate, name)) in ancestors.iter().enumerate().skip(starter) {
        let (process, in_job, user) = inspect(*candidate);
        let same_user = user.as_ref() == Some(&ours);
        seen.push(format!(
            "{candidate} {name} in_job={in_job:?} same_user={same_user}"
        ));
        match (process, in_job, same_user) {
            (Some(process), Some(false), true) => return Ok(process),
            (Some(process), _, true) if index == starter => fallback = Some(process),
            _ => {}
        }
    }
    fallback.ok_or_else(|| {
        io::Error::other(format!(
            "the process that started cargo cannot start a program as this test's user: {}",
            seen.join("; ")
        ))
    })
}

fn current_process() -> HANDLE {
    // The pseudo-handle needs no closing.
    unsafe { GetCurrentProcess() }
}

/// The process `pid` opened to be a parent, whether it is in a job, and its
/// user's SID.
fn inspect(pid: u32) -> (Option<OwnedHandle>, Option<bool>, Option<Vec<u8>>) {
    // Opening by PID has no preconditions.
    let raw = unsafe {
        OpenProcess(
            PROCESS_CREATE_PROCESS | PROCESS_DUP_HANDLE | PROCESS_QUERY_LIMITED_INFORMATION,
            0,
            pid,
        )
    };
    if raw.is_null() {
        return (None, None, None);
    }
    // A new handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut in_job = 1;
    // The handle is open; the call writes one BOOL.
    let asked = unsafe { IsProcessInJob(raw, ptr::null_mut(), &mut in_job) };
    let user = user(raw);
    (Some(process), (asked != 0).then_some(in_job != 0), user)
}

/// The SID of the user `process` runs as.
fn user(process: HANDLE) -> Option<Vec<u8>> {
    let mut token = ptr::null_mut();
    // The process handle is open with PROCESS_QUERY_LIMITED_INFORMATION.
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return None;
    }
    // A new handle nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut size = 0;
    // Asks for the size only.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut size,
        )
    };
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    // The buffer holds `size` bytes, aligned for TOKEN_USER.
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buffer.as_mut_ptr().cast(),
            size,
            &mut size,
        )
    } == 0
    {
        return None;
    }
    // The buffer now starts with a TOKEN_USER whose SID points into it.
    unsafe {
        let sid = (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid;
        let length = GetLengthSid(sid) as usize;
        Some(std::slice::from_raw_parts(sid.cast::<u8>(), length).to_vec())
    }
}

/// Every process's parent PID and executable name.
fn processes() -> io::Result<HashMap<u32, (u32, String)>> {
    // A snapshot of every process.
    let raw = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if raw as isize == -1 {
        return Err(io::Error::last_os_error());
    }
    // A new handle nothing else owns.
    let snapshot = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut parents = HashMap::new();
    // The snapshot is open and `entry` has its size set.
    let mut more = unsafe { Process32FirstW(snapshot.as_raw_handle(), &mut entry) } != 0;
    while more {
        let name = String::from_utf16_lossy(&entry.szExeFile);
        let name = name.trim_end_matches(char::from(0)).to_owned();
        parents.insert(entry.th32ProcessID, (entry.th32ParentProcessID, name));
        // As above.
        more = unsafe { Process32NextW(snapshot.as_raw_handle(), &mut entry) } != 0;
    }
    Ok(parents)
}

/// Inheritable duplicates of the standard streams inside the parent, closed
/// there when dropped.
struct Remote<'a> {
    parent: &'a OwnedHandle,
    handles: [HANDLE; 3],
}

impl<'a> Remote<'a> {
    fn duplicate(parent: &'a OwnedHandle, streams: &[File; 3]) -> io::Result<Self> {
        let mut remote = Self {
            parent,
            handles: [ptr::null_mut(); 3],
        };
        for (stream, slot) in streams.iter().zip(&mut remote.handles) {
            // Both processes are open with PROCESS_DUP_HANDLE.
            let duplicated = unsafe {
                DuplicateHandle(
                    GetCurrentProcess(),
                    stream.as_raw_handle(),
                    parent.as_raw_handle(),
                    slot,
                    0,
                    1,
                    DUPLICATE_SAME_ACCESS,
                )
            };
            if duplicated == 0 {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(remote)
    }
}

impl Drop for Remote<'_> {
    fn drop(&mut self) {
        for handle in self.handles.into_iter().filter(|handle| !handle.is_null()) {
            // Closes the duplicate inside the parent.
            unsafe {
                DuplicateHandle(
                    self.parent.as_raw_handle(),
                    handle,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    0,
                    0,
                    DUPLICATE_CLOSE_SOURCE,
                )
            };
        }
    }
}

fn environment(variables: &[(OsString, OsString)]) -> Vec<u16> {
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

use windows_sys::Win32::System::Threading::UpdateProcThreadAttribute;
