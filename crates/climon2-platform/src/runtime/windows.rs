//! The runtime directory and AF_UNIX sockets on Windows (#316; ADR 0003,
//! "Windows I/O and transport"). The directory is
//! `%LOCALAPPDATA%\c2\<hash8>`, prepared with the private descriptor; the
//! socket file inherits its DACL. Sockets go through socket2. A socket file
//! is an NTFS reparse point tagged `IO_REPARSE_TAG_AF_UNIX`, which std reports
//! as a regular file, so a stale socket is recognised by that tag.
use socket2::{Domain, SockAddr, Socket, Type};
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        fs::OpenOptionsExt,
        io::{AsRawHandle, AsSocket, BorrowedSocket},
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Foundation::FALSE,
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FileAttributeTagInfo,
        GetFileInformationByHandleEx,
    },
    System::SystemServices::IO_REPARSE_TAG_AF_UNIX,
};

pub type SocketStream = Socket;

/// A connected socket2 socket is already this platform's stream.
pub(crate) fn into_stream(socket: Socket) -> SocketStream {
    socket
}

/// A bound, listening AF_UNIX socket.
pub struct SocketListener(Socket);

impl SocketListener {
    /// The next connection, as std's `UnixListener::accept` gives it.
    pub fn accept(&self) -> io::Result<(SocketStream, SockAddr)> {
        self.0.accept()
    }
}

impl AsSocket for SocketListener {
    fn as_socket(&self) -> BorrowedSocket<'_> {
        self.0.as_socket()
    }
}

/// `sun_path` holds 108 bytes of UTF-8 with its NUL.
pub(super) const SOCKET_PATH_MAX: usize = 107;

/// `%LOCALAPPDATA%\c2`, created when missing and not checked: only the
/// directory in it is private. Windows has no numeric uid to name it by.
pub(super) fn runtime_root() -> io::Result<(PathBuf, Option<u32>)> {
    let base = PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default());
    if !base.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LOCALAPPDATA must name an absolute directory",
        ));
    }
    let root = base.join("c2");
    fs::create_dir_all(&root)?;
    Ok((root, None))
}

pub fn bind(path: &Path) -> io::Result<SocketListener> {
    super::within_budget(path)?;
    listening(&SockAddr::unix(path)?).map(SocketListener)
}

fn listening(address: &SockAddr) -> io::Result<Socket> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    socket.bind(address)?;
    // SOMAXCONN in winsock2.h: the provider's reasonable maximum.
    socket.listen(i32::MAX)?;
    Ok(socket)
}

/// A missing path is no socket to remove; anything there but an AF_UNIX
/// socket file is refused.
pub fn validate_stale_socket(path: &Path) -> io::Result<()> {
    match open_itself(path) {
        Ok(file) => require_socket(&file),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// The name itself, never a reparse point's target.
fn open_itself(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

fn require_socket(file: &File) -> io::Result<()> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    // The handle is open and `info` is writable for its whole size.
    let read = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileAttributeTagInfo,
            (&raw mut info).cast(),
            size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
        )
    };
    if read == FALSE {
        return Err(io::Error::last_os_error());
    }
    // The tag means nothing unless the file is a reparse point.
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT == 0
        || info.ReparseTag != IO_REPARSE_TAG_AF_UNIX
    {
        return Err(io::Error::other("stale socket path is not a socket"));
    }
    Ok(())
}
