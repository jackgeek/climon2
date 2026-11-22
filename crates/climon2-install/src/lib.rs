//! The installer: it places the release beside it into a climon2 executable
//! home. macOS and GNU/Linux share one implementation (`unix.rs`); Windows has
//! its own (`windows.rs`, docs/for-humans/windows/distribution.md).
#[cfg(feature = "test-hooks")]
#[cfg(unix)]
mod hooks;
#[cfg(unix)]
#[path = "unix.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;

pub use native::*;
// The Unix modules name each other from the crate root.
#[cfg(unix)]
use native::{payload, platform, transaction};
