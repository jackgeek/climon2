//! The CLI's hand-offs on Windows (#321; docs/windows-lifecycle.md,
//! "Hand-offs"), by the launcher's rules. Windows cannot replace a running
//! process, so the CLI starts the client, waits for it and exits with its raw
//! 32-bit status. While it waits it shares the client's console: Ctrl+C and
//! Ctrl+Break are the client's, and on close, logoff or shutdown the CLI
//! gives the client three seconds to end before it exits 129.
use std::{io, process::Command, thread::sleep, time::Duration};
use windows_sys::Win32::System::{
    Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler},
    Threading::{GetCurrentProcess, TerminateProcess},
};

/// How long a close, logoff or shutdown waits for the child to end.
const CLOSE_WAIT: Duration = Duration::from_secs(3);
/// The status when the child outlasts that wait, as on Unix for a hang-up.
const HANGUP: u32 = 129;

/// Runs `command`, which inherits this process's console and standard
/// handles, and exits with its status. Returns only the reason it could not.
pub fn replace(mut command: Command) -> io::Error {
    if let Err(error) = wait_through_console_events() {
        return error;
    }
    match command.status() {
        // A Windows status always has a code, all 32 bits of it.
        Ok(status) => exit(status.code().map_or(HANGUP, |code| code as u32)),
        Err(error) => error,
    }
}

/// Keeps this process waiting through its child's console events, with a
/// handler routine, never the ignore flag, which the child would inherit.
pub fn wait_through_console_events() -> io::Result<()> {
    // SAFETY: a handler routine that lives as long as the process.
    if unsafe { SetConsoleCtrlHandler(Some(console_event), 1) } == 0 {
        let error = io::Error::last_os_error();
        return Err(io::Error::new(
            error.kind(),
            format!("cannot handle console events: {error}"),
        ));
    }
    Ok(())
}

/// Ctrl+C and Ctrl+Break are the child's to handle. Any other event ends the
/// console session: if the child ends within the wait, the waiting thread
/// exits with its status first; otherwise this exits 129.
unsafe extern "system" fn console_event(event: u32) -> i32 {
    match event {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => 1,
        _ => {
            sleep(CLOSE_WAIT);
            exit(HANGUP)
        }
    }
}

/// Ends this process with `status` at once. `std::process::exit` sets the
/// status only after unloading the process's libraries, and a close event
/// arriving meanwhile replaces it with `STATUS_CONTROL_C_EXIT`.
fn exit(status: u32) -> ! {
    // SAFETY: this process's own pseudo-handle.
    unsafe { TerminateProcess(GetCurrentProcess(), status) };
    std::process::exit(status as i32)
}
