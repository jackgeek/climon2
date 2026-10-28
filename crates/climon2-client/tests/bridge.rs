#![cfg(target_os = "macos")]

use climon2_core::{
    SessionId,
    protocol::{ClientMessage, HelloReply, ServerMessage, read_frame, write_frame},
};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{fs::OpenOptionsExt, net::UnixListener, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";

struct Pty {
    master: File,
    slave: File,
    child: Child,
    saved: libc::termios,
    /// The open-file description the client gets as stdout: the stdin one unless separate.
    output: File,
    /// F_GETFL of stdin's then stdout's description before the client started.
    status_flags: [i32; 2],
}

impl Pty {
    fn spawn(home: &Path, metadata: &Path, fault: Option<&str>) -> Self {
        Self::configured(home, metadata, fault, None)
    }

    fn configured(home: &Path, metadata: &Path, fault: Option<&str>, filter: Option<&str>) -> Self {
        Self::launch(home, metadata, fault, filter, false, false)
    }

    /// Gives the client a stdout that is a separate open-file description of the
    /// same terminal, so stdin's and stdout's file-status flags are independent.
    fn separate_output(home: &Path, metadata: &Path) -> Self {
        Self::launch(home, metadata, None, None, false, true)
    }

    /// Runs the client as a background job of a shell that owns the PTY as its
    /// controlling terminal, with SIGTTIN/SIGTTOU ignored, so terminal reads fail with EIO.
    /// The shell records the client's status and stderr under `home`, then stays alive:
    /// a session leader's exit would revoke the PTY before termios could be checked.
    fn background(home: &Path, metadata: &Path) -> Self {
        Self::launch(home, metadata, None, None, true, false)
    }

    fn launch(
        home: &Path,
        metadata: &Path,
        fault: Option<&str>,
        filter: Option<&str>,
        background: bool,
        separate_output: bool,
    ) -> Self {
        let mut master = -1;
        let mut slave = -1;
        let mut size = libc::winsize {
            ws_row: 31,
            ws_col: 97,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let allocation = climon2_platform::pty::allocation().unwrap();
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    &mut size,
                )
            },
            0,
            "openpty: {}",
            std::io::Error::last_os_error()
        );
        drop(allocation);
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut saved = std::mem::MaybeUninit::uninit();
        assert_eq!(
            unsafe { libc::tcgetattr(slave.as_raw_fd(), saved.as_mut_ptr()) },
            0
        );
        let saved = unsafe { saved.assume_init() };
        for file in [&master, &slave] {
            assert_eq!(
                unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
                0
            );
        }
        let output = if separate_output {
            reopen(&slave)
        } else {
            slave.try_clone().unwrap()
        };
        let status_flags = [status_flags(&slave), status_flags(&output)];
        let mut command = if background {
            background_job()
        } else {
            Command::new(env!("CARGO_BIN_EXE_climon2-client"))
        };
        command
            .args(["attach", "--metadata"])
            .arg(metadata)
            .env_clear()
            .env("HOME", home)
            .env("CLIMON2_HOME", home)
            .stdin(slave.try_clone().unwrap())
            .stdout(output.try_clone().unwrap())
            .stderr(Stdio::piped());
        if let Some(fault) = fault {
            command.env("CLIMON2_CLIENT_FAULT", fault);
        }
        if let Some(filter) = filter {
            command.env("CLIMON2_LOG", filter);
        }
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        let child = command.spawn().unwrap();
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        Self {
            master,
            slave,
            child,
            saved,
            output,
            status_flags,
        }
    }

    fn termios(&self) -> libc::termios {
        let mut termios = std::mem::MaybeUninit::uninit();
        assert_eq!(
            unsafe { libc::tcgetattr(self.slave.as_raw_fd(), termios.as_mut_ptr()) },
            0
        );
        unsafe { termios.assume_init() }
    }

    fn wait_raw(&self) {
        let deadline = Instant::now() + Duration::from_secs(3);
        while self.termios().c_lflag & libc::ICANON != 0 {
            assert!(Instant::now() < deadline, "client never entered raw mode");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn finish(&mut self, expected: i32) -> (Vec<u8>, String) {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let status = loop {
            while let Ok(count) = self.master.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&buffer[..count]);
            }
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "client did not exit; {} output bytes",
                bytes.len()
            );
            wait_ready(&self.master, libc::POLLIN, 5);
        };
        assert_eq!(status.code(), Some(expected));
        self.assert_restored();
        while let Ok(count) = self.master.read(&mut buffer) {
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..count]);
        }
        let mut error = String::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        (bytes, error)
    }

    fn assert_restored(&self) {
        assert_eq!(
            [status_flags(&self.slave), status_flags(&self.output)],
            self.status_flags,
            "terminal file-status flags (e.g. O_NONBLOCK) not restored"
        );
        assert_ne!(
            self.termios().c_lflag & libc::ICANON,
            0,
            "canonical mode not restored"
        );
        assert_ne!(self.termios().c_lflag & libc::ECHO, 0, "echo not restored");
        let restored = self.termios();
        assert_eq!(restored.c_iflag, self.saved.c_iflag);
        assert_eq!(restored.c_oflag, self.saved.c_oflag);
        assert_eq!(restored.c_cflag, self.saved.c_cflag);
        // Darwin sets this transient pending-input flag when canonical mode resumes.
        assert_eq!(
            restored.c_lflag & !libc::PENDIN,
            self.saved.c_lflag & !libc::PENDIN
        );
        assert_eq!(restored.c_cc, self.saved.c_cc);
        assert_eq!(restored.c_ispeed, self.saved.c_ispeed);
        assert_eq!(restored.c_ospeed, self.saved.c_ospeed);
    }
}

/// Darwin's kernel-private FWASWRITTEN: set by any write, not settable by F_SETFL.
const WAS_WRITTEN: i32 = 0x0001_0000;

/// Waits until `file` is ready for `events`, or `milliseconds` pass so the caller can
/// recheck its own exit conditions. A blind sleep would move only the PTY's ~1 KiB queue
/// per sleep (or notice a connection late), making the test, not the client, the
/// bottleneck or the clock on slow machines.
fn wait_ready(file: &impl AsRawFd, events: i16, milliseconds: i32) {
    let mut descriptor = libc::pollfd {
        fd: file.as_raw_fd(),
        events,
        revents: 0,
    };
    unsafe { libc::poll(&mut descriptor, 1, milliseconds) };
}

fn status_flags(file: &File) -> i32 {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFL) };
    assert!(flags >= 0, "{}", std::io::Error::last_os_error());
    flags & !WAS_WRITTEN
}

/// A new open-file description of the same terminal device.
fn reopen(terminal: &File) -> File {
    let mut name = [0 as libc::c_char; 128];
    assert_eq!(
        unsafe { libc::ttyname_r(terminal.as_raw_fd(), name.as_mut_ptr(), name.len()) },
        0
    );
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY)
        .open(name.to_str().unwrap())
        .unwrap()
}

fn background_job() -> Command {
    let mut command = Command::new("/bin/sh");
    command.args([
        "-c",
        "trap '' TTIN TTOU; set -m; \"$0\" \"$@\" 2>\"$HOME/stderr\" & wait $!; \
         echo $? >\"$HOME/status\"; exec /bin/sleep 60",
        env!("CARGO_BIN_EXE_climon2-client"),
    ]);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY.into(), 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    command
}

impl Drop for Pty {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn setup() -> (tempfile::TempDir, PathBuf, UnixListener) {
    let home = tempfile::Builder::new()
        .prefix("client-")
        .tempdir_in(std::env::current_dir().unwrap())
        .unwrap();
    let socket = home.path().join("peer.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let metadata = home.path().join(format!("{ID}.pty"));
    let value = serde_json::json!({
        "format_version": 1, "session_id": ID, "socket_path": socket,
        "daemon_pid": std::process::id(), "daemon_start_time": 1,
        "command": "fixture", "args": [], "cwd": home.path(),
        "depth": 1, "created_at": "2026-10-28T00:00:00.000Z",
        "climon2_version": "0.1.0"
    });
    fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
    (home, metadata, listener)
}

fn receive(stream: &mut std::os::unix::net::UnixStream) -> ClientMessage {
    ClientMessage::from_frame(read_frame(stream).unwrap()).unwrap()
}

fn send(stream: &mut std::os::unix::net::UnixStream, message: ServerMessage) {
    write_frame(stream, &message.to_frame().unwrap()).unwrap();
}

fn accept(listener: &UnixListener) -> std::os::unix::net::UnixStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "client did not connect");
                wait_ready(listener, libc::POLLIN, 5);
            }
            Err(error) => panic!("{error}"),
        }
    }
}

fn hello(stream: &mut std::os::unix::net::UnixStream) {
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    assert_eq!(receive(stream), ClientMessage::Hello { protocol: 1 });
    send(
        stream,
        ServerMessage::Hello(HelloReply {
            session_id: ID.parse::<SessionId>().unwrap(),
            climon2_version: "0.1.0".parse().unwrap(),
            protocol: 1,
        }),
    );
    assert_eq!(
        receive(stream),
        ClientMessage::Attach(climon2_core::protocol::TerminalSize { cols: 97, rows: 31 })
    );
}

/// #20: real raw terminal, exact binary/TUI bytes, Ctrl-C/Ctrl-Z and final status.
#[test]
fn raw_bridge_preserves_bytes_and_restores_termios_after_final_output() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    assert_ne!(
        pty.termios().c_lflag & libc::ICANON,
        0,
        "raw before Attached"
    );
    send(&mut stream, ServerMessage::Attached);
    pty.wait_raw();
    let input = b"\x03\x1a\x00\xff\x1b[200~paste\x1b[201~";
    pty.master.write_all(input).unwrap();
    let mut received = Vec::new();
    while received.len() < input.len() {
        match receive(&mut stream) {
            ClientMessage::Input(bytes) => received.extend(bytes),
            message => panic!("{message:?}"),
        }
    }

    assert_eq!(received, input);
    let output = b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?2004h\xff\x00\r\nfinal";
    send(&mut stream, ServerMessage::Output(output.to_vec()));
    send(&mut stream, ServerMessage::Exit { code: 47 });
    let (bytes, error) = pty.finish(47);
    assert_eq!(bytes, output);
    assert_eq!(error, "");
}

/// #19/#20: signals are statuses even while waiting for Attached, never errors.
#[test]
fn termination_before_attached_does_not_enter_raw_mode_or_report_error() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    assert_eq!(
        unsafe { libc::kill(pty.child.id() as i32, libc::SIGTERM) },
        0
    );
    let (bytes, error) = pty.finish(143);
    assert!(bytes.is_empty());
    assert_eq!(error, "");
}

/// #19/#20: HUP/TERM/INT restore termios and close rather than sending policy.
#[test]
fn raw_client_signals_restore_termios_close_connection_and_return_signal_status() {
    for signal in [libc::SIGHUP, libc::SIGTERM, libc::SIGINT] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, ServerMessage::Attached);
        pty.wait_raw();
        assert_eq!(unsafe { libc::kill(pty.child.id() as i32, signal) }, 0);
        let (bytes, error) = pty.finish(128 + signal);
        assert!(bytes.is_empty());
        assert!(error.is_empty());
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    }
}

/// #20/#25: every terminal protocol failure restores before one-line diagnosis.
#[test]
fn protocol_error_lost_connection_and_panic_restore_without_terminal_sequences() {
    for mode in [
        "error",
        "lost",
        "panic",
        "repeated-hello",
        "unknown",
        "oversized",
    ] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, (mode == "panic").then_some("panic"));
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, ServerMessage::Attached);
        if mode != "panic" {
            pty.wait_raw();
        }
        match mode {
            "error" => send(
                &mut stream,
                ServerMessage::Error {
                    message: "peer failed\nunsafe second line".into(),
                },
            ),
            "lost" => {
                stream.shutdown(std::net::Shutdown::Both).unwrap();
            }
            "repeated-hello" => send(
                &mut stream,
                ServerMessage::Hello(HelloReply {
                    session_id: ID.parse().unwrap(),
                    climon2_version: "0.1.0".parse().unwrap(),
                    protocol: 1,
                }),
            ),
            "unknown" => stream.write_all(&[0, 0, 0, 1, 0xff]).unwrap(),
            "oversized" => stream.write_all(&[0, 1, 0, 1, 0x21]).unwrap(),
            _ => (),
        }
        let (bytes, error) = pty.finish(125);
        assert!(bytes.is_empty(), "{mode}: {bytes:?}");
        assert_eq!(error.lines().count(), 1, "{mode}: {error}");
        assert!(error.starts_with("climon2: error: "), "{mode}: {error}");
        assert!(
            error.contains(&format!("(logs: {}/logs/{ID})", home.path().display())),
            "{error}"
        );
        if mode == "lost" {
            assert!(error.contains(&format!("lost connection to session {ID}")));
        }
        if mode == "panic" {
            assert!(error.contains("client panicked"));
            let log =
                fs::read_to_string(home.path().join(format!("logs/{ID}/client.log"))).unwrap();
            assert!(log.contains("client panic"));
            assert!(!log.contains("CLIMON2_CLIENT_FAULT_PANIC"));
        }
    }
}

/// ADR 0001: each hello identity dimension is checked before raw mode.
#[test]
fn hello_rejects_mismatched_session_protocol_version_and_unexpected_frames() {
    for mode in ["session", "protocol", "version", "attached", "malformed"] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut stream = accept(&listener);
        assert_eq!(receive(&mut stream), ClientMessage::Hello { protocol: 1 });
        if mode == "attached" {
            send(&mut stream, ServerMessage::Attached);
        } else if mode == "malformed" {
            stream.write_all(&[0, 0, 0, 2, 1, b'[']).unwrap();
        } else {
            send(
                &mut stream,
                ServerMessage::Hello(HelloReply {
                    session_id: if mode == "session" {
                        "11234567-89ab-4cde-8fab-0123456789ab"
                    } else {
                        ID
                    }
                    .parse()
                    .unwrap(),
                    climon2_version: if mode == "version" { "9.0.0" } else { "0.1.0" }
                        .parse()
                        .unwrap(),
                    protocol: if mode == "protocol" { 2 } else { 1 },
                }),
            );
        }
        let (bytes, error) = pty.finish(125);
        assert!(bytes.is_empty());
        assert_eq!(error.lines().count(), 1);
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
    }
}

/// ADR 0001: partial reply bytes never restart the two-second budget.
#[test]
fn hello_deadline_is_absolute_across_partial_reply() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    assert_eq!(receive(&mut stream), ClientMessage::Hello { protocol: 1 });
    let started = Instant::now();
    stream.write_all(&[0, 0]).unwrap();
    thread::sleep(Duration::from_millis(1100));
    stream.write_all(&[0, 100, 1, b'{']).unwrap();
    let (bytes, error) = pty.finish(125);
    assert!(started.elapsed() >= Duration::from_millis(1900));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(bytes.is_empty());
    assert!(error.contains(&format!("session {ID} did not respond")));
}

/// #20: fragmented headers/bodies and changed-size-only SIGWINCH forwarding.
#[test]
fn fragmented_frames_and_coalesced_resize_preserve_the_stream() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    let mut attached = Vec::new();
    write_frame(&mut attached, &ServerMessage::Attached.to_frame().unwrap()).unwrap();
    for byte in attached {
        stream.write_all(&[byte]).unwrap();
        thread::sleep(Duration::from_millis(2));
    }
    pty.wait_raw();
    let mut size = libc::winsize {
        ws_row: 44,
        ws_col: 132,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    assert_eq!(
        unsafe { libc::ioctl(pty.slave.as_raw_fd(), libc::TIOCSWINSZ, &mut size) },
        0
    );
    for _ in 0..10 {
        assert_eq!(
            unsafe { libc::kill(pty.child.id() as i32, libc::SIGWINCH) },
            0
        );
    }
    assert_eq!(
        receive(&mut stream),
        ClientMessage::Resize(climon2_core::protocol::TerminalSize {
            cols: 132,
            rows: 44
        })
    );
    for _ in 0..10 {
        assert_eq!(
            unsafe { libc::kill(pty.child.id() as i32, libc::SIGWINCH) },
            0
        );
    }
    stream
        .set_read_timeout(Some(Duration::from_millis(150)))
        .unwrap();
    let error = stream.read(&mut [0]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    let output = b"\xff\0\x1b[6nfinal";
    let mut frame = Vec::new();
    write_frame(
        &mut frame,
        &ServerMessage::Output(output.to_vec()).to_frame().unwrap(),
    )
    .unwrap();
    for byte in frame {
        stream.write_all(&[byte]).unwrap();
    }
    send(&mut stream, ServerMessage::Exit { code: 0 });
    let (bytes, error) = pty.finish(0);
    assert_eq!(bytes, output);
    assert!(error.is_empty());
}

/// #20: the caller's terminal file-status flags (O_NONBLOCK) are restored on normal,
/// error and signal exits when stdin and stdout are separate open-file descriptions;
/// the shared-description case is checked by every other `Pty::finish`.
#[test]
fn separate_stdin_and_stdout_descriptions_both_get_their_flags_back() {
    for ending in ["exit", "error", "signal"] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::separate_output(home.path(), &metadata);
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, ServerMessage::Attached);
        pty.wait_raw();
        let expected = match ending {
            "exit" => {
                send(&mut stream, ServerMessage::Exit { code: 7 });
                7
            }
            "error" => {
                send(
                    &mut stream,
                    ServerMessage::Error {
                        message: "peer failed".into(),
                    },
                );
                125
            }
            _ => {
                assert_eq!(
                    unsafe { libc::kill(pty.child.id() as i32, libc::SIGTERM) },
                    0
                );
                128 + libc::SIGTERM
            }
        };
        let (bytes, _) = pty.finish(expected);
        assert!(bytes.is_empty(), "{ending}: {bytes:?}");
    }
}

/// #20: blocked stdout backpressures sockets and Exit cannot overtake output.
#[test]
fn bounded_output_backpressure_drains_every_byte_before_exit() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    send(&mut stream, ServerMessage::Attached);
    pty.wait_raw();
    stream
        .set_write_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let producer = thread::spawn(move || {
        for _ in 0..16 {
            send(&mut stream, ServerMessage::Output(vec![b'X'; 32768]));
        }
        send(&mut stream, ServerMessage::Exit { code: 51 });
    });
    thread::sleep(Duration::from_millis(200));
    assert!(
        !producer.is_finished(),
        "blocked terminal did not backpressure the producer"
    );
    let (bytes, error) = pty.finish(51);
    producer.join().unwrap();
    assert_eq!(bytes.len(), 524_288);
    assert!(bytes.iter().all(|byte| *byte == b'X'));
    assert!(error.is_empty());
}

/// #20: losing stdin is a silent HUP, without manufacturing an Error frame.
#[test]
fn terminal_eof_or_eio_is_silent_status_129_and_closes_connection() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    send(&mut stream, ServerMessage::Attached);
    pty.wait_raw();
    drop(std::mem::replace(
        &mut pty.master,
        File::open("/dev/null").unwrap(),
    ));
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = pty.child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(status.code(), Some(129));
    let mut error = String::new();
    pty.child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut error)
        .unwrap();
    assert!(error.is_empty());
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
}

/// #20: a terminal read failing with EIO (lost terminal) is the same silent HUP as EOF.
#[test]
fn terminal_eio_is_silent_status_129_and_restores_termios() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::background(home.path(), &metadata);
    let mut stream = accept(&listener);
    hello(&mut stream);
    send(&mut stream, ServerMessage::Attached);
    pty.wait_raw();
    pty.master.write_all(b"x").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let status = loop {
        let status = fs::read_to_string(home.path().join("status")).unwrap_or_default();
        if status.ends_with('\n') {
            break status;
        }
        assert!(Instant::now() < deadline, "client did not exit");
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(status, "129\n");
    pty.assert_restored();
    assert_eq!(fs::read_to_string(home.path().join("stderr")).unwrap(), "");
    assert_eq!(stream.read(&mut [0]).unwrap(), 0);
}

/// ADR 0001/#19: a refused or unexpected reply to Attach fails before raw mode.
#[test]
fn attach_refusal_or_unexpected_reply_fails_without_raw_mode() {
    for (reply, diagnosis) in [
        (
            ServerMessage::Error {
                message: "session refused attach".into(),
            },
            "session refused attach",
        ),
        (ServerMessage::Exit { code: 0 }, "expected Attached"),
    ] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, reply);
        let (bytes, error) = pty.finish(125);
        assert!(bytes.is_empty());
        assert_eq!(error.lines().count(), 1, "{error}");
        assert!(error.contains(diagnosis), "{error}");
        assert!(
            error.contains(&format!("(logs: {}/logs/{ID})", home.path().display())),
            "{error}"
        );
    }
}

/// #19/#20: while terminal output is blocked, a session that hung up must not spin
/// the client: it sleeps in poll, then delivers every received byte once stdout
/// drains, and finally honours a buffered Exit or reports the lost connection (125).
#[test]
fn hung_up_session_during_blocked_output_sleeps_then_delivers_everything() {
    for exit in [None, Some(51)] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, ServerMessage::Attached);
        pty.wait_raw();
        send(&mut stream, ServerMessage::Output(vec![b'X'; 60_000]));
        if let Some(code) = exit {
            send(&mut stream, ServerMessage::Exit { code });
        }
        drop(stream);
        thread::sleep(Duration::from_millis(200));
        pty.master.write_all(b"x").unwrap();
        thread::sleep(Duration::from_millis(100));
        let before = cpu_time(pty.child.id());
        thread::sleep(Duration::from_secs(1));
        let used = cpu_time(pty.child.id()) - before;
        assert!(
            used < Duration::from_millis(200),
            "client used {used:?} CPU in 1 s while output was blocked"
        );
        let (bytes, error) = pty.finish(exit.map_or(125, |code| code as i32));
        assert_eq!(bytes.len(), 60_000);
        assert!(bytes.iter().all(|byte| *byte == b'X'));
        if exit.is_some() {
            assert_eq!(error, "");
        } else {
            assert_eq!(error.lines().count(), 1, "{error}");
            assert!(
                error.contains(&format!("lost connection to session {ID}")),
                "{error}"
            );
            assert!(
                error.contains(&format!("(logs: {}/logs/{ID})", home.path().display())),
                "{error}"
            );
        }
    }
}

/// CPU time (user + system) as reported by ps, e.g. `0:01.25`.
fn cpu_time(pid: u32) -> Duration {
    let output = Command::new("/bin/ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    let (minutes, seconds) = text.trim().rsplit_once(':').unwrap();
    let minutes: u64 = minutes.rsplit(':').next().unwrap().parse().unwrap();
    Duration::from_secs(minutes * 60) + Duration::from_secs_f64(seconds.parse().unwrap())
}

/// #19: disconnects during Attach identify the session, before entering raw.
#[test]
fn disconnect_before_attached_names_the_session() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    stream.shutdown(std::net::Shutdown::Both).unwrap();
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert!(
        error.contains(&format!("lost connection to session {ID}")),
        "{error}"
    );
}

/// ADR 0001: invalid prefix is rejected without waiting for a type/body byte.
#[test]
fn invalid_length_prefix_closes_immediately_without_a_type_byte() {
    for prefix in [[0, 0, 0, 0], [0, 16, 0, 1]] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, ServerMessage::Attached);
        pty.wait_raw();
        stream.write_all(&prefix).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while pty.child.try_wait().unwrap().is_none() {
            assert!(
                Instant::now() < deadline,
                "client waited for bytes after invalid prefix"
            );
            thread::sleep(Duration::from_millis(5));
        }
        let (bytes, error) = pty.finish(125);
        assert!(bytes.is_empty());
        assert!(error.contains("invalid frame length"));
    }
}

/// #20: socket saturation stops stdin, then forwards every byte in bounded frames.
#[test]
fn input_backpressure_is_bounded_and_lossless() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut stream = accept(&listener);
    hello(&mut stream);
    send(&mut stream, ServerMessage::Attached);
    pty.wait_raw();
    let mut writer = pty.master.try_clone().unwrap();
    let producer = thread::spawn(move || {
        let payload = vec![0xff; 524_288];
        let mut position = 0;
        let deadline = Instant::now() + Duration::from_secs(10);
        while position != payload.len() {
            match writer.write(&payload[position..]) {
                Ok(count) => position += count,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    wait_ready(&writer, libc::POLLOUT, 2)
                }
                Err(error) => panic!("{error}"),
            }
            assert!(Instant::now() < deadline);
        }
    });
    thread::sleep(Duration::from_millis(200));
    assert!(
        !producer.is_finished(),
        "blocked socket did not backpressure stdin"
    );
    let mut count = 0;
    while count < 524_288 {
        match receive(&mut stream) {
            ClientMessage::Input(bytes) => {
                assert!(bytes.len() <= 65_535);
                assert!(bytes.iter().all(|byte| *byte == 0xff));
                count += bytes.len();
            }
            message => panic!("{message:?}"),
        }
    }
    assert_eq!(count, 524_288);
    producer.join().unwrap();
    send(&mut stream, ServerMessage::Exit { code: 0 });
    let (bytes, error) = pty.finish(0);
    assert!(bytes.is_empty());
    assert!(error.is_empty());
}

/// #17/#25: connection errors are one-line failures with only written log paths.
#[test]
fn nonexistent_socket_and_bad_metadata_fail_without_touching_termios() {
    let (home, metadata, listener) = setup();
    drop(listener);
    fs::remove_file(home.path().join("peer.sock")).unwrap();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert_eq!(error.lines().count(), 1);
    assert!(error.contains("(logs: "));
    fs::write(&metadata, b"{").unwrap();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert_eq!(error.lines().count(), 1);
    assert!(!error.contains("(logs: "));
}

/// #25/#28: client filtering is environment-only and defaults include DEBUG.
#[test]
fn log_filter_reaches_client_through_environment_without_cli_flags() {
    for (filter, debug) in [(None, true), (Some("warn"), false)] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::configured(home.path(), &metadata, None, filter);
        let mut stream = accept(&listener);
        hello(&mut stream);
        send(&mut stream, ServerMessage::Attached);
        send(&mut stream, ServerMessage::Exit { code: 0 });
        let (bytes, error) = pty.finish(0);
        assert!(bytes.is_empty());
        assert!(error.is_empty());
        let log = fs::read_to_string(home.path().join(format!("logs/{ID}/client.log"))).unwrap();
        assert_eq!(log.contains("DEBUG"), debug, "{log}");
        assert!(!log.contains("CLIMON2_LOG"));
    }
}
