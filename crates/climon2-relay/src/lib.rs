//! The relay (spec #488): started by `c2 relay`, it binds its listener, owns
//! its terminal until it is stopped, and is the only thing that controls it.
//! Its pair key mints a link invitation, and it pairs with a hub monitor and
//! accepts the links of the hub monitors it remembers (#501). The sessions
//! it serves arrive with later tickets.
//!
//! The relay runs on macOS and Linux (`unix.rs`). On Windows it refuses to
//! start (`windows.rs`).

#[cfg(unix)]
mod desk;
#[cfg(unix)]
mod host;
#[cfg(unix)]
mod keys;
#[cfg(unix)]
mod links;
#[cfg(unix)]
#[path = "unix.rs"]
mod native;
#[cfg(windows)]
#[path = "windows.rs"]
mod native;
#[cfg(unix)]
mod screen;
#[cfg(unix)]
mod start;

/// A one-line diagnostic that the thin main prints, after the terminal is
/// restored, with its exit status.
pub use climon2_core::failure::Failure as Error;
pub use native::run;
