//! Real-terminal testing at the process boundary (#21/#31).

mod isolation;
mod ownership;
mod shell;
mod terminal;

pub use isolation::{Isolation, bin_path};
pub use shell::controlled_bash;
pub use terminal::Terminal;
