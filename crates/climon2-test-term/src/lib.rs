//! Real-terminal testing at the process boundary (#21/#31).
//!
//! macOS and Linux share the terminal/session assertions. Process ownership uses
//! macOS kernel process/image ids or Linux procfs pid/start ticks; neither trusts
//! the environment marker alone. Linux ancestry must be observed before reparenting,
//! while macOS additionally retains the kernel's parent-image identity.

mod isolation;
mod ownership;
mod shell;
mod terminal;

pub use isolation::{Isolation, bin_path};
pub use shell::controlled_bash;
pub use terminal::{Modes, Options, Terminal};
