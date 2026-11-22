//! Capturing the daemon's stderr into its log, which needs Unix descriptors.
use super::LogFile;
use std::{
    io::{self, Write},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
};

impl LogFile {
    pub fn capture_stderr(&self) -> io::Result<StderrCapture> {
        let (reader, writer) = crate::daemon::pipe()?;
        let saved = saved_stderr()?;
        replace_stderr(writer.as_raw_fd())?;
        drop(writer);
        let mut sink = self.writer();
        let reader = std::thread::spawn(move || {
            let mut reader = reader;
            io::copy(&mut reader, &mut sink)?;
            sink.flush()
        });
        Ok(StderrCapture {
            saved,
            reader: Some(reader),
        })
    }
}

pub struct StderrCapture {
    saved: OwnedFd,
    reader: Option<std::thread::JoinHandle<io::Result<()>>>,
}

/// A duplicate of stderr, numbered above the standard streams, that is owned
/// here and never inherited across exec.
fn saved_stderr() -> io::Result<OwnedFd> {
    let fd = crate::syscall::checked(unsafe {
        libc::fcntl(libc::STDERR_FILENO, libc::F_DUPFD_CLOEXEC, 3)
    })?;
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

fn replace_stderr(fd: libc::c_int) -> io::Result<()> {
    crate::syscall::checked(unsafe { libc::dup2(fd, libc::STDERR_FILENO) }).map(drop)
}

impl Drop for StderrCapture {
    fn drop(&mut self) {
        if let Some(message) = self.restore() {
            eprintln!("climon2: error: {message}");
        }
    }
}

impl StderrCapture {
    fn restore(&mut self) -> Option<String> {
        if let Err(error) = replace_stderr(self.saved.as_raw_fd()) {
            return Some(format!("cannot restore daemon stderr: {error}"));
        }
        reader_failure(self.reader.take().expect("stderr reader").join())
    }
}

fn reader_failure(joined: std::thread::Result<io::Result<()>>) -> Option<String> {
    match joined {
        Ok(written) => write_failure(written),
        Err(_) => Some("daemon log reader panicked".to_owned()),
    }
}

fn write_failure(written: io::Result<()>) -> Option<String> {
    if let Err(error) = written {
        return Some(format!("daemon log write failed: {error}"));
    }
    None
}
