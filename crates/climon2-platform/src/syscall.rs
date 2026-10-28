use std::io;

/// One EINTR policy for blocking system calls: an interrupted call is
/// retried by the caller; every other error is final.
pub(crate) fn retry_if_interrupted(error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::Interrupted {
        return Ok(());
    }
    Err(error)
}
