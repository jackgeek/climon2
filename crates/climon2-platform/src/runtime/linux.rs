use std::{
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct SocketFreshness {
    socket: PathBuf,
    refreshed: Instant,
}

impl SocketFreshness {
    pub fn new(socket: &Path, now: Instant) -> Self {
        Self {
            socket: socket.to_path_buf(),
            refreshed: now,
        }
    }

    /// Conservatively refresh timestamps for age-based runtime cleanup policies.
    pub fn refresh(&mut self, now: Instant) -> io::Result<()> {
        if now.duration_since(self.refreshed) < Duration::from_secs(3600) {
            return Ok(());
        }
        refresh_path(&self.socket)?;
        let parent = self
            .socket
            .parent()
            .ok_or_else(|| io::Error::other("socket has no parent directory"))?;
        refresh_path(parent)?;
        self.refreshed = now;
        Ok(())
    }
}

fn refresh_path(path: &Path) -> io::Result<()> {
    let path =
        std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).map_err(io::Error::other)?;
    if unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            std::ptr::null(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
