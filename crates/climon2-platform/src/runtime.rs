//! Private runtime directories and bounded AF_UNIX sockets (#18).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("runtime directory and socket seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use climon2_core::{SessionId, layout::RuntimeLayout};
    use std::{
        fs::{self, DirBuilder},
        io,
        os::unix::{
            fs::{DirBuilderExt, MetadataExt},
            net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
        sync::Mutex,
        time::{Duration, Instant},
    };

    static UMASK: Mutex<()> = Mutex::new(());

    pub type SocketListener = UnixListener;
    pub type SocketStream = UnixStream;

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

        /// Linux tmpfiles removes old sockets as well as directories, even while bound.
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

    #[cfg(target_os = "linux")]
    fn refresh_path(path: &Path) -> io::Result<()> {
        let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
            .map_err(io::Error::other)?;
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

    #[cfg(target_os = "macos")]
    fn refresh_path(_path: &Path) -> io::Result<()> {
        Ok(())
    }

    pub fn canonical_home(path: &Path) -> io::Result<PathBuf> {
        if !path.is_absolute() {
            return Err(io::Error::other("climon2 home must be absolute"));
        }
        fs::create_dir_all(path)?;
        fs::canonicalize(path)
    }

    pub struct RuntimeDirectory(RuntimeLayout);

    impl RuntimeDirectory {
        /// `home` is the result of the invocation's single `canonical_home` call.
        pub fn prepare(home: &Path) -> io::Result<Self> {
            let layout = RuntimeLayout::new(Path::new("/tmp"), crate::peer::effective_uid(), home)
                .map_err(io::Error::other)?;
            private_directory(layout.directory())?;
            Ok(Self(layout))
        }

        pub fn path(&self) -> &Path {
            self.0.directory()
        }

        pub fn socket_path(&self, id: &SessionId) -> io::Result<PathBuf> {
            self.0.socket_path(id, 103).map_err(io::Error::other)
        }
    }

    pub fn private_directory(path: &Path) -> io::Result<()> {
        create_directory(path)?;
        check_directory(path)
    }

    fn create_directory(path: &Path) -> io::Result<()> {
        match DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        Ok(())
    }

    fn check_directory(path: &Path) -> io::Result<()> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() {
            return Err(unsafe_directory(path, "not a real directory"));
        }
        if metadata.uid() != crate::peer::effective_uid() {
            return Err(unsafe_directory(path, "wrong owner"));
        }
        if metadata.mode() & 0o077 != 0 {
            return Err(unsafe_directory(path, "group or other permissions"));
        }
        Ok(())
    }

    fn unsafe_directory(path: &Path, problem: &str) -> io::Error {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("unsafe runtime directory {}: {problem}", path.display()),
        )
    }

    pub fn bind(path: &Path) -> io::Result<SocketListener> {
        if path.as_os_str().as_encoded_bytes().len() > 103 {
            return Err(io::Error::other("socket path exceeds 103 bytes"));
        }
        let _lock = UMASK
            .lock()
            .map_err(|_| io::Error::other("socket creation lock poisoned"))?;
        // umask is process-wide. The daemon binds before creating worker threads.
        let mask = Mask(unsafe { libc::umask(0o077) });
        let result = UnixListener::bind(path);
        drop(mask);
        result
    }

    struct Mask(libc::mode_t);

    impl Drop for Mask {
        fn drop(&mut self) {
            unsafe { libc::umask(self.0) };
        }
    }
}
