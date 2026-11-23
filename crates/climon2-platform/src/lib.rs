//! Shared platform seams from ADR 0003.
#[cfg(windows)]
#[path = "command/windows.rs"]
pub mod command;
pub mod connection;
pub mod daemon;
#[cfg(windows)]
#[path = "handoff/windows.rs"]
pub mod handoff;
pub mod home;
pub mod library;
pub mod logging;
pub mod native_path;
pub mod notifier;
pub mod peer;
pub mod presentation;
pub mod private;
pub mod process;
pub mod pty;
pub mod runtime;
#[cfg(unix)]
mod syscall;
pub mod terminal;
pub mod waiting;
