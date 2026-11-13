pub mod stand_in;

use std::{
    fs::{self, File},
    io::{self, Read},
    os::fd::AsRawFd,
    path::Path,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// The longest a fixture waits, for its test or for itself: longer than any
/// test waits on a fixture under load, and short enough that a fixture left by
/// a killed test ends soon.
pub const BOUND: Duration = Duration::from_secs(60);

/// Runs `command` with its output in `log`, waiting at most `BOUND`; a
/// command still running then is killed and reaped. A failure carries the
/// output.
pub fn run(command: &mut Command, log: &Path) -> io::Result<()> {
    let output = File::create(log)?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(output)
        .spawn()?;
    let deadline = Instant::now() + BOUND;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            child.wait()?;
            return Err(io::Error::other(format!("{command:?} outlived {BOUND:?}")));
        }
        thread::sleep(Duration::from_millis(10));
    };
    if !status.success() {
        let output = fs::read_to_string(log)?;
        return Err(io::Error::other(format!("{command:?}: {status}\n{output}")));
    }
    Ok(())
}

pub fn size() -> io::Result<[u16; 2]> {
    let mut size = nix::libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // TIOCGWINSZ writes exactly one winsize to the supplied pointer.
    if unsafe { nix::libc::ioctl(io::stdin().as_raw_fd(), nix::libc::TIOCGWINSZ, &mut size) } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok([size.ws_row, size.ws_col])
}

pub fn report(args: &[String]) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    Ok(serde_json::json!({
        "argv": args,
        "cwd": std::env::current_dir()?,
        "env": std::env::vars().collect::<std::collections::BTreeMap<_,_>>(),
        "size": size()?,
        "pid": nix::unistd::getpid().as_raw(),
        "pgid": nix::unistd::getpgrp().as_raw(),
        "isatty": [nix::unistd::isatty(io::stdin())?, nix::unistd::isatty(io::stdout())?, nix::unistd::isatty(io::stderr())?],
    }))
}

/// Reads input in raw mode (see [`RawTerminal`]) until `done` holds for all of
/// it so far, asking before the first read and after every read, for at most
/// [`BOUND`].
pub fn read_until(mut done: impl FnMut(&[u8]) -> io::Result<bool>) -> io::Result<Vec<u8>> {
    let deadline = Instant::now() + BOUND;
    let mut input = Vec::new();
    while !done(&input)? {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "input deadline exceeded",
            ));
        }
        // Raw mode returns from a read after at most 0.1 s without input.
        let mut buffer = [0; 1024];
        let count = io::stdin().read(&mut buffer)?;
        input.extend_from_slice(&buffer[..count]);
    }
    Ok(input)
}

pub struct RawTerminal(nix::sys::termios::Termios);

impl RawTerminal {
    pub fn enter() -> Result<Self, nix::Error> {
        use nix::sys::termios::{self, SetArg, SpecialCharacterIndices};
        let original = termios::tcgetattr(io::stdin())?;
        let mut raw = original.clone();
        termios::cfmakeraw(&mut raw);
        raw.control_chars[SpecialCharacterIndices::VMIN as usize] = 0;
        raw.control_chars[SpecialCharacterIndices::VTIME as usize] = 1;
        termios::tcsetattr(io::stdin(), SetArg::TCSANOW, &raw)?;
        Ok(Self(original))
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        if let Err(error) =
            nix::sys::termios::tcsetattr(io::stdin(), nix::sys::termios::SetArg::TCSANOW, &self.0)
        {
            eprintln!("fixture: error: restoring terminal: {error}");
        }
    }
}
