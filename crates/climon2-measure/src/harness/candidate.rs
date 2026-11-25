//! The exact candidate archive, installed by its own installer, and the
//! isolated users whose sessions and clients are measured.

use super::{bounded, reaped};
use climon2_core::{VersionName, layout::ExecutableHome, metadata::SessionMetadata};
use portable_pty::CommandBuilder;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

/// The host platform's archive suffix; an archive of another platform
/// cannot stand in for this one.
#[cfg(target_os = "linux")]
const SUFFIX: &str = "-linux-x86_64-gnu";
#[cfg(target_os = "macos")]
const SUFFIX: &str = "-macos-universal";

/// A bound for one CLI call.
const CLI: Duration = Duration::from_secs(20);

/// The candidate release, installed from its archive.
pub struct Candidate {
    _directory: tempfile::TempDir,
    pub home: ExecutableHome,
    pub version: VersionName,
    pub archive: String,
    pub sha256: String,
}

impl Candidate {
    /// Extracts `archive` and runs that archive's own `install --dir`.
    pub fn install(archive: &Path) -> io::Result<Self> {
        let bytes = fs::read(archive)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let directory = tempfile::Builder::new().prefix("c2m-").tempdir()?;
        let extracted = Command::new("tar")
            .arg("-xzf")
            .arg(archive)
            .arg("-C")
            .arg(directory.path())
            .output()?;
        check(
            &extracted.status.success(),
            "extract the archive",
            &extracted.stderr,
        )?;
        let root = fs::read_dir(directory.path())?
            .next()
            .ok_or_else(|| io::Error::other("the archive is empty"))??
            .file_name();
        let version = root
            .to_str()
            .and_then(|root| root.strip_prefix("climon2-")?.strip_suffix(SUFFIX))
            .ok_or_else(|| io::Error::other(format!("not a {SUFFIX} release archive: {root:?}")))?
            .parse::<VersionName>()
            .map_err(io::Error::other)?;
        let payload = directory.path().join(&root);
        let home = directory.path().canonicalize()?.join("eh");
        let installed = Command::new(payload.join("install"))
            .env_clear()
            .env("HOME", &payload)
            .env("PATH", "/usr/bin:/bin")
            .arg("--dir")
            .arg(&home)
            .output()?;
        check(
            &installed.status.success(),
            "run the archive's installer",
            &installed.stderr,
        )?;
        Ok(Self {
            home: ExecutableHome::new(&home).map_err(io::Error::other)?,
            version,
            archive: archive
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            sha256,
            _directory: directory,
        })
    }
}

fn check(succeeded: &bool, what: &str, stderr: &[u8]) -> io::Result<()> {
    if *succeeded {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "could not {what}: {}",
        String::from_utf8_lossy(stderr)
    )))
}

/// One created session.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub daemon: i32,
    pub fifo: Option<PathBuf>,
}

/// An isolated user: a HOME and climon2 home of its own, the key
/// introduction already dismissed, and its monitor configuration.
pub struct User {
    directory: tempfile::TempDir,
    executables: PathBuf,
}

impl User {
    /// A new user of `candidate`, with `limits` as its `monitor.limits`, or
    /// the defaults.
    pub fn new(candidate: &Candidate, limits: Option<&Value>) -> io::Result<Self> {
        // Short, so every Unix socket path fits.
        let directory = tempfile::Builder::new().prefix("c2u-").tempdir_in("/tmp")?;
        let user = Self {
            directory,
            executables: candidate.home.path().to_owned(),
        };
        fs::create_dir(user.climon2_home())?;
        fs::write(
            user.climon2_home().join("monitor-preferences.json"),
            "{\"version\":1,\"suppress_key_introduction\":true}\n",
        )?;
        if let Some(limits) = limits {
            fs::write(
                user.climon2_home().join("config.jsonc"),
                json!({ "monitor": { "limits": limits } }).to_string(),
            )?;
        }
        Ok(user)
    }

    pub fn home(&self) -> &Path {
        self.directory.path()
    }

    pub fn climon2_home(&self) -> PathBuf {
        self.home().join("climon2")
    }

    fn environment(&self) -> [(&'static str, String); 6] {
        [
            ("HOME", self.home().display().to_string()),
            ("CLIMON2_HOME", self.climon2_home().display().to_string()),
            (
                "PATH",
                format!("{}:/usr/bin:/bin", self.executables.display()),
            ),
            ("TERM", "xterm-256color".to_owned()),
            ("LANG", "en_US.UTF-8".to_owned()),
            ("USER", std::env::var("USER").unwrap_or_default()),
        ]
    }

    /// The installed `c2` with `args`, for a terminal.
    pub fn terminal_command(&self, args: &[&str]) -> CommandBuilder {
        let mut command = CommandBuilder::new(self.executables.join("c2"));
        command.env_clear();
        command.cwd(self.home());
        for (name, value) in self.environment() {
            command.env(name, value);
        }
        command.env("SHELL", climon2_test_term::harness_shell());
        command.args(args);
        command
    }

    /// The installed `c2` with `args`, as a script runs it.
    fn script(&self, args: &[&OsStr]) -> Command {
        let mut command = Command::new(self.executables.join("c2"));
        command
            .args(args)
            .env_clear()
            .envs(self.environment())
            .current_dir(self.home())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// The installed `c2` with `args`, as a script runs it; its
    /// standard output once it exits successfully within its bound.
    pub fn cli(&self, args: &[&OsStr]) -> io::Result<String> {
        let mut command = self.script(args);
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
        let output = bounded(command, CLI)?;
        check(
            &output.status.success(),
            &format!("run c2 {args:?}"),
            &output.stderr,
        )?;
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }

    /// A headless session of `measure`'s workload titled `title`, writing
    /// `rate` bytes a second, with an attention FIFO if asked for.
    pub fn start(
        &self,
        measure: &Path,
        title: &str,
        rate: u32,
        attention: bool,
        heuristic: bool,
    ) -> io::Result<Session> {
        let fifo = attention.then(|| self.home().join(format!("{title}.fifo")));
        if let Some(fifo) = &fifo {
            let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes())?;
            // SAFETY: `path` is a valid C string.
            if unsafe { libc::mkfifo(path.as_ptr(), 0o600) } != 0 {
                return Err(io::Error::last_os_error());
            }
        }
        let rate = rate.to_string();
        let mut args: Vec<&OsStr> = ["exec", "--headless"].map(OsStr::new).to_vec();
        args.push(measure.as_os_str());
        args.extend(["workload", "--title", title, "--rate", &rate].map(OsStr::new));
        if let Some(fifo) = &fifo {
            args.extend([OsStr::new("--attention"), fifo.as_os_str()]);
        }
        if heuristic {
            args.push(OsStr::new("--heuristic"));
        }
        let id = self.cli(&args)?;
        let metadata = self
            .climon2_home()
            .join("sessions")
            .join(format!("{id}.pty"));
        let parsed = SessionMetadata::from_json(&metadata, &fs::read(&metadata)?)
            .map_err(io::Error::other)?;
        Ok(Session {
            id,
            title: title.to_owned(),
            daemon: i32::try_from(parsed.daemon_pid).map_err(io::Error::other)?,
            fifo,
        })
    }

    /// Ends every session it started, by `c2 kill`, and any daemon
    /// that outlives that.
    /// Every `c2 kill` runs at once: each waits out the session's
    /// hang-up grace.
    pub fn end(&self, sessions: &[Session]) {
        let kills: Vec<_> = sessions
            .iter()
            .map(|session| {
                // SAFETY: SIGCONT to a pid this user's session owns.
                unsafe { libc::kill(session.daemon, libc::SIGCONT) };
                self.script(&[OsStr::new("kill"), OsStr::new(&session.id)])
                    .spawn()
            })
            .collect();
        let deadline = Instant::now() + CLI;
        for (session, kill) in sessions.iter().zip(kills) {
            if !kill.is_ok_and(|child| reaped(child, deadline)) {
                // SAFETY: as above.
                unsafe { libc::kill(session.daemon, libc::SIGKILL) };
            }
        }
    }
}
