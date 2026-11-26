#![cfg(any(target_os = "macos", target_os = "linux"))]

use climon2_platform::terminal;
use std::{
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
    process::{Child, Command, Stdio},
    sync::Mutex,
    time::{Duration, Instant},
};

static RAW_TEST: Mutex<()> = Mutex::new(());

fn pty() -> (File, File) {
    let (mut master, mut slave) = (-1, -1);
    let _allocation = climon2_platform::pty::allocation().unwrap();
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

/// #20: one client process owns raw mode once; a second owner is refused without touching termios.
#[test]
fn issue20_second_raw_mode_owner_is_refused() {
    let _lock = RAW_TEST.lock().unwrap();
    let (_master, slave) = pty();
    let (_other_master, other) = pty();
    let original = flags(&other);
    let _raw = terminal::RawModeGuard::new(&slave).unwrap();
    let error = terminal::RawModeGuard::new(&other)
        .err()
        .expect("second raw owner");
    assert_eq!(error.to_string(), "raw terminal already owned");
    assert_eq!(flags(&other), original);
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
    // A prompt's signals would hear this test's SIGTERM too.
    let _lock = RAW_TEST.lock().unwrap();
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
    // Unix terminals have no Ctrl+Break (#326): no signal is one.
    assert!(!signals.take_break());
}

/// Reads `master` until what it has read ends with `text`.
fn read_until(master: &mut File, text: &[u8]) {
    use std::io::Read;
    let mut read = Vec::new();
    while !read.ends_with(text) {
        let mut chunk = [0u8; 64];
        let count = master.read(&mut chunk).unwrap();
        assert_ne!(count, 0, "{read:?}");
        read.extend_from_slice(&chunk[..count]);
    }
}

/// #139: a nested-start prompt discards input typed before it, shows its
/// text on its output, gives typed bytes one at a time, so nothing after an
/// answer is taken, ends its wait on each signal that would end the process,
/// with 128 + signal, and restores the terminal when dropped.
#[test]
fn a_prompt_flushes_type_ahead_reads_bytes_singly_and_ends_on_signals() {
    use climon2_core::session::Input;
    use std::io::Write;
    let _lock = RAW_TEST.lock().unwrap();
    let (mut master, slave) = pty();
    let original = flags(&slave);
    master.write_all(b"early\r").unwrap();
    read_until(&mut master, b"early\r\n");
    let mut prompt =
        terminal::Prompt::new(slave.try_clone().unwrap(), slave.try_clone().unwrap()).unwrap();
    prompt.show("SHOWN\r\n").unwrap();
    read_until(&mut master, b"SHOWN\r\n");
    master.write_all(b"yz").unwrap();
    assert_eq!(prompt.read_input().unwrap(), Input::Key(b'y'));
    assert_eq!(prompt.read_input().unwrap(), Input::Key(b'z'));
    for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM] {
        unsafe { libc::raise(signal) };
        assert_eq!(
            prompt.read_input().unwrap(),
            Input::Ended(128 + signal as u32)
        );
    }
    drop(prompt);
    assert_eq!(flags(&slave), original);
}

const PROMPT_SIGNALS: [libc::c_int; 4] = [libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM];

/// Each prompt signal's current disposition.
fn dispositions() -> Vec<libc::sigaction> {
    PROMPT_SIGNALS
        .iter()
        .map(|signal| {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            assert_eq!(
                unsafe { libc::sigaction(*signal, std::ptr::null(), &mut action) },
                0
            );
            action
        })
        .collect()
}

/// Each prompt signal's current handler.
fn handlers() -> Vec<libc::sighandler_t> {
    dispositions()
        .iter()
        .map(|action| action.sa_sigaction)
        .collect()
}

/// #139: a prompt replaces the dispositions of the signals it hears and,
/// when dropped, puts back exactly what it replaced: the default action, so
/// that the signals end the process again while the session starts, and
/// any other handler as it was.
#[test]
fn a_dropped_prompt_puts_back_each_signals_disposition() {
    let _lock = RAW_TEST.lock().unwrap();
    let defaults = vec![libc::SIG_DFL; PROMPT_SIGNALS.len()];
    let ignored = vec![libc::SIG_IGN; PROMPT_SIGNALS.len()];
    for earlier in [&defaults, &ignored] {
        let saved = dispositions();
        for signal in PROMPT_SIGNALS {
            unsafe { libc::signal(signal, earlier[0]) };
        }
        let (_master, slave) = pty();
        let prompt =
            terminal::Prompt::new(slave.try_clone().unwrap(), slave.try_clone().unwrap()).unwrap();
        assert!(handlers().iter().all(|handler| !earlier.contains(handler)));
        drop(prompt);
        assert_eq!(&handlers(), earlier);
        for (signal, action) in PROMPT_SIGNALS.iter().zip(&saved) {
            unsafe { libc::sigaction(*signal, action, std::ptr::null_mut()) };
        }
    }
}

/// #139: a prompt whose terminal is lost, its pty master closed, reads it
/// as gone (EIO on Linux), never as a failure.
#[test]
fn a_prompt_reads_a_lost_terminal_as_gone() {
    use climon2_core::session::Input;
    let _lock = RAW_TEST.lock().unwrap();
    let (master, slave) = pty();
    let mut prompt =
        terminal::Prompt::new(slave.try_clone().unwrap(), slave.try_clone().unwrap()).unwrap();
    drop(master);
    assert_eq!(prompt.read_input().unwrap(), Input::Gone);
}

fn terminal_fixture(mode: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "owned_terminal_fixture",
            "--nocapture",
        ])
        .env("CLIMON2_TERMINAL_FIXTURE", mode);
    command
}

fn wait_fixture(mut child: Child) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "terminal fixture status: {status}");
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("terminal fixture exceeded deadline");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
#[ignore = "owned subprocess fixture"]
fn owned_terminal_fixture() {
    match std::env::var("CLIMON2_TERMINAL_FIXTURE").unwrap().as_str() {
        "daemon-signals" => {
            for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
                unsafe {
                    libc::signal(signal, libc::SIG_DFL);
                }
            }
            let signals = terminal::Signals::daemon().unwrap();
            for signal in [libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
                assert_eq!(unsafe { libc::raise(signal) }, 0);
                assert_eq!(signals.termination_signal(), None);
            }
            assert_eq!(unsafe { libc::raise(libc::SIGTERM) }, 0);
            assert_eq!(signals.termination_signal(), Some(15));
        }
        "tty" => {
            let accepted = std::env::var("CLIMON2_EXPECT_TTY").unwrap() == "yes";
            let result = terminal::require_ttys();
            if accepted {
                result.unwrap();
            } else {
                assert_eq!(result.unwrap_err().to_string(), "c2 exec needs a terminal");
            }
        }
        _ => unreachable!(),
    }
}

#[test]
fn issue19_daemon_ignores_hup_int_pipe_and_defers_term_in_owned_process() {
    wait_fixture(terminal_fixture("daemon-signals").spawn().unwrap());
}

#[test]
fn issue20_tty_requires_both_stdin_and_stdout_but_not_stderr() {
    for (input_tty, output_tty) in [(true, true), (false, true), (true, false)] {
        let (_master, slave) = pty();
        let mut command = terminal_fixture("tty");
        command
            .env(
                "CLIMON2_EXPECT_TTY",
                if input_tty && output_tty { "yes" } else { "no" },
            )
            .stdin(if input_tty {
                Stdio::from(slave.try_clone().unwrap())
            } else {
                Stdio::null()
            })
            .stdout(if output_tty {
                Stdio::from(slave)
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null());
        wait_fixture(command.spawn().unwrap());
    }
}
