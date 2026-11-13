use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
/// Every executable a release places in its version folder.
pub const COMPANIONS: [&str; 4] = [
    "climon2-cli",
    "climon2-daemon",
    "climon2-client",
    "climon2-monitor",
];
// Linux exec refuses inodes whose writable descriptors another test's fork
// inherited. These fixtures share one process descriptor table, not a filesystem.
#[cfg(target_os = "linux")]
static EXECUTABLE_FIXTURES: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub struct Workspace {
    pub root: PathBuf,
    #[cfg(target_os = "linux")]
    _ownership: std::sync::MutexGuard<'static, ()>,
}

impl Workspace {
    pub fn new() -> Self {
        #[cfg(target_os = "linux")]
        let ownership = EXECUTABLE_FIXTURES.lock().unwrap();
        let root = std::env::current_dir().unwrap().join(format!(
            "target/install tests 雪 {}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&root).unwrap();
        Self {
            root,
            #[cfg(target_os = "linux")]
            _ownership: ownership,
        }
    }

    pub fn payload(&self, name: &str, version: &str, status: u8) -> PathBuf {
        let root = self.root.join(name);
        fs::create_dir_all(root.join(version)).unwrap();
        fs::copy(env!("CARGO_BIN_EXE_climon2-install"), root.join("install")).unwrap();
        for binary in COMPANIONS {
            script(&root.join(version).join(binary), &format!("exit {status}"));
        }
        script(&root.join("climon2"), "exit 11");
        root
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.root).unwrap();
    }
}

pub fn script(path: &Path, body: &str) {
    fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
