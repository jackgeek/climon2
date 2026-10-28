use climon2_core::{
    SessionId, Timestamp,
    diagnostics::{CappedWriter, subscriber},
};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::fs::OpenOptionsExt,
    },
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};
use time::{OffsetDateTime, macros::format_description};

const RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

#[derive(Clone, Copy)]
pub enum Role {
    Cli,
    Daemon,
    Client,
}

impl Role {
    fn name(self) -> &'static str {
        match self {
            Self::Cli => "cli",
            Self::Daemon => "daemon",
            Self::Client => "client",
        }
    }
}

pub struct LogFile {
    path: PathBuf,
    writer: CappedWriter<File>,
}

pub fn timestamp() -> io::Result<Timestamp> {
    let text = OffsetDateTime::now_utc()
        .format(format_description!(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
        ))
        .map_err(io::Error::other)?;
    text.parse().map_err(io::Error::other)
}

impl LogFile {
    fn open(directory: &Path, name: &str, role: &str) -> io::Result<Self> {
        crate::runtime::private_directory(directory)?;
        let path = directory.join(name);
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        let mut writer = CappedWriter::new(file);
        writeln!(
            writer,
            "{} INFO climon2: role={role} pid={} version={}",
            timestamp()?,
            std::process::id(),
            env!("CARGO_PKG_VERSION")
        )?;
        Ok(Self { path, writer })
    }

    pub fn general(home: &Path) -> io::Result<Self> {
        crate::runtime::private_directory(&home.join("logs"))?;
        let stamp = OffsetDateTime::now_utc()
            .format(format_description!(
                "[year][month][day]T[hour][minute][second]Z"
            ))
            .map_err(io::Error::other)?;
        Self::open(
            &home.join("logs/general"),
            &format!("{stamp}-{}.log", std::process::id()),
            "cli",
        )
    }

    pub fn session(
        home: &Path,
        id: &SessionId,
        role: Role,
        cli_pid: Option<u32>,
        previous: Option<&Path>,
    ) -> io::Result<Self> {
        crate::runtime::private_directory(&home.join("logs"))?;
        let mut log = Self::open(
            &home.join("logs").join(id.as_str()),
            &format!("{}.log", role.name()),
            role.name(),
        )?;
        writeln!(log.writer, "session_id={id}")?;
        log.links(cli_pid, previous)?;
        Ok(log)
    }

    fn links(&mut self, cli_pid: Option<u32>, previous: Option<&Path>) -> io::Result<()> {
        if let Some(pid) = cli_pid {
            writeln!(self.writer, "cli_pid={pid}")?;
        }
        if let Some(path) = previous {
            writeln!(self.writer, "general_log={:?}", path.as_os_str())?;
        }
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn writer(&self) -> CappedWriter<File> {
        self.writer.clone()
    }

    pub fn dispatch(&self, filter: Option<&str>) -> io::Result<tracing::Dispatch> {
        subscriber(self.writer(), filter)
    }

    pub fn capture_stderr(&self) -> io::Result<StderrCapture> {
        let (reader, writer) = crate::daemon::pipe()?;
        // The saved descriptor is owned here and is never inherited across exec.
        let saved =
            owned_fd(unsafe { libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD_CLOEXEC, 3) })?;
        replace_stderr(writer.as_raw_fd())?;
        drop(writer);
        let mut sink = self.writer();
        let reader = std::thread::spawn(move || {
            let mut reader = reader;
            io::copy(&mut reader, &mut sink)?;
            sink.flush()
        });
        Ok(StderrCapture {
            saved,
            reader: Some(reader),
        })
    }
}

pub struct StderrCapture {
    saved: OwnedFd,
    reader: Option<std::thread::JoinHandle<io::Result<()>>>,
}

fn owned_fd(fd: libc::c_int) -> io::Result<OwnedFd> {
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // Every caller transfers a newly allocated descriptor.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn replace_stderr(fd: libc::c_int) -> io::Result<()> {
    if unsafe { libc::dup2(fd, libc::STDERR_FILENO) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Drop for StderrCapture {
    fn drop(&mut self) {
        if let Some(message) = self.restore() {
            eprintln!("climon2: error: {message}");
        }
    }
}

impl StderrCapture {
    fn restore(&mut self) -> Option<String> {
        if let Err(error) = replace_stderr(self.saved.as_raw_fd()) {
            return Some(format!("cannot restore daemon stderr: {error}"));
        }
        reader_failure(self.reader.take().expect("stderr reader").join())
    }
}

fn reader_failure(joined: std::thread::Result<io::Result<()>>) -> Option<String> {
    match joined {
        Ok(written) => write_failure(written),
        Err(_) => Some("daemon log reader panicked".to_owned()),
    }
}

fn write_failure(written: io::Result<()>) -> Option<String> {
    if let Err(error) = written {
        return Some(format!("daemon log write failed: {error}"));
    }
    None
}

fn old(path: &Path, now: SystemTime) -> io::Result<bool> {
    let modified = fs::symlink_metadata(path)?.modified()?;
    Ok(now
        .duration_since(modified)
        .is_ok_and(|age| age > RETENTION))
}

fn old_session(directory: &Path, now: SystemTime) -> io::Result<bool> {
    let entries = directory_entries(directory)?;
    if entries.is_empty() {
        return old(directory, now);
    }
    all_old_files(&entries, now)
}

fn all_old_files(entries: &[fs::DirEntry], now: SystemTime) -> io::Result<bool> {
    for entry in entries {
        if !old_file(entry, now)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn directory_entries(directory: &Path) -> io::Result<Vec<fs::DirEntry>> {
    fs::read_dir(directory)?.collect()
}

fn old_file(entry: &fs::DirEntry, now: SystemTime) -> io::Result<bool> {
    Ok(entry.file_type()?.is_file() && old(&entry.path(), now)?)
}

fn prune_general(directory: &Path, now: SystemTime) -> io::Result<()> {
    for entry in directory_entries(directory)? {
        prune_general_file(&entry, now)?;
    }
    Ok(())
}

fn prune_general_file(entry: &fs::DirEntry, now: SystemTime) -> io::Result<()> {
    if old_file(entry, now)? {
        fs::remove_file(entry.path())?;
    }
    Ok(())
}

fn prune_session(home: &Path, path: &Path, id: &SessionId, now: SystemTime) -> io::Result<()> {
    let metadata = home.join("sessions").join(format!("{id}.pty"));
    if metadata_present(&metadata)? {
        return Ok(());
    }
    if !old_session(path, now)? {
        return Ok(());
    }
    remove_session_logs(path)
}

fn metadata_present(path: &Path) -> io::Result<bool> {
    Ok(existing_metadata(path)?.is_some())
}

fn existing_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn remove_session_logs(path: &Path) -> io::Result<()> {
    for entry in directory_entries(path)? {
        fs::remove_file(entry.path())?;
    }
    fs::remove_dir(path)
}

fn prune_directory(home: &Path, path: &Path, now: SystemTime) -> io::Result<()> {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    if name == "general" {
        return prune_general(path, now);
    }
    if let Ok(id) = name.parse::<SessionId>() {
        prune_session(home, path, &id, now)?;
    }
    Ok(())
}

fn log_root(home: &Path) -> io::Result<Option<PathBuf>> {
    let root = home.join("logs");
    let Some(metadata) = existing_metadata(&root)? else {
        return Ok(None);
    };
    if !metadata.file_type().is_dir() {
        return Err(io::Error::other("logs is not a real directory"));
    }
    Ok(Some(root))
}

fn prune_entry(home: &Path, entry: &fs::DirEntry, now: SystemTime) -> io::Result<()> {
    if entry.file_type()?.is_dir() {
        prune_directory(home, &entry.path(), now)?;
    }
    Ok(())
}

fn prune_logs(home: &Path, now: SystemTime) -> io::Result<()> {
    let Some(root) = log_root(home)? else {
        return Ok(());
    };
    prune_root(home, &root, now)
}

fn prune_root(home: &Path, root: &Path, now: SystemTime) -> io::Result<()> {
    for entry in directory_entries(root)? {
        prune_entry(home, &entry, now)?;
    }
    Ok(())
}

pub fn prune(home: &Path, now: SystemTime) {
    if let Err(error) = prune_logs(home, now) {
        tracing::warn!(errno = error.raw_os_error(), "log pruning failed");
    }
}
