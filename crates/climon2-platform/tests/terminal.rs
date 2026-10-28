#![cfg(target_os = "macos")]

use climon2_platform::terminal;
use std::{
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
    sync::Mutex,
};

static RAW_TEST: Mutex<()> = Mutex::new(());

fn pty() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) }
}

fn flags(file: &File) -> libc::tcflag_t {
    let mut state = std::mem::MaybeUninit::uninit();
    assert_eq!(
        unsafe { libc::tcgetattr(file.as_raw_fd(), state.as_mut_ptr()) },
        0
    );
    // Darwin marks pending input when canonical processing is restored.
    unsafe { state.assume_init() }.c_lflag & !libc::PENDIN
}

#[test]
fn issue20_real_pty_raw_mode_restores_on_drop_and_panic_without_screen_sequences() {
    let _lock = RAW_TEST.lock().unwrap();
    let (_master, slave) = pty();
    let original = flags(&slave);
    {
        let _raw = terminal::RawModeGuard::new(&slave).unwrap();
        assert_eq!(flags(&slave) & (libc::ICANON | libc::ECHO | libc::ISIG), 0);
    }
    assert_eq!(flags(&slave), original);
    let raw = terminal::RawModeGuard::new(&slave).unwrap();
    let result = std::panic::catch_unwind(|| {
        panic!("owned terminal panic");
    });
    assert!(result.is_err());
    // The guard is still alive: this checks the panic hook, not Drop again.
    assert_eq!(flags(&slave), original);
    drop(raw);
}

#[test]
fn issue20_real_pty_size_and_zero_fallback() {
    let (_master, slave) = pty();
    assert_eq!(terminal::size(&slave).unwrap().cols, 80);
    let size = libc::winsize {
        ws_row: 43,
        ws_col: 132,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe { libc::ioctl(slave.as_raw_fd(), libc::TIOCSWINSZ, &size) },
        0
    );
    let size = terminal::size(&slave).unwrap();
    assert_eq!((size.cols, size.rows), (132, 43));
    assert!(terminal::RawModeGuard::new(&File::open("/dev/null").unwrap()).is_err());
}

#[test]
fn issue20_signals_coalesce_resize_and_report_exit_signal() {
    let signals = terminal::Signals::client().unwrap();
    unsafe {
        libc::raise(libc::SIGWINCH);
        libc::raise(libc::SIGWINCH);
    }
    assert!(signals.take_resize());
    assert!(!signals.take_resize());
    unsafe {
        libc::raise(libc::SIGTERM);
    }
    assert_eq!(signals.termination_signal(), Some(15));
}
