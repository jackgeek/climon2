//! Private runtime directories and bounded AF_UNIX sockets (#18).
#[cfg(not(target_os = "macos"))]
compile_error!("runtime directory and socket seam requires macOS");

#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "macos")]
mod macos {
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
    };

    static UMASK: Mutex<()> = Mutex::new(());

    pub type SocketListener = UnixListener;
    pub type SocketStream = UnixStream;

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
