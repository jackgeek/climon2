//! The session daemon's virtual terminal: a pure, platform-neutral adapter
//! around upstream Alacritty that keeps every engine type private.

mod geometry;
pub mod token;

pub use geometry::Geometry;
