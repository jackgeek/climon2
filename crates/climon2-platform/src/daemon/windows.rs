//! Starting the daemon on Windows (#317; docs/windows-lifecycle.md,
//! "Starting the daemon").
//!
//! One `CreateProcessW`: detached, breaking away when this process is in a
//! job, never in a new process group or console, with null standard handles,
//! in the daemon's own folder, and suspended until checked. Its readiness
//! comes back on an inheritable pipe that a one-entry handle list limits to
//! the daemon, whose handle value the daemon is given as an argument.
//! Breakaway leaves only the innermost job, so a daemon still in a job
//! afterwards is ended and refused: it would die with its host.
use super::{Readiness, ReadinessWriter};
use crate::pty::{Attributes, check, command_line, started, wide};
use climon2_core::protocol::STARTUP_TIMEOUT;
use std::{
    ffi::OsStr,
    fs::File,
    io::{self, PipeReader, Read},
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle},
    path::Path,
    process::Command,
    ptr,
    sync::mpsc,
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, TRUE},
    System::{
        JobObjects::IsProcessInJob,
        Threading::{
            CREATE_BREAKAWAY_FROM_JOB, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT,
            CreateProcessW, DETACHED_PROCESS, EXTENDED_STARTUPINFO_PRESENT, GetCurrentProcess,
            INFINITE, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, ResumeThread,
            STARTF_USESTDHANDLES, STARTUPINFOEXW, TerminateProcess, WaitForSingleObject,
        },
    },
};

/// The exit status a daemon ended before it was ready gets.
const ABORTED: u32 = 125;

impl ReadinessWriter {
    /// # Safety
    /// `handle` must be the inherited readiness pipe, owned exclusively by
    /// the caller and not represented by another Rust I/O owner.
    pub unsafe fn from_inherited(handle: u32) -> io::Result<Self> {
        let handle = handle as usize as HANDLE;
        // Nothing the daemon starts inherits the pipe; failing here also
        // refuses a value that names no open handle. Spelled `0u32`: the
        // command starts with inheritance off, so leaving the flag set would
        // be an equivalent mutant.
        check(unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0u32) })?;
        // As the caller guarantees.
        Ok(Self(File::from(unsafe {
            OwnedHandle::from_raw_handle(handle)
        })))
    }
}

/// Starts `command`'s program with its arguments and exactly the variables
/// set on it, which callers clear first, as `exec` does. `configure` adds the
/// readiness handle's argument, never to the eventual command's environment.
pub fn spawn(
    mut command: Command,
    configure: impl FnOnce(&mut Command, u32),
) -> io::Result<PendingDaemon> {
    let (reader, writer) = io::pipe()?;
    let writer = OwnedHandle::from(writer);
    // The write end is open; only the handle list passes it on.
    check(unsafe {
        SetHandleInformation(
            writer.as_raw_handle(),
            HANDLE_FLAG_INHERIT,
            HANDLE_FLAG_INHERIT,
        )
    })?;
    configure(&mut command, writer.as_raw_handle() as usize as u32);
    let in_job = in_job(current())?;
    let started = Instant::now();
    let created = create(&command, writer.as_raw_handle(), in_job);
    drop(writer);
    let (process, thread, pid) = created.map_err(|error| refused(error, in_job))?;
    let daemon = PendingDaemon {
        pid,
        process: Some(process),
        reader: Some(reader),
        started,
    };
    daemon.resume(&thread)?;
    Ok(daemon)
}

fn current() -> RawHandle {
    // The pseudo-handle needs no closing.
    unsafe { GetCurrentProcess() }
}

fn in_job(process: RawHandle) -> io::Result<bool> {
    let mut result = TRUE;
    // The handle is open; the call writes one BOOL.
    check(unsafe { IsProcessInJob(process, ptr::null_mut(), &mut result) })?;
    Ok(result == TRUE)
}

/// A spawn that fails while this process is in a job is a job refusing
/// breakaway.
fn refused(error: io::Error, in_job: bool) -> io::Error {
    if in_job {
        return io::Error::other(format!(
            "the session daemon cannot leave the job this command runs in: {error}"
        ));
    }
    error
}

fn create(
    command: &Command,
    readiness: RawHandle,
    in_job: bool,
) -> io::Result<(OwnedHandle, OwnedHandle, u32)> {
    let program = Path::new(command.get_program());
    let mut line = command_line(std::iter::once(program.as_os_str()).chain(command.get_args()));
    let environment = environment(command);
    let directory = wide(program.parent().unwrap_or(program).as_os_str());
    let handles = [readiness];
    let mut attributes = Attributes::new(1)?;
    attributes.update(
        PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
        handles.as_ptr().cast(),
        size_of_val(&handles),
    )?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = attributes.list();
    let mut information = PROCESS_INFORMATION::default();
    // NUL-terminated program, directory and mutable command line, a
    // double-NUL-terminated UTF-16 environment and the extended startup
    // information; the handle list holds an open inheritable handle.
    check(unsafe {
        CreateProcessW(
            wide(program.as_os_str()).as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            TRUE,
            flags(in_job),
            environment.as_ptr().cast(),
            directory.as_ptr(),
            &startup.StartupInfo,
            &mut information,
        )
    })?;
    Ok(started(information))
}

/// Never `CREATE_NEW_PROCESS_GROUP`, which would make the command ignore
/// Ctrl+C, nor `CREATE_NEW_CONSOLE`, which can open a terminal window.
fn flags(in_job: bool) -> u32 {
    let breakaway = if in_job { CREATE_BREAKAWAY_FROM_JOB } else { 0 };
    DETACHED_PROCESS
        | CREATE_SUSPENDED
        | CREATE_UNICODE_ENVIRONMENT
        | EXTENDED_STARTUPINFO_PRESENT
        | breakaway
}

/// The variables set on `command`, as a UTF-16 block; the extra NULs end it
/// even when it is empty.
fn environment(command: &Command) -> Vec<u16> {
    let mut block = Vec::new();
    for (name, value) in command.get_envs() {
        if let Some(value) = value {
            let entry = [name, OsStr::new("="), value].join(OsStr::new(""));
            block.extend(wide(&entry));
        }
    }
    block.extend([0u16, 0u16]);
    block
}

pub struct PendingDaemon {
    pid: u32,
    /// Present until the daemon is ready or aborted; dropping ends it.
    process: Option<OwnedHandle>,
    reader: Option<PipeReader>,
    started: Instant,
}

impl PendingDaemon {
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Lets the suspended daemon run, unless it is still in a job.
    fn resume(&self, thread: &OwnedHandle) -> io::Result<()> {
        let process = self.process.as_ref().expect("a daemon not yet ready");
        if in_job(process.as_raw_handle())? {
            return Err(io::Error::other(
                "the session daemon cannot leave the job this command runs in",
            ));
        }
        // The thread handle is open with THREAD_SUSPEND_RESUME.
        if unsafe { ResumeThread(thread.as_raw_handle()) } == u32::MAX {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Supply metadata then socket as cleanup paths. Success leaves the
    /// daemon running; every failure ends it before deleting files.
    pub fn wait_ready(mut self, cleanup: &[&Path]) -> io::Result<Readiness> {
        let result = self.readiness();
        if matches!(result, Ok(Readiness::Ready)) {
            // A ready daemon runs on: dropping its handle does not end it.
            self.process.take();
            return result;
        }
        self.abort();
        for path in cleanup {
            let _ = std::fs::remove_file(path);
        }
        result
    }

    /// The reply, read on a thread of its own, which ends with the pipe
    /// when the daemon is ended at the deadline.
    fn readiness(&mut self) -> io::Result<Readiness> {
        let mut reader = self.reader.take().expect("an unread readiness pipe");
        let (reply, replied) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = reply.send(super::decode(&mut |bytes| reader.read_exact(bytes)));
        });
        let remaining = (self.started + STARTUP_TIMEOUT).saturating_duration_since(Instant::now());
        replied.recv_timeout(remaining).unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "daemon readiness timed out",
            ))
        })
    }

    fn abort(&mut self) {
        if let Some(process) = self.process.take() {
            // The handle is open with PROCESS_TERMINATE and SYNCHRONIZE.
            unsafe {
                TerminateProcess(process.as_raw_handle(), ABORTED);
                WaitForSingleObject(process.as_raw_handle(), INFINITE);
            }
        }
    }
}

impl Drop for PendingDaemon {
    fn drop(&mut self) {
        self.abort();
    }
}
