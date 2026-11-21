//! Shared platform seams from ADR 0003.
#[cfg(unix)]
pub mod connection;
#[cfg(unix)]
pub mod daemon;
pub mod home;
#[cfg(unix)]
pub mod library;
#[cfg(unix)]
pub mod logging;
#[cfg(unix)]
pub mod peer;
#[cfg(unix)]
pub mod presentation;
#[cfg(unix)]
pub mod process;
#[cfg(unix)]
pub mod pty;
#[cfg(unix)]
pub mod runtime;
#[cfg(unix)]
mod syscall;
#[cfg(unix)]
pub mod terminal;
pub mod waiting;
