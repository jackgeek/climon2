//! Real-terminal testing at the process boundary (#21/#31).

mod cell;
mod isolation;
mod ownership;
mod shell;
mod terminal;

pub use cell::{Cell, Color, Cursor, CursorShape, Rendition, Underline, Width};
pub use isolation::{Isolation, bin_path};
pub use shell::controlled_bash;
pub use terminal::{Modes, Options, Terminal};
