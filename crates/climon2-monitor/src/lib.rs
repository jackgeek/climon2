//! The monitor library (ADR 0011): a whole monitor with no interface of
//! its own. It discovers sessions, keeps one passive client per live
//! session through client embedding, and decides selection, control,
//! viewing, acknowledgement, the attention jump, Kill and Delete, each
//! session's terminal model, memory relief, the selector's rows and search
//! matching, the toast queue, and client shutdown with its verdict.
//!
//! A consumer (the terminal interface, `climon2-monitor-tui`) owns one
//! [`Monitor`] on one thread and steps it. It sends [`Action`]s, delivers
//! [`Event`]s its own way, answers what a [`Consumer`] reports, and reads
//! rows, toasts and the selected session's read-only views. The library
//! never reads or writes the process's terminal, and takes the time, the
//! client library loader and the climon2 home as inputs.

mod action;
mod attention;
mod cadence;
mod consumer;
mod delete;
mod discovery;
#[cfg(feature = "test-hooks")]
pub mod hooks;
mod initial;
#[cfg(unix)]
pub mod links;
mod machine;
mod monitor;
mod pressure;
mod registry;
mod remote;
pub mod search;
mod session;
mod shutdown;
mod startup;
mod supervisor;
mod terminal;
pub mod toast;

pub use action::{Action, Choice, Drive, Look, Paste};
pub use attention::NO_ATTENTION;
/// A one-line diagnostic for a consumer to print with the climon2 error
/// code.
pub use climon2_core::failure::Failure as Error;
/// The client library loader a monitor is given.
pub use climon2_embed::load;
/// Read-only views of a session's terminal model: all of `climon2-vt` a
/// consumer sees (ADR 0011, rule 4).
pub use climon2_vt::view;
pub use consumer::Consumer;
pub use delete::fallback;
pub use machine::{Machine, SessionAt};
pub use monitor::{Clock, Event, Inputs, Monitor, System, TICK, Taking, Target};
pub use registry::{LOCAL, News, Offer, Recovery, View, truncate};
pub use session::{Canvas, GONE, Placeholder, Session, diagnostic};
pub use shutdown::Verdict;
pub use startup::{Startup, baseline, own_session};
pub use terminal::{Authority, Control};
