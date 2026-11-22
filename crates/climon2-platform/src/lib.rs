//! Shared platform seams from ADR 0003.
pub mod connection;
#[cfg(unix)]
pub mod daemon;
pub mod home;
#[cfg(unix)]
pub mod library;
pub mod logging;
#[cfg(unix)]
pub mod notifier;
pub mod peer;
pub mod presentation;
pub mod private;
pub mod process;
#[cfg(unix)]
pub mod pty;
pub mod runtime;
#[cfg(unix)]
mod syscall;
pub mod terminal;
pub mod waiting;
