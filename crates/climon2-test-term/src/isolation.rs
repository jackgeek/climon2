use portable_pty::CommandBuilder;
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
use tempfile::TempDir;

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

pub fn bin_path(name: &str) -> PathBuf {
    let executable = env::current_exe().expect("locate test executable");
    let path = executable
        .parent()
        .expect("deps directory")
        .parent()
        .expect("binary directory")
        .join(name);
    assert!(
        path.is_file(),
        "missing workspace binary {}; run cargo test --workspace (no nested builds)",
        path.display()
    );
    path
}

pub struct Isolation {
    directory: TempDir,
    id: String,
}

impl Default for Isolation {
    fn default() -> Self {
        Self::new()
    }
}

impl Isolation {
    pub fn new() -> Self {
        let directory = tempfile::tempdir().expect("isolated HOME");
        fs::create_dir(directory.path().join("climon2")).expect("isolated climon2 home");
        let id = format!(
            "climon2-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        );
        Self { directory, id }
    }

    pub fn home(&self) -> &Path {
        self.directory.path()
    }

    pub fn climon2_home(&self) -> PathBuf {
        self.home().join("climon2")
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn command(&self, executable: impl AsRef<std::ffi::OsStr>) -> CommandBuilder {
        let mut command = CommandBuilder::new(executable);
        command.env_clear();
        command.cwd(self.home());
        command.env("HOME", self.home());
        command.env("CLIMON2_HOME", self.climon2_home());
        command.env("CLIMON2_TEST_ID", self.id());
        command.env(
            "PATH",
            format!(
                "{}:/usr/bin:/bin",
                bin_path("climon2-fixture").parent().unwrap().display()
            ),
        );
        command.env("TERM", "xterm-256color");
        command.env("LANG", "en_US.UTF-8");
        command
    }

    pub fn assert_clean(&self) {
        let output = Command::new("/bin/ps")
            .args(["-axEww", "-o", "pid=,stat=,command="])
            .output()
            .expect("scan process environment");
        assert!(output.status.success(), "ps failed: {:?}", output);
        let marker = format!("CLIMON2_TEST_ID={}", self.id);
        let processes = String::from_utf8_lossy(&output.stdout);
        let leaks: Vec<_> = processes
            .lines()
            .filter(|line| {
                let mut fields = line.split_whitespace();
                fields.next();
                let zombie = fields.next().is_some_and(|state| state.starts_with('Z'));
                !zombie && line.split_whitespace().any(|field| field == marker)
            })
            .collect();
        assert!(leaks.is_empty(), "live test processes: {leaks:?}");
        let sessions = self.climon2_home().join("sessions");
        if sessions.exists() {
            for entry in fs::read_dir(sessions).expect("scan session metadata") {
                let path = entry.expect("session entry").path();
                assert!(
                    path.extension().is_none_or(|ext| ext != "pty"),
                    "leaked metadata: {}",
                    path.display()
                );
            }
        }
        let canonical = self.climon2_home().canonicalize().expect("canonical home");
        let digest = format!(
            "{:x}",
            Sha256::digest(canonical.as_os_str().as_encoded_bytes())
        );
        let runtime = PathBuf::from(format!(
            "/tmp/climon2-{}-{}",
            nix::unistd::getuid(),
            &digest[..8]
        ));
        if runtime.exists() {
            use std::os::unix::fs::FileTypeExt;
            for entry in fs::read_dir(runtime).expect("scan runtime directory") {
                let entry = entry.expect("runtime entry");
                assert!(
                    !entry.file_type().expect("runtime type").is_socket(),
                    "leaked socket: {}",
                    entry.path().display()
                );
            }
        }
    }
}
