//! Shared platform seams from ADR 0003.
pub mod connection;
pub mod daemon;
pub mod home;
pub mod library;
pub mod logging;
pub mod nonblocking;
pub mod peer;
pub mod presentation;
pub mod private_file;
pub mod process;
pub mod pty;
pub mod read_end;
pub mod readiness;
pub mod runtime;
mod syscall;
pub mod terminal;
