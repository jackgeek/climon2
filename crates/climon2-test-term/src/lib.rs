//! Real-terminal testing at the process boundary (#21/#31).

mod isolation;
mod shell;
mod terminal;

pub use isolation::{Isolation, bin_path};
pub use shell::bash_profiles;
pub use terminal::Terminal;
