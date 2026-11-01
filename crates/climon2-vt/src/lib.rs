//! The session daemon's virtual terminal: a pure, platform-neutral adapter
//! around upstream Alacritty that keeps every engine type private.

mod classify;
mod engine;
mod geometry;
mod history;
mod modes;
mod presentation;
mod render;
mod restore;
mod sync;
mod title;
pub mod token;
mod vt;

pub use geometry::Geometry;
pub use presentation::{Mode, Presentation};
pub use restore::{Chunk, Restoration};
pub use vt::{Fed, Vt, VtConfig};
