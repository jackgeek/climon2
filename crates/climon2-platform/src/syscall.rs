use std::io;

/// One EINTR policy for blocking system calls: an interrupted call is
/// retried by the caller; every other error is final.
pub(crate) fn retry_if_interrupted(error: io::Error) -> io::Result<()> {
    if error.kind() == io::ErrorKind::Interrupted {
        return Ok(());
    }
    Err(error)
}

/// One failure test for C calls that report failure as -1 with errno set.
pub(crate) fn checked(result: libc::c_int) -> io::Result<libc::c_int> {
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(result)
}
