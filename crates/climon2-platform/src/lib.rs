//! Shared platform seams from ADR 0003.
pub mod clock;
#[cfg(windows)]
#[path = "command/windows.rs"]
pub mod command;
pub mod connection;
pub mod daemon;
#[cfg(windows)]
#[path = "handoff/windows.rs"]
pub mod handoff;
pub mod home;
#[cfg(unix)]
pub mod home_lock;
#[cfg(unix)]
pub mod host_name;
#[cfg(unix)]
pub mod key_storage;
pub mod library;
#[cfg(unix)]
pub mod link_socket;
pub mod logging;
pub mod native_path;
pub mod notifier;
pub mod peer;
pub mod presentation;
pub mod private;
pub mod process;
pub mod pty;
pub mod randomness;
pub mod runtime;
#[cfg(unix)]
mod syscall;
pub mod terminal;
pub mod waiting;
