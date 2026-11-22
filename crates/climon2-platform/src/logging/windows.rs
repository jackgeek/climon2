//! Capturing the daemon's stderr into its log on Windows (#317). The daemon
//! starts with no standard handles, so a panic message would be lost; while
//! captured, the process's standard error handle is a pipe whose reader copies
//! into the log. Rust looks that handle up on every write.
use super::LogFile;
use std::{
    io::{self, PipeWriter, Write},
    os::windows::io::AsRawHandle,
    thread::JoinHandle,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::Console::{GetStdHandle, STD_ERROR_HANDLE, SetStdHandle},
};

impl LogFile {
    pub fn capture_stderr(&self) -> io::Result<StderrCapture> {
        let (mut reader, writer) = io::pipe()?;
        // Reading a standard handle has no preconditions.
        let saved = unsafe { GetStdHandle(STD_ERROR_HANDLE) };
        replace_stderr(writer.as_raw_handle())?;
        let mut sink = self.writer();
        let reader = std::thread::spawn(move || {
            io::copy(&mut reader, &mut sink)?;
            sink.flush()
        });
        Ok(StderrCapture {
            saved,
            writer: Some(writer),
            reader: Some(reader),
        })
    }
}

/// While it lives, standard error goes to the log; dropping it restores the
/// handle the process had and waits until everything written is copied.
pub struct StderrCapture {
    saved: HANDLE,
    writer: Option<PipeWriter>,
    reader: Option<JoinHandle<io::Result<()>>>,
}

fn replace_stderr(handle: HANDLE) -> io::Result<()> {
    // Setting a standard handle only records the value; the caller keeps it
    // open for as long as it is the standard handle.
    if unsafe { SetStdHandle(STD_ERROR_HANDLE, handle) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

impl Drop for StderrCapture {
    fn drop(&mut self) {
        // With no standard error to report to, a failure here is dropped.
        let _ = replace_stderr(self.saved);
        self.writer.take();
        let _ = self.reader.take().map(JoinHandle::join);
    }
}
