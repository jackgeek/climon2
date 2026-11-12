//! Real-terminal testing at the process boundary (#21/#31).
//!
//! macOS and Linux share the terminal/session assertions. Process ownership uses
//! macOS kernel process/image ids or Linux procfs pid/start ticks and an inherited
//! test-owned descriptor. Neither trusts the environment marker alone, or requires
//! an orphan's parent to survive until the next process-table sample.

mod isolation;
mod ownership;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
pub mod ptrace;
mod shell;
mod terminal;

pub use isolation::{Isolation, bin_path, copy_executable, process_command};
pub use shell::controlled_bash;
pub use terminal::{InputTerminal, Modes, Options, Terminal};
