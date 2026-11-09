//! Real-terminal testing at the process boundary (#21/#31).

mod cell;
mod input;
mod isolation;
mod ownership;
mod shell;
mod signals;
mod terminal;

pub use cell::{Cell, Color, Cursor, CursorShape, Rendition, Underline, Width};
pub use input::{Mouse, MouseButton};
pub use isolation::{Isolation, bin_path};
pub use shell::controlled_bash;
pub use signals::HostSignal;
pub use terminal::{Modes, Options, Terminal};
