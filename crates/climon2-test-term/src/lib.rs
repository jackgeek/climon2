//! Real-terminal testing at the process boundary (#21/#31).
//!
//! macOS and Linux share the terminal/session assertions. Process ownership uses
//! macOS kernel process/image ids or Linux procfs pid/start ticks, plus an inherited
//! test-owned descriptor on both. Neither trusts the environment marker alone, or requires
//! an orphan's parent to survive until the next process-table sample.
//!
//! On Windows the same emulated terminal hosts its program in a pseudoconsole that
//! the harness creates directly with the product's pinned ConPTY, configured as a
//! qualified host does (#320, `windows`).

mod cell;
#[cfg(unix)]
mod exit;
#[cfg(unix)]
mod hello;
mod input;
#[cfg(unix)]
mod isolation;
#[cfg(unix)]
mod ownership;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub mod ptrace;
#[cfg(unix)]
mod shell;
mod signals;
mod terminal;
#[cfg(windows)]
mod windows;

pub use cell::{AnsiColor, Cell, Color, Cursor, CursorShape, Rendition, Underline, Width};
#[cfg(unix)]
pub use exit::{ChildProcess, exit_within};
#[cfg(unix)]
pub use hello::hello_peer;
pub use input::{Mouse, MouseButton};
#[cfg(unix)]
pub use isolation::{
    Isolation, bin_path, client_library_path, copy_executable, output_within, process_command,
};
#[cfg(unix)]
pub use shell::controlled_bash;
pub use signals::HostSignal;
#[cfg(unix)]
pub use terminal::InputTerminal;
pub use terminal::{Modes, Options, Terminal};
#[cfg(windows)]
pub use windows::{
    App, ConPty, Finished, HostModel, HostSettings, JobShape, Key, Modifiers, Outside, PWSH_PROMPT,
    controlled_pwsh, pwsh, relay_outside, rerun_outside,
};
