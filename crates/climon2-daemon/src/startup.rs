use crate::args::Options;
use climon2_core::metadata::{DisplayCommand, SessionMetadata};
use climon2_platform::{logging, process, runtime};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    num::NonZeroU32,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::PathBuf,
};

pub(crate) struct Failure {
    pub code: u32,
    pub message: &'static str,
}

impl Failure {
    pub fn own(message: &'static str) -> Self {
        Self { code: 125, message }
    }
    pub fn launch(code: u32) -> Self {
        let message = if code == 127 {
            "command not found"
        } else {
            "command cannot be executed"
        };
        Self { code, message }
    }
}

pub(crate) struct Resources {
    pub listener: runtime::SocketListener,
    metadata: PathBuf,
    socket: PathBuf,
    temporary: PathBuf,
    published: bool,
    temporary_owned: bool,
}

impl Resources {
    pub fn prepare(options: &Options) -> Result<Self, Failure> {
        runtime::private_directory(&options.home.join("sessions"))
            .map_err(|_| Failure::own("sessions directory is not private or usable"))?;
        let parent = options
            .socket
            .parent()
            .ok_or_else(|| Failure::own("invalid socket path"))?;
        runtime::private_directory(parent)
            .map_err(|_| Failure::own("socket directory is not private or usable"))?;
        let listener = runtime::bind(&options.socket)
            .map_err(|_| Failure::own("cannot bind session socket"))?;
        listener
            .set_nonblocking(true)
            .map_err(|_| Failure::own("cannot configure session socket"))?;
        let metadata = options
            .home
            .join("sessions")
            .join(format!("{}.pty", options.id));
        let temporary = metadata.with_extension("publishing");
        Ok(Self {
            listener,
            metadata,
            temporary,
            socket: options.socket.clone(),
            published: false,
            temporary_owned: false,
        })
    }

    pub fn publish(&mut self, options: &Options) -> Result<(), Failure> {
        self.write_metadata(options)
            .map_err(|_| Failure::own("cannot publish session metadata"))?;
        self.published = true;
        Ok(())
    }

    fn write_metadata(&mut self, options: &Options) -> io::Result<()> {
        let bytes = metadata(options)?.to_json().map_err(io::Error::other)?;
        let mut file = self.create_metadata_file()?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        self.rename()
    }

    fn rename(&mut self) -> io::Result<()> {
        if self.metadata.try_exists()? {
            return Err(super::io_error("session metadata already exists"));
        }
        fs::rename(&self.temporary, &self.metadata)?;
        self.temporary_owned = false;
        Ok(())
    }

    pub fn cleanup(&mut self) -> io::Result<()> {
        if self.published {
            remove(&self.metadata)?;
            self.published = false;
        }
        if self.temporary_owned {
            remove(&self.temporary)?;
            self.temporary_owned = false;
        }
        remove(&self.socket)
    }
    fn create_metadata_file(&mut self) -> io::Result<std::fs::File> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&self.temporary)?;
        self.temporary_owned = true;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        Ok(file)
    }
}

impl Drop for Resources {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn remove(path: &std::path::Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn metadata(options: &Options) -> io::Result<SessionMetadata> {
    let display = DisplayCommand::from_runtime(&options.command, &options.arguments, &options.cwd)
        .map_err(io::Error::other)?;
    Ok(SessionMetadata {
        format_version: NonZeroU32::MIN,
        session_id: options.id.clone(),
        socket_path: options
            .socket
            .to_string_lossy()
            .parse()
            .map_err(io::Error::other)?,
        daemon_pid: std::process::id(),
        daemon_start_time: process::start_time(std::process::id())?
            .ok_or_else(|| super::io_error("daemon process identity unavailable"))?,
        command: display.command,
        args: display.args,
        cwd: display.cwd,
        depth: std::env::var("CLIMON2_SESSION_DEPTH")
            .ok()
            .and_then(|depth| depth.parse().ok())
            .unwrap_or(NonZeroU32::MIN),
        created_at: logging::timestamp()?,
        climon2_version: options.version.clone(),
    })
}

pub fn fault(_log: &logging::LogFile) -> Result<(), Failure> {
    #[cfg(feature = "test-hooks")]
    if selected_fault("panic") {
        panic_fault(_log)?;
    }
    #[cfg(feature = "test-hooks")]
    if selected_fault("never-ready") {
        never_ready();
    }
    Ok(())
}

#[cfg(feature = "test-hooks")]
fn selected_fault(name: &str) -> bool {
    std::env::var("CLIMON2_DAEMON_FAULT").is_ok_and(|value| value == name)
}

#[cfg(feature = "test-hooks")]
fn panic_fault(log: &logging::LogFile) -> Result<(), Failure> {
    let _capture = log
        .capture_stderr()
        .map_err(|_| Failure::own("cannot capture daemon diagnostics"))?;
    panic!("controlled daemon fault");
}

#[cfg(feature = "test-hooks")]
fn never_ready() {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}
