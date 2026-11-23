//! The daemon's command terminal on Windows (#317; ADR 0003, "Windows I/O and
//! transport"; docs/for-humans/windows/lifecycle.md, "Starting the command").
//!
//! The command runs in a pseudoconsole of the bundled ConPTY loaded from the
//! daemon's own folder, inside the session job; the platform seam does the
//! Windows work. The daemon resolves the command through the session's
//! `PATH` and `PATHEXT` (#322), and `CreateProcessW` reports a command that
//! cannot start, so there is no exec helper and no startup socket as on Unix.
use crate::{args::Options, startup::Failure};
use climon2_core::{protocol::TerminalSize, session::TERMINAL_IDENTITY, wire::CommandResult};
use climon2_platform::{
    command::{self, Refusal},
    pty::{self, ConPty, Hosted},
};
use std::{ffi::OsString, io, path::PathBuf, time::Duration};

pub(crate) struct Pty {
    hosted: Hosted,
    /// The command's result; `None` until its leader has exited.
    status: Option<CommandResult>,
}

impl Pty {
    pub fn spawn(options: &Options, _startup: ExecListener) -> Result<Self, Failure> {
        let opened = pseudoconsole(options)?;
        let hosted = opened
            .start(&resolved(options)?, &options.cwd, &environment(options))
            .map_err(|error| {
                tracing::error!(%error, "the command cannot start");
                Failure::launch(126)
            })?;
        tracing::debug!(command_pid = hosted.pid(), "command pseudoconsole created");
        Ok(Self {
            hosted,
            status: None,
        })
    }

    pub fn pid(&self) -> u32 {
        self.hosted.pid()
    }

    pub fn resize(&self, value: TerminalSize) -> io::Result<()> {
        self.hosted.resize(value.rows, value.cols)
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.hosted.write(bytes)
    }

    pub fn wait_readable(&self, timeout: Duration) {
        self.hosted.wait_readable(timeout);
    }

    pub fn read_output(&self, bytes: &mut [u8]) -> io::Result<usize> {
        self.hosted.read(bytes)
    }

    /// Learns the command ended from its leader's handle, never the job.
    pub fn try_wait(&mut self) -> io::Result<Option<CommandResult>> {
        if self.status.is_none() {
            self.status = self.hosted.try_wait()?.map(exited);
        }
        Ok(self.status.clone())
    }

    pub fn result(&self) -> Option<CommandResult> {
        self.status.clone()
    }

    /// Closes the pseudoconsole; `end_hangup` follows the grace.
    pub fn hang_up(&mut self) -> io::Result<()> {
        self.hosted.close();
        Ok(())
    }

    /// Ends what is left in the session job; the leader's status as it stands.
    pub fn end_hangup(&mut self) -> io::Result<CommandResult> {
        let result = self.hosted.end().map(exited);
        self.status = result.as_ref().ok().cloned().or(self.status.take());
        result
    }

    /// The daemon loop failed, so the command session gets its grace here.
    pub fn abandon(&mut self) {
        let _ = self.hang_up();
        std::thread::sleep(climon2_core::protocol::HANGUP_GRACE);
        let _ = self.end_hangup();
    }

    pub fn finish(&mut self) -> io::Result<()> {
        self.hosted.finish()
    }

    /// Windows has no terminal job groups to record: the session job holds
    /// every process.
    pub fn observe_jobs(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The command's pseudoconsole, of the bundled ConPTY, not yet running it.
fn pseudoconsole(options: &Options) -> Result<pty::Opened, Failure> {
    let conpty = ConPty::load(&own_folder()?)
        .map_err(|_| Failure::own("the bundled ConPTY is missing or cannot be loaded"))?;
    pty::open(conpty, options.size.rows, options.size.cols)
        .map_err(|_| Failure::own("cannot create the command's pseudoconsole"))
}

/// The command as `CreateProcessW` is to start it, found through the
/// session's `PATH` and `PATHEXT`.
fn resolved(options: &Options) -> Result<command::Launch, Failure> {
    command::launch(&options.command, &options.arguments, &options.cwd)
        .map_err(|refusal| Failure::launch(launch_status(refusal)))
}

/// 127 for a command not found, as on Unix. The CLI refuses a batch argument
/// `cmd.exe` cannot represent before starting the daemon, so here it is only
/// a command that cannot be executed.
fn launch_status(refusal: Refusal) -> u32 {
    match refusal {
        Refusal::NotFound => 127,
        Refusal::Unrepresentable => 126,
    }
}

/// A leader's status is recorded in full, as Windows reports it, and logged.
fn exited(code: u32) -> CommandResult {
    tracing::info!(status = format!("{code:#010x}"), "command exited");
    CommandResult::Exit { code }
}

fn own_folder() -> Result<PathBuf, Failure> {
    let executable =
        std::env::current_exe().map_err(|_| Failure::own("cannot locate daemon executable"))?;
    Ok(executable.with_file_name(""))
}

/// The terminal identity and the session ID, set in the daemon's own
/// environment before the command starts.
fn environment(options: &Options) -> Vec<(&'static str, &str)> {
    let mut environment: Vec<(&str, &str)> = TERMINAL_IDENTITY.to_vec();
    environment.push(("CLIMON2_SESSION_ID", options.id.as_str()));
    environment
}

/// Unix confirms the command started over a startup socket; on Windows
/// `CreateProcessW` reports it, so there is nothing to bind.
pub(crate) struct ExecListener;

impl ExecListener {
    pub fn bind(_options: &Options) -> Result<Self, Failure> {
        Ok(Self)
    }
}

/// Unix's exec helper; nothing starts the daemon this way on Windows.
pub fn exec_command(_arguments: &[OsString]) -> u8 {
    125
}
