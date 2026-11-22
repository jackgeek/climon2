//! The Windows host (#320): the harness creates the pseudoconsole itself with
//! the product's pinned ConPTY and configures it as a qualified host does
//! (docs/windows-qualification.md, "Host models"). portable-pty cannot,
//! because it fixes the creation flags and only ends a process abruptly.
//!
//! This is test code: it must pass its own tests in the Windows Test job and
//! is outside CRAP and mutation (docs/windows-quality.md).

mod conpty;
mod host;
mod job;
mod keys;
mod outside;
mod pwsh;

pub use conpty::ConPty;
pub use host::{HostModel, HostSettings, Terminal};
pub use job::JobShape;
pub use keys::{Key, Modifiers};
pub use outside::{Finished, Outside};
pub use pwsh::{PWSH_PROMPT, controlled_pwsh, pwsh};
