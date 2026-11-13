//! The session daemon's virtual terminal: a pure, platform-neutral adapter
//! around upstream Alacritty that keeps every engine type private.

mod bounded;
mod classify;
mod cost;
mod derive;
mod engine;
mod geometry;
mod history;
mod model;
mod modes;
mod presentation;
mod render;
mod restore;
mod sync;
mod title;
pub mod token;
mod vt;

pub use geometry::Geometry;
pub use model::{Limits, Mandatory, Model, Rejected};
pub use presentation::{Mode, Presentation};
pub use restore::{Chunk, Restoration};
pub use vt::{Fed, Vt, VtConfig};
