//! Private runtime directories and bounded AF_UNIX sockets (#18).
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("runtime directory and socket seam requires macOS or Linux");

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use unix::*;

#[cfg(target_os = "linux")]
#[path = "runtime/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::SocketFreshness;

#[cfg(target_os = "macos")]
#[path = "runtime/macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::SocketFreshness;

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use climon2_core::{SessionId, layout::RuntimeLayout};
    use std::{
        fs, io,
        os::unix::{
            fs::FileTypeExt,
            net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
        sync::Mutex,
    };

    static UMASK: Mutex<()> = Mutex::new(());

    pub type SocketListener = UnixListener;
    pub type SocketStream = UnixStream;

    pub fn validate_stale_socket(path: &Path) -> io::Result<()> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => require_socket(&metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    fn require_socket(metadata: &fs::Metadata) -> io::Result<()> {
        if !metadata.file_type().is_socket() {
            return Err(io::Error::other("stale socket path is not a socket"));
        }
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
            crate::private::private_directory(layout.directory())?;
            Ok(Self(layout))
        }

        pub fn path(&self) -> &Path {
            self.0.directory()
        }

        pub fn socket_path(&self, id: &SessionId) -> io::Result<PathBuf> {
            self.0.socket_path(id, 103).map_err(io::Error::other)
        }
    }

    /// The longest path, in bytes with its NUL, the system accepts.
    pub const PATH_MAX: usize = libc::PATH_MAX as usize;

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
