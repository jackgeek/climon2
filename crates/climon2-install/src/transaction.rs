use crate::Payload;
use crate::payload::COMPANIONS;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// The reserved staging directory is the only recovery/cleanup scope.
pub(crate) struct Stage {
    root: PathBuf,
    _lock: File,
}

impl Stage {
    pub(crate) fn new(home: &Path) -> io::Result<Self> {
        let lock = super::platform::lock(home)?;
        let root = home.join(".climon2-install-stage");
        remove_stage(&root)?;
        fs::create_dir(&root)?;
        Ok(Self { root, _lock: lock })
    }

    pub(crate) fn copy_payload(&self, payload: &Payload) -> io::Result<()> {
        fs::create_dir(self.version())?;
        self.copy_companions(payload)?;
        fs::copy(&payload.launcher, self.launcher())?;
        fs::write(self.current(), format!("{}\n", payload.version()))?;
        Ok(())
    }

    fn copy_companions(&self, payload: &Payload) -> io::Result<()> {
        for (source, name) in payload.companions.iter().zip(COMPANIONS) {
            fs::copy(source, self.version().join(name))?;
            #[cfg(feature = "test-hooks")]
            if name == "climon2-cli" {
                crate::hooks::checkpoint("copy-cli")?;
            }
        }
        Ok(())
    }

    pub(crate) fn version(&self) -> PathBuf {
        self.root.join("version")
    }

    pub(crate) fn launcher(&self) -> PathBuf {
        self.root.join("climon2")
    }

    pub(crate) fn current(&self) -> PathBuf {
        self.root.join("current")
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn remove_stage(root: &Path) -> io::Result<()> {
    match fs::remove_dir_all(root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}
