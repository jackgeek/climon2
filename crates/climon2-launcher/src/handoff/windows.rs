//! The Windows hand-off (ADR 0002; docs/for-humans/windows/lifecycle.md, "Hand-offs").
//! Windows cannot replace a running process, so the launcher starts the CLI,
//! waits for it and exits with its raw 32-bit status. While it waits it shares
//! the CLI's console: it survives Ctrl+C and Ctrl+Break, which the CLI handles,
//! and on close, logoff or shutdown it gives the CLI three seconds to end.
use std::ffi::c_void;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The CLI's file name in a version folder.
pub(super) const CLI: &str = "climon2-cli.exe";

/// How long a `current` that is missing or denied is retried: an installer
/// renames a new `current` over the old one, and a reader can briefly find it
/// missing (docs/for-humans/windows/distribution.md).
const CURRENT_RETRY: Duration = Duration::from_secs(1);
const CURRENT_POLL: Duration = Duration::from_millis(10);

/// How long a close, logoff or shutdown waits for the CLI to end.
const CLOSE_WAIT: Duration = Duration::from_secs(3);
/// The status when the CLI outlasts that wait, as on Unix for a hang-up.
const HANGUP: u32 = 129;

const CTRL_C_EVENT: u32 = 0;
const CTRL_BREAK_EVENT: u32 = 1;
const TRUE: i32 = 1;

#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
    fn GetCurrentProcess() -> *mut c_void;
    fn TerminateProcess(process: *mut c_void, status: u32) -> i32;
}

pub(super) fn read_current(path: &Path) -> io::Result<String> {
    let deadline = Instant::now() + CURRENT_RETRY;
    loop {
        let contents = std::fs::read_to_string(path);
        if !replacing(&contents) || deadline.saturating_duration_since(Instant::now()).is_zero() {
            return contents;
        }
        sleep(CURRENT_POLL);
    }
}

/// Whether a failed read can be an installer replacing `current`.
fn replacing(contents: &io::Result<String>) -> bool {
    contents.as_ref().is_err_and(|error| {
        matches!(
            error.kind(),
            io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
        )
    })
}

/// Runs the CLI with this launcher's arguments, environment, working
/// directory and standard handles, and exits with its status. Returns only
/// the reason it could not.
pub(super) fn hand_off(path: PathBuf) -> io::Error {
    // A handler routine, never the ignore flag, which the CLI would inherit.
    if unsafe { SetConsoleCtrlHandler(Some(console_event), TRUE) } == 0 {
        let error = io::Error::last_os_error();
        return io::Error::new(
            error.kind(),
            format!("cannot handle console events: {error}"),
        );
    }
    match Command::new(&path)
        .args(std::env::args_os().skip(1))
        .status()
    {
        // A Windows status always has a code, all 32 bits of it.
        Ok(status) => exit(status.code().map_or(HANGUP, |code| code as u32)),
        Err(error) => io::Error::new(
            error.kind(),
            format!("cannot execute climon2-cli at {path:?}: {error}"),
        ),
    }
}

/// Ctrl+C and Ctrl+Break are the CLI's to handle. Any other event ends the
/// console session: if the CLI ends within the wait, the waiting main thread
/// exits with its status first; otherwise this exits 129.
unsafe extern "system" fn console_event(event: u32) -> i32 {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => TRUE,
        _ => {
            sleep(CLOSE_WAIT);
            exit(HANGUP)
        }
    }
}

/// Ends the launcher with `status` at once. `std::process::exit` sets the
/// status only after unloading the process's libraries, and a close event
/// arriving meanwhile replaces it with `STATUS_CONTROL_C_EXIT`.
fn exit(status: u32) -> ! {
    unsafe { TerminateProcess(GetCurrentProcess(), status) };
    std::process::exit(status as i32)
}
