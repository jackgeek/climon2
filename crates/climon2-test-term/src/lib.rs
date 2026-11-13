//! Real-terminal testing at the process boundary (#21/#31).
//!
//! macOS and Linux share the terminal/session assertions. Process ownership uses
//! macOS kernel process/image ids or Linux procfs pid/start ticks and an inherited
//! test-owned descriptor. Neither trusts the environment marker alone, or requires
//! an orphan's parent to survive until the next process-table sample.

mod cell;
mod exit;
mod hello;
mod input;
mod isolation;
mod ownership;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub mod ptrace;
mod shell;
mod signals;
mod terminal;

pub use cell::{AnsiColor, Cell, Color, Cursor, CursorShape, Rendition, Underline, Width};
pub use exit::{ChildProcess, exit_within};
pub use hello::hello_peer;
pub use input::{Mouse, MouseButton};
pub use isolation::{Isolation, bin_path, copy_executable, output_within, process_command};
pub use shell::controlled_bash;
pub use signals::HostSignal;
pub use terminal::{InputTerminal, Modes, Options, Terminal};
