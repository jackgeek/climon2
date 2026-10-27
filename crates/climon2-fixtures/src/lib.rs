use std::{io, os::fd::AsRawFd};

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
