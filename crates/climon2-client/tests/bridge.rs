#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Ordinary attach (#20, #101): the real client executable on real PTYs,
//! attaching through its own version folder's client library over daemon
//! wire revision 1. Each test installs the client executable and the
//! workspace's client library in a private version folder, and answers as a
//! revision-1 peer whose frames are written here by hand from the wire
//! reference, never by the code under test. These are the bridge tests of
//! the private protocol, re-expressed over revision-1 peers: they prove the
//! same terminal behaviour.

use climon2_core::protocol::{ClientMessage, PROTOCOL_VERSION, read_frame};
use climon2_test_term::{client_library_path, copy_executable};
use serde_json::{Value, json};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::{
            fs::{OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const ID: &str = "01234567-89ab-4cde-8fab-0123456789ab";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// A bound for eventual events on a loaded host: process start-up, the
/// library's load and connection, and the client's reactions.
const BOUND: Duration = Duration::from_secs(30);

/// Which open-file descriptions of the terminal the client gets as stdin and stdout.
#[derive(Clone, Copy)]
enum Descriptions {
    Shared,
    SeparateOutput,
    /// Each open for one direction only: a read-only stdin whose file-status flags are
    /// all clear, and a write-only stdout.
    OneWay,
}

struct Pty {
    master: File,
    slave: File,
    child: Child,
    saved: libc::termios,
    /// The open-file description the client gets as stdin: the slave's unless read-only.
    input: File,
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
        Self::launch(home, metadata, fault, filter, false, Descriptions::Shared)
    }

    /// Gives the client a stdout that is a separate open-file description of the
    /// same terminal, so stdin's and stdout's file-status flags are independent.
    fn separate_output(home: &Path, metadata: &Path) -> Self {
        Self::launch(
            home,
            metadata,
            None,
            None,
            false,
            Descriptions::SeparateOutput,
        )
    }

    /// Gives the client a read-only stdin whose file-status flags are all clear and a
    /// write-only stdout.
    fn one_way(home: &Path, metadata: &Path) -> Self {
        Self::launch(home, metadata, None, None, false, Descriptions::OneWay)
    }

    /// Runs the client as a background job of a shell that owns the PTY as its
    /// controlling terminal, with SIGTTIN/SIGTTOU ignored, so terminal reads fail with EIO.
    /// The shell records the client's status and stderr under `home`, then stays alive:
    /// a session leader's exit would revoke the PTY before termios could be checked.
    fn background(home: &Path, metadata: &Path) -> Self {
        Self::launch(home, metadata, None, None, true, Descriptions::Shared)
    }

    fn launch(
        home: &Path,
        metadata: &Path,
        fault: Option<&str>,
        filter: Option<&str>,
        background: bool,
        descriptions: Descriptions,
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
                    &raw mut size,
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
        let (input, output) = match descriptions {
            Descriptions::Shared => (slave.try_clone().unwrap(), slave.try_clone().unwrap()),
            Descriptions::SeparateOutput => (slave.try_clone().unwrap(), reopen(&slave)),
            Descriptions::OneWay => (
                one_way(&master, &slave, libc::O_RDONLY),
                one_way(&master, &slave, libc::O_WRONLY),
            ),
        };
        let status_flags = [status_flags(&input), status_flags(&output)];
        let client = installed_client(home);
        let mut command = if background {
            background_job(&client)
        } else {
            climon2_test_term::process_command(&client)
        };
        command
            .args(["attach", "--metadata"])
            .arg(metadata)
            .env_clear()
            .env("HOME", home)
            .env("CLIMON2_HOME", home)
            .env(
                "CLIMON2_CLIENT_STARTUP",
                r#"{"schema":1,"operation":"attach","settings":{"escape_prefix":29,"take_control_key":20}}"#,
            )
            .stdin(input.try_clone().unwrap())
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
            input,
            output,
            status_flags,
        }
    }

    /// Runs `action` while the client is stopped, so the client sees only its outcome.
    fn while_stopped(&mut self, action: impl FnOnce(&mut Self)) {
        let pid = self.child.id() as libc::pid_t;
        assert_eq!(unsafe { libc::kill(pid, libc::SIGSTOP) }, 0);
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid, &raw mut status, libc::WUNTRACED) },
            pid
        );
        assert!(libc::WIFSTOPPED(status));
        action(self);
        assert_eq!(unsafe { libc::kill(pid, libc::SIGCONT) }, 0);
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
        let deadline = Instant::now() + BOUND;
        while self.termios().c_lflag & libc::ICANON != 0 {
            assert!(Instant::now() < deadline, "client never entered raw mode");
            thread::sleep(Duration::from_millis(5));
        }
    }

    /// Bytes typed that the client has not read.
    fn unread(&self) -> i32 {
        let mut pending = 0;
        assert_eq!(
            unsafe { libc::ioctl(self.slave.as_raw_fd(), libc::FIONREAD, &mut pending) },
            0
        );
        pending
    }

    fn wait_consumed(&self) {
        let deadline = Instant::now() + BOUND;
        loop {
            let mut pending = 0;
            assert_eq!(
                unsafe { libc::ioctl(self.slave.as_raw_fd(), libc::FIONREAD, &mut pending) },
                0
            );
            if pending == 0 {
                return;
            }
            assert!(Instant::now() < deadline, "client did not read its input");
            thread::sleep(Duration::from_millis(2));
        }
    }

    /// Resizes the terminal and tells the client once.
    fn resize(&self, cols: u16, rows: u16) {
        self.resize_signalled(cols, rows, 1);
    }

    /// Resizes the terminal, then sends the client `times` SIGWINCH.
    fn resize_signalled(&self, cols: u16, rows: u16, times: usize) {
        let mut size = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe { libc::ioctl(self.slave.as_raw_fd(), libc::TIOCSWINSZ, &mut size) },
            0
        );
        for _ in 0..times {
            assert_eq!(
                unsafe { libc::kill(self.child.id() as i32, libc::SIGWINCH) },
                0
            );
        }
    }

    /// Everything the client wrote, up to and including `needle`.
    fn read_until(&mut self, needle: &[u8]) -> Vec<u8> {
        let deadline = Instant::now() + BOUND;
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        while !bytes.windows(needle.len()).any(|window| window == needle) {
            assert!(Instant::now() < deadline, "client never wrote {needle:?}");
            wait_ready(&self.master, libc::POLLIN, 5);
            if let Ok(count) = self.master.read(&mut buffer) {
                bytes.extend_from_slice(&buffer[..count]);
            }
        }
        bytes
    }

    fn finish(&mut self, expected: i32) -> (Vec<u8>, String) {
        let deadline = Instant::now() + BOUND;
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
        if status.code() != Some(expected) {
            let mut error = String::new();
            self.child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut error)
                .unwrap();
            panic!(
                "expected client exit {expected}, got {:?}: {error}",
                status.code()
            );
        }
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
            [status_flags(&self.input), status_flags(&self.output)],
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
#[cfg(target_os = "macos")]
const WAS_WRITTEN: i32 = 0x0001_0000;
#[cfg(target_os = "linux")]
const WAS_WRITTEN: i32 = 0;

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

/// A new read-only open-file description of the same terminal, with every
/// file-status flag clear, as `climon2 attach < /dev/tty` gives on macOS.
/// Opens the terminal again for `access` alone (O_RDONLY or O_WRONLY).
fn one_way(master: &File, terminal: &File, access: i32) -> File {
    #[cfg(target_os = "macos")]
    {
        let _ = master;
        let mut name = [0 as libc::c_char; libc::PATH_MAX as usize];
        assert_ne!(
            unsafe { libc::fcntl(terminal.as_raw_fd(), libc::F_GETPATH, name.as_mut_ptr()) },
            -1
        );
        let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
        let file = fs::OpenOptions::new()
            .read(access == libc::O_RDONLY)
            .write(access == libc::O_WRONLY)
            .custom_flags(libc::O_NOCTTY)
            .open(name.to_str().unwrap())
            .unwrap();
        close_on_exec(&file);
        file
    }
    #[cfg(target_os = "linux")]
    {
        // A path open adds O_LARGEFILE; the master's peer open keeps the flags as given.
        let _ = terminal;
        let fd = unsafe {
            libc::ioctl(
                master.as_raw_fd(),
                libc::TIOCGPTPEER,
                access | libc::O_NOCTTY,
            )
        };
        assert!(fd >= 0, "{}", std::io::Error::last_os_error());
        let file = unsafe { File::from_raw_fd(fd) };
        close_on_exec(&file);
        file
    }
}

fn close_on_exec(file: &File) {
    assert_eq!(
        unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
        0
    );
}

/// A new open-file description of the same terminal device.
fn reopen(terminal: &File) -> File {
    #[cfg(target_os = "macos")]
    {
        // F_GETPATH, unlike Darwin's ttyname_r, never fails intermittently with ERANGE.
        let mut name = [0 as libc::c_char; libc::PATH_MAX as usize];
        assert_ne!(
            unsafe { libc::fcntl(terminal.as_raw_fd(), libc::F_GETPATH, name.as_mut_ptr()) },
            -1
        );
        let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(name.to_str().unwrap())
            .unwrap()
    }
    #[cfg(target_os = "linux")]
    {
        fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(format!("/proc/self/fd/{}", terminal.as_raw_fd()))
            .unwrap()
    }
}

fn background_job(client: &Path) -> Command {
    let mut command = climon2_test_term::process_command("/bin/sh");
    command.args([
        "-c",
        "trap '' TTIN TTOU; set -m; \"$0\" \"$@\" 2>\"$HOME/stderr\" & wait $!; \
         echo $? >\"$HOME/status\"; exec /bin/sleep 60",
    ]);
    command.arg(client);
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
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

/// The client executable installed in its version folder, beside the
/// workspace's client library.
fn installed_client(home: &Path) -> PathBuf {
    home.join("exe").join(VERSION).join("climon2-client")
}

/// A session of this release whose socket the test serves, and the client
/// executable and library installed in a private version folder.
fn setup() -> (tempfile::TempDir, PathBuf, UnixListener) {
    let home = tempfile::Builder::new()
        .prefix("client-")
        .tempdir_in("/tmp")
        .unwrap();
    let client = installed_client(home.path());
    let folder = client.parent().unwrap();
    fs::create_dir_all(folder).unwrap();
    copy_executable(Path::new(env!("CARGO_BIN_EXE_climon2-client")), &client);
    let library = client_library_path();
    copy_executable(&library, &folder.join(library.file_name().unwrap()));
    let socket = home.path().join("peer.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    let metadata = home.path().join(format!("{ID}.pty"));
    let value = json!({
        "format_version": 1, "session_id": ID, "socket_path": socket,
        "daemon_pid": std::process::id(), "daemon_start_time": 1,
        "command": "fixture", "args": [], "cwd": home.path(),
        "depth": 1, "created_at": "2026-10-28T00:00:00.000Z",
        "climon2_version": VERSION
    });
    fs::write(&metadata, serde_json::to_vec(&value).unwrap()).unwrap();
    (home, metadata, listener)
}

/// A control period and restoration ID.
type Authority = (u64, u64);

/// The authority of the attach grant.
const FIRST: Authority = (1, 1);

/// A request the client's library sent.
#[derive(Debug, PartialEq)]
enum Request {
    /// A JSON request: its type and body.
    Json(u8, Value),
    /// Input: its request ID, authority and bytes.
    Input(u64, Authority, Vec<u8>),
}

/// One revision-1 frame: a big-endian length counting the type byte, the
/// type and the body.
fn frame(code: u8, body: &[u8]) -> Vec<u8> {
    let length = u32::try_from(body.len() + 1).unwrap();
    [&length.to_be_bytes()[..], &[code], body].concat()
}

/// A Display of `bytes` under `authority`: a binary header of the period
/// and restoration ID, then the bytes.
fn display_frame((period, restoration): Authority, bytes: &[u8]) -> Vec<u8> {
    let header = [period.to_be_bytes(), restoration.to_be_bytes()].concat();
    frame(0x49, &[&header[..], bytes].concat())
}

fn ids((period, restoration): Authority) -> Value {
    json!({ "period": period.to_string(), "restoration_id": restoration.to_string() })
}

/// The session daemon, as a revision-1 peer of the client's library.
struct Peer {
    stream: UnixStream,
}

impl Peer {
    fn accept(listener: &UnixListener) -> Self {
        Self::accept_checked(listener, None)
    }

    /// Fails at once, with the client's diagnostic, if it exits instead.
    fn accept_client(listener: &UnixListener, child: &mut Child) -> Self {
        Self::accept_checked(listener, Some(child))
    }

    fn accept_checked(listener: &UnixListener, mut child: Option<&mut Child>) -> Self {
        listener.set_nonblocking(true).unwrap();
        let deadline = Instant::now() + BOUND;
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream.set_read_timeout(Some(BOUND)).unwrap();
                    return Self { stream };
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if let Some(status) = child
                        .as_deref_mut()
                        .and_then(|child| child.try_wait().unwrap())
                    {
                        let child = child.unwrap();
                        let mut stderr = String::new();
                        child
                            .stderr
                            .take()
                            .unwrap()
                            .read_to_string(&mut stderr)
                            .unwrap();
                        panic!("client exited before connecting ({status}): {stderr}");
                    }
                    assert!(Instant::now() < deadline, "client did not connect");
                    wait_ready(listener, libc::POLLIN, 5);
                }
                Err(error) => panic!("{error}"),
            }
        }
    }

    fn write(&mut self, bytes: &[u8]) {
        self.stream.write_all(bytes).unwrap();
    }

    fn send(&mut self, code: u8, body: &Value) {
        self.write(&frame(code, body.to_string().as_bytes()));
    }

    /// The next frame's type and body, or `None` once the client closed the
    /// connection.
    fn frame(&mut self) -> Option<(u8, Vec<u8>)> {
        let mut length = [0; 4];
        self.stream.read_exact(&mut length).ok()?;
        let mut frame = vec![0; u32::from_be_bytes(length) as usize];
        self.stream.read_exact(&mut frame).unwrap();
        let body = frame.split_off(1);
        Some((frame[0], body))
    }

    fn request(&mut self) -> Request {
        let (code, body) = self.frame().expect("the client closed the connection");
        if code != 0x31 {
            return Request::Json(code, serde_json::from_slice(&body).unwrap());
        }
        let number = |index: usize| u64::from_be_bytes(body[index..index + 8].try_into().unwrap());
        Request::Input(number(0), (number(8), number(16)), body[24..].to_vec())
    }

    /// The next request, a JSON one of type `code`: its body.
    fn json(&mut self, code: u8) -> Value {
        match self.request() {
            Request::Json(received, body) if received == code => body,
            other => panic!("expected request {code:#x}, got {other:?}"),
        }
    }

    /// Nothing more arrives: the client closed the connection.
    fn assert_closed(&mut self) {
        assert_eq!(self.frame(), None);
    }

    fn respond(&mut self, id: &Value, outcome: &str, result: Value, error: Value) {
        let body =
            json!({ "request_id": id, "outcome": outcome, "result": result, "error": error });
        self.send(0x40, &body);
    }

    /// Answers `request` successfully with `result`.
    fn answer(&mut self, request: &Value, result: Value) {
        self.respond(&request["request_id"], "ok", result, Value::Null);
    }

    /// Answers the library's frozen hello with `extensions` merged over a
    /// reply advertising revision 1 (a null removes a member).
    fn hello_with(&mut self, extensions: Value) {
        let hello = read_frame(&mut self.stream).ok();
        assert_eq!(
            hello.map(|frame| ClientMessage::from_frame(frame).unwrap()),
            Some(ClientMessage::Hello {
                protocol: PROTOCOL_VERSION
            })
        );
        let mut body = json!({
            "session_id": ID, "climon2_version": VERSION, "protocol": 1,
            "capabilities": ["attach", "kill", "state-v1", "client-startup-v1", "daemon-wire-v1"],
            "state": { "lifecycle": "starting", "attachments": 0, "controlled": false },
            "daemon_wire_v1": {
                "max_request_frame_bytes": 1_048_576, "max_server_frame_bytes": 1_048_576
            },
        });
        let reply = body.as_object_mut().unwrap();
        for (name, value) in extensions.as_object().unwrap() {
            match value {
                Value::Null => reply.remove(name),
                value => reply.insert(name.clone(), value.clone()),
            };
        }
        self.send(1, &body);
    }

    /// The hello, then the interactive Open at the terminal's 97x31, which
    /// requires `daemon-wire-v1` and selects no optional family.
    fn open(&mut self) -> Value {
        self.hello_with(json!({}));
        let open = self.json(0x30);
        assert_eq!(
            open,
            json!({
                "request_id": open["request_id"], "revision": 1,
                "required_capabilities": ["daemon-wire-v1"], "optional_capabilities": [],
                "mode": "interactive", "geometry": { "cols": 97, "rows": 31 }
            })
        );
        open
    }

    /// Admits `open`: its success, then the Initial metadata.
    fn opened(&mut self, open: &Value) {
        let opened = json!({
            "kind": "opened", "connection_id": "00112233-4455-6677-8899-aabbccddeeff",
            "selected_capabilities": ["daemon-wire-v1"]
        });
        self.answer(open, opened);
        let unavailable = json!({ "available": false, "value": null });
        let metadata = json!({
            "title": unavailable, "status": unavailable, "progress": unavailable,
            "command_result": null
        });
        self.send(0x41, &metadata);
    }

    /// The hello and the Open, admitted.
    fn attach(&mut self) {
        let open = self.open();
        self.opened(&open);
    }

    /// Control granted under `authority`, and its restoration's beginning.
    fn grant(&mut self, authority: Authority, reason: &str) {
        let geometry = json!({ "cols": 97, "rows": 31 });
        let mut granted = ids(authority);
        granted["geometry"] = geometry.clone();
        granted["reason"] = reason.into();
        self.send(0x46, &granted);
        let mut begin = ids(authority);
        begin["geometry"] = geometry;
        self.send(0x48, &begin);
    }

    fn complete(&mut self, authority: Authority) {
        self.send(0x4a, &ids(authority));
    }

    fn display(&mut self, authority: Authority, bytes: &[u8]) {
        self.write(&display_frame(authority, bytes));
    }

    fn revoke(&mut self, authority: Authority, reason: &str) {
        let mut revoked = ids(authority);
        revoked["reason"] = reason.into();
        self.send(0x47, &revoked);
    }

    /// The client's Ready for `authority`, answered.
    fn ready(&mut self, authority: Authority) {
        let ready = self.json(0x34);
        let mut expected = ids(authority);
        expected["request_id"] = ready["request_id"].clone();
        assert_eq!(ready, expected);
        let mut result = ids(authority);
        result["kind"] = "ready".into();
        result["changed"] = true.into();
        self.answer(&ready, result);
    }

    /// The attach grant's empty restoration, and the client's Ready.
    fn begin(&mut self) {
        self.grant(FIRST, "attach");
        self.complete(FIRST);
        self.ready(FIRST);
    }

    /// The next request, an Input under `authority`: its bytes, accepted.
    fn input_under(&mut self, authority: Authority) -> Vec<u8> {
        match self.request() {
            Request::Input(id, under, bytes) if under == authority => {
                let accepted = json!({ "kind": "input_accepted", "accepted_bytes": bytes.len() });
                self.respond(&id.to_string().into(), "ok", accepted, Value::Null);
                bytes
            }
            other => panic!("expected Input under {authority:?}, got {other:?}"),
        }
    }

    fn input(&mut self) -> Vec<u8> {
        self.input_under(FIRST)
    }

    /// A connection-fatal error carrying `message`.
    fn fatal(&mut self, message: &str) {
        self.send(
            0x4d,
            &json!({ "code": "internal_error", "message": message }),
        );
    }

    fn final_exit(&mut self, code: u32) {
        let result = json!({ "kind": "exit", "code": code });
        self.send(0x4c, &json!({ "command_result": result, "status": null }));
    }

    /// The command exited with `code`: Command final, then the close.
    fn exit(mut self, code: u32) {
        self.final_exit(code);
    }
}

/// COM-01 Monitor parity case (#20): real raw terminal, exact binary/TUI
/// bytes, Ctrl-C/Ctrl-Z and the final status. Raw mode begins only once the
/// Open is admitted; input goes as Input under the grant's authority; the
/// command's exit status, then the session-ending revocation and the close,
/// end the client with that status.
#[test]
fn raw_bridge_preserves_bytes_and_restores_termios_after_final_output() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept_client(&listener, &mut pty.child);
    let open = peer.open();
    assert_ne!(
        pty.termios().c_lflag & libc::ICANON,
        0,
        "raw before the Open was admitted"
    );
    peer.opened(&open);
    peer.begin();
    pty.wait_raw();
    let input = b"\x03\x1a\x00\xff\x1b[200~paste\x1b[201~";
    pty.master.write_all(input).unwrap();
    let mut received = Vec::new();
    while received.len() < input.len() {
        received.extend(peer.input());
    }

    assert_eq!(received, input);
    let output = b"\x1b[?1049h\x1b[?25l\x1b[?1000h\x1b[?2004h\xff\x00\r\nfinal";
    peer.display(FIRST, output);
    peer.final_exit(47);
    peer.revoke(FIRST, "session_ending");
    drop(peer);
    let (bytes, error) = pty.finish(47);
    assert_eq!(bytes, output);
    assert_eq!(error, "");
}

/// COM-01 Monitor parity case: keys typed before the restoration completes
/// are drained and never replayed; Ready follows the written restoration,
/// and only later keys are input.
#[test]
fn input_typed_before_restore_end_is_drained_and_never_replayed() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    pty.wait_raw();

    pty.master.write_all(b"stale\n").unwrap();
    pty.wait_consumed();

    peer.grant(FIRST, "attach");
    peer.display(FIRST, b"restored-screen");
    peer.complete(FIRST);
    peer.ready(FIRST);
    assert!(
        pty.read_until(b"restored-screen")
            .ends_with(b"restored-screen")
    );
    pty.master.write_all(b"live").unwrap();
    assert_eq!(peer.input(), b"live");
    peer.exit(0);
    assert_eq!(pty.finish(0), (Vec::new(), String::new()));
}

/// COM-01 Monitor parity case: Ready waits until typed-ahead keys are
/// drained, not only until the restoration is written. Keys typed while the
/// written restoration waits for the terminal, more than one read takes,
/// are all drained and dropped before Ready; only later keys are input.
#[test]
fn ready_waits_until_typed_ahead_keys_are_drained() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    pty.wait_raw();
    fill_output_queue(&pty.slave);
    peer.grant(FIRST, "attach");
    peer.display(FIRST, b"restored-screen");
    peer.complete(FIRST);
    wait_received(&peer.stream);
    pty.while_stopped(|pty| {
        fill_input_queue(&pty.master);
        assert_eq!(
            unsafe { libc::tcflow(pty.slave.as_raw_fd(), libc::TCOON) },
            0
        );
    });

    assert!(
        pty.read_until(b"restored-screen")
            .ends_with(b"restored-screen")
    );
    peer.ready(FIRST);
    pty.master.write_all(b"live").unwrap();
    let input = peer.input();
    assert!(
        input == b"live",
        "{} typed-ahead bytes became input",
        input.len()
    );
    peer.exit(0);
    assert_eq!(pty.finish(0), (Vec::new(), String::new()));
}

/// Types into the terminal until it takes no more keys.
fn fill_input_queue(mut master: &File) {
    loop {
        match master.write(&[b'x'; 4096]) {
            Ok(0) => panic!("terminal input made no progress"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("{error}"),
        }
    }
}

/// Waits until the client's library has read everything sent to it, so a
/// stopped client resumes with those frames on its side.
#[cfg(target_os = "linux")]
fn wait_received(stream: &UnixStream) {
    let deadline = Instant::now() + BOUND;
    loop {
        let mut unread: libc::c_int = 0;
        // SIOCOUTQ, which Linux defines as TIOCOUTQ: bytes sent and not yet read.
        assert_eq!(
            unsafe { libc::ioctl(stream.as_raw_fd(), libc::TIOCOUTQ, &mut unread) },
            0
        );
        if unread == 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the library never read its frames"
        );
        thread::sleep(Duration::from_millis(2));
    }
}

/// macOS reports no unread count for a Unix socket; the frames are read
/// once the client resumes.
#[cfg(target_os = "macos")]
fn wait_received(_: &UnixStream) {}

const PLACEHOLDER: &[u8] =
    b"\x1b[H\x1b[2JThis session is being viewed elsewhere.\r\nPress Ctrl+T to take control";

/// COM-01 Monitor parity case (#9): after its release bytes a revoked
/// client blanks the screen without losing what was visible, redraws on
/// resize, asks to take control with the configured key (Acquire), and ends
/// the placeholder below itself before restoring the terminal.
#[test]
fn a_revoked_client_shows_the_placeholder_until_the_session_ends() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    peer.display(FIRST, b"release");
    peer.revoke(FIRST, "takeover");
    let mut expected = b"release\x1b[31;1H".to_vec();
    expected.extend([b'\n'; 31]);
    expected.extend(b"\x1b[H");
    expected.extend(&PLACEHOLDER[7..]);
    assert_eq!(pty.read_until(b"take control"), expected);

    pty.resize(132, 44);
    assert_eq!(pty.read_until(b"take control"), PLACEHOLDER);
    pty.master.write_all(b"typed\x14").unwrap();
    let acquire = peer.json(0x33);
    assert_eq!(acquire, json!({ "request_id": acquire["request_id"] }));
    peer.exit(5);
    assert_eq!(pty.finish(5), (b"\x1b[44;1H\r\n".to_vec(), String::new()));
}

/// COM-01 Monitor parity case: resize forwarding. An observer holds no
/// authority to resize under, so the session hears its new size once it
/// holds control again: right after the grant, under the new authority,
/// before its Ready.
#[test]
fn a_resized_observer_tells_the_session_its_size_once_it_holds_control() {
    let (_home, mut pty, mut peer) = observing();
    pty.resize(132, 44);
    assert_eq!(pty.read_until(b"take control"), PLACEHOLDER);
    pty.master.write_all(b"\x14").unwrap();
    let acquire = peer.json(0x33);
    let second = (2, 2);
    peer.grant(second, "acquire");
    let mut acquired = ids(second);
    acquired["kind"] = "acquired".into();
    acquired["changed"] = true.into();
    peer.answer(&acquire, acquired);
    let resize = peer.json(0x32);
    let mut expected = ids(second);
    expected["request_id"] = resize["request_id"].clone();
    expected["geometry"] = json!({ "cols": 132, "rows": 44 });
    assert_eq!(resize, expected);
    peer.answer(&resize, json!({ "kind": "resize_accepted" }));
    peer.complete(second);
    peer.ready(second);
    pty.master.write_all(b"live").unwrap();
    assert_eq!(peer.input_under(second), b"live");
    peer.exit(0);
    pty.finish(0);
}

/// Section 1.7.1 item 3: what unexpected loss can still restore without a daemon
/// response; never the main screen.
const CONSERVATIVE_RESET: &[u8] = b"\x1b[?2026l\x1b[0m\x1b[?25h\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?1l\x1b>";

/// A client that shows the placeholder after a revocation.
fn observing() -> (tempfile::TempDir, Pty, Peer) {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    peer.revoke(FIRST, "takeover");
    pty.read_until(b"take control");
    (home, pty, peer)
}

/// COM-01 Monitor parity case (section 3.5 exit ordering): a daemon's fatal
/// error ends the placeholder below itself before the terminal is restored
/// and the one-line diagnostic is printed.
#[test]
fn an_error_while_observing_ends_the_placeholder_before_the_diagnostic() {
    let (_home, mut pty, mut peer) = observing();
    peer.fatal("refused");
    let (bytes, error) = pty.finish(125);
    assert_eq!(bytes, b"\x1b[31;1H\r\n");
    assert_eq!(error.lines().count(), 1, "{error}");
    assert!(error.contains("refused"), "{error}");
}

/// COM-01 Monitor parity case: a termination signal keeps its status but
/// never leaves the placeholder's cursor at the top of the screen.
#[test]
fn a_signal_while_observing_ends_the_placeholder_and_keeps_the_signal_status() {
    let (_home, mut pty, _peer) = observing();
    assert_eq!(
        unsafe { libc::kill(pty.child.id() as i32, libc::SIGTERM) },
        0
    );
    assert_eq!(pty.finish(143), (b"\x1b[31;1H\r\n".to_vec(), String::new()));
}

/// COM-01 Monitor parity case (section 1.7.1 item 3): unexpected loss writes
/// the conservative reset, then still ends the placeholder it knows it drew.
#[test]
fn a_lost_connection_while_observing_resets_conservatively_and_ends_the_placeholder() {
    let (_home, mut pty, peer) = observing();
    drop(peer);
    let (bytes, error) = pty.finish(125);
    let mut expected = CONSERVATIVE_RESET.to_vec();
    expected.extend(b"\x1b[31;1H\r\n");
    assert_eq!(bytes, expected);
    assert!(error.contains(&format!("lost connection to session {ID}")));
}

/// COM-01 Monitor parity case (#9): the paste stream outlives role changes:
/// a paste begun under one role is discarded through its closing marker,
/// never read as keys, under the next.
#[test]
fn a_paste_spanning_role_changes_is_never_replayed_or_read_as_keys() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    pty.master.write_all(b"\x1b[200~abc").unwrap();
    assert_eq!(peer.input(), b"\x1b[200~abc");
    peer.revoke(FIRST, "takeover");
    pty.read_until(b"take control");
    pty.master.write_all(b"\x14\x1dd more\x1b[201~").unwrap();
    pty.wait_consumed();
    pty.master.write_all(b"\x14").unwrap();
    let acquire = peer.json(0x33);

    pty.master.write_all(b"\x1b[200~observed").unwrap();
    pty.wait_consumed();
    let second = (2, 2);
    peer.grant(second, "acquire");
    let mut acquired = ids(second);
    acquired["kind"] = "acquired".into();
    acquired["changed"] = true.into();
    peer.answer(&acquire, acquired);
    peer.complete(second);
    peer.ready(second);
    pty.master.write_all(b"\x1dd\x14tail\x1b[201~live").unwrap();
    assert_eq!(peer.input_under(second), b"live");
    peer.exit(0);
    pty.finish(0);
}

/// COM-01 Monitor parity case: the escape prefix and `d` send Leave; without
/// its answer the client gives up after the five-second detach deadline,
/// writes the conservative reset and restores the terminal. Keys typed after
/// the detach stay unread, for the next program.
#[test]
fn unconfirmed_detach_exits_after_five_seconds_and_restores_terminal() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();

    pty.master.write_all(b"\x1dd").unwrap();
    let leave = peer.json(0x37);
    assert_eq!(leave, json!({ "request_id": leave["request_id"] }));
    let started = Instant::now();
    pty.master.write_all(b"kept").unwrap();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(pty.unread(), 4, "the client read keys after detaching");
    let (bytes, error) = pty.finish(125);
    assert!(started.elapsed() >= Duration::from_millis(4900));
    assert!(started.elapsed() < Duration::from_secs(7));
    assert_eq!(bytes, CONSERVATIVE_RESET);
    assert!(error.contains("detach not confirmed"), "{error}");
}

/// COM-01 Monitor parity case: a confirmed Leave of the attachment detaches:
/// the notice on the restored terminal, status 0.
#[test]
fn a_confirmed_leave_detaches_with_the_notice() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    pty.master.write_all(b"\x1dd").unwrap();
    let leave = peer.json(0x37);
    peer.display(FIRST, b"release");
    peer.answer(
        &leave,
        json!({ "kind": "left", "attachment_detached": true }),
    );
    drop(peer);
    let (bytes, error) = pty.finish(0);
    assert_eq!(bytes, b"release");
    assert_eq!(
        error,
        format!("climon2: detached from session {ID}; reattach with: climon2 attach {ID}\n")
    );
}

/// COM-01 Monitor parity case (#19/#20): signals are statuses even while
/// waiting for the Open's answer, never errors.
#[test]
fn termination_before_attached_does_not_enter_raw_mode_or_report_error() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.open();
    assert_eq!(
        unsafe { libc::kill(pty.child.id() as i32, libc::SIGTERM) },
        0
    );
    let (bytes, error) = pty.finish(143);
    assert!(bytes.is_empty());
    assert_eq!(error, "");
}

/// COM-01 Monitor parity case (#19/#20): HUP/TERM/INT restore termios and
/// close the connection rather than sending policy: no Leave is sent.
#[test]
fn raw_client_signals_restore_termios_close_connection_and_return_signal_status() {
    for signal in [libc::SIGHUP, libc::SIGTERM, libc::SIGINT] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        pty.wait_raw();
        assert_eq!(unsafe { libc::kill(pty.child.id() as i32, signal) }, 0);
        let (bytes, error) = pty.finish(128 + signal);
        assert!(bytes.is_empty());
        assert!(error.is_empty());
        peer.assert_closed();
    }
}

/// COM-01 Monitor parity case (#20/#25): every failure while bridging
/// restores before one-line diagnosis; only a lost connection writes the
/// conservative reset (section 1.7.1). A daemon's fatal error, a lost
/// connection, a client panic, a repeated hello, an unknown record type and
/// a record over the fixed `metadata_record_bytes` each end with status 125.
#[test]
fn protocol_error_lost_connection_and_panic_restore_before_one_line_diagnosis() {
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
        let mut peer = Peer::accept(&listener);
        peer.attach();
        if mode != "panic" {
            peer.begin();
            pty.wait_raw();
        }
        match mode {
            "error" => peer.fatal("peer failed\nunsafe second line"),
            "lost" => peer.stream.shutdown(std::net::Shutdown::Both).unwrap(),
            "repeated-hello" => peer.send(
                1,
                &json!({ "session_id": ID, "climon2_version": VERSION, "protocol": 1 }),
            ),
            "unknown" => peer.write(&[0, 0, 0, 1, 0xff]),
            "oversized" => peer.write(&frame(0x41, &[b' '; 65_536])),
            _ => (),
        }
        let (bytes, error) = pty.finish(125);
        let reset: &[u8] = if mode == "lost" {
            CONSERVATIVE_RESET
        } else {
            b""
        };
        assert_eq!(bytes, reset, "{mode}");
        assert_eq!(error.lines().count(), 1, "{mode}: {error}");
        assert!(error.starts_with("climon2: error: "), "{mode}: {error}");
        assert!(
            error.contains(&format!("(logs: {}/logs/{ID})", home.path().display())),
            "{error}"
        );
        if mode == "lost" {
            assert!(error.contains(&format!("lost connection to session {ID}")));
        }
        if mode == "error" {
            assert!(
                error.contains("peer failed\\nunsafe second line"),
                "{error}"
            );
        }
        if mode == "panic" {
            assert!(error.contains("client panicked"));
            let log = client_log(home.path());
            assert!(log.contains("client panic"));
            assert!(!log.contains("CLIMON2_CLIENT_FAULT_PANIC"));
        }
    }
}

/// COM-01 Monitor parity case (ADR 0001): each hello identity dimension is
/// checked before raw mode and before any request; so is daemon wire
/// revision 1, whose support is never inferred from the release.
#[test]
fn hello_rejects_mismatched_session_protocol_version_and_unexpected_frames() {
    for (mode, diagnosis) in [
        ("session", "the session daemon is another session's"),
        (
            "protocol",
            "the session daemon speaks another hello protocol",
        ),
        ("version", "the session runs another release"),
        (
            "capabilities",
            "the session daemon does not advertise daemon wire revision 1",
        ),
        ("malformed", "the session daemon's hello reply is not valid"),
    ] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        let other = "11234567-89ab-4cde-8fab-0123456789ab";
        match mode {
            "session" => peer.hello_with(json!({ "session_id": other })),
            "protocol" => peer.hello_with(json!({ "protocol": 2 })),
            "version" => peer.hello_with(json!({ "climon2_version": "9.0.0" })),
            "capabilities" => peer.hello_with(json!({ "daemon_wire_v1": null })),
            _ => {
                read_frame(&mut peer.stream).unwrap();
                peer.write(&[0, 0, 0, 2, 1, b'[']);
            }
        }
        let (bytes, error) = pty.finish(125);
        assert!(bytes.is_empty());
        assert_eq!(error.lines().count(), 1);
        assert!(error.contains(diagnosis), "{mode}: {error}");
        peer.assert_closed();
    }
}

/// COM-01 Monitor parity case (ADR 0001): partial reply bytes never restart
/// the hello's two-second budget.
#[test]
fn hello_deadline_is_absolute_across_partial_reply() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    read_frame(&mut peer.stream).unwrap();
    let started = Instant::now();
    peer.write(&[0, 0]);
    thread::sleep(Duration::from_millis(1100));
    peer.write(&[0, 100, 1, b'{']);
    let (bytes, error) = pty.finish(125);
    assert!(started.elapsed() >= Duration::from_millis(1900));
    assert!(started.elapsed() < Duration::from_secs(3));
    assert!(bytes.is_empty());
    assert!(
        error.contains("the session daemon did not answer in time"),
        "{error}"
    );
}

/// COM-01 Monitor parity case (#20): fragmented frames, and changed-size-only
/// SIGWINCH forwarding: one Resize under the held authority.
#[test]
fn fragmented_frames_and_coalesced_resize_preserve_the_stream() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    let open = peer.open();
    let opened = json!({
        "request_id": open["request_id"], "outcome": "ok", "error": null,
        "result": {
            "kind": "opened", "connection_id": "00112233-4455-6677-8899-aabbccddeeff",
            "selected_capabilities": ["daemon-wire-v1"]
        }
    });
    for byte in frame(0x40, opened.to_string().as_bytes()) {
        peer.write(&[byte]);
        thread::sleep(Duration::from_millis(2));
    }
    let unavailable = json!({ "available": false, "value": null });
    peer.send(
        0x41,
        &json!({
            "title": unavailable, "status": unavailable, "progress": unavailable,
            "command_result": null
        }),
    );
    peer.begin();
    pty.wait_raw();
    pty.resize_signalled(132, 44, 10);
    let resize = peer.json(0x32);
    let mut expected = ids(FIRST);
    expected["request_id"] = resize["request_id"].clone();
    expected["geometry"] = json!({ "cols": 132, "rows": 44 });
    assert_eq!(resize, expected);
    peer.answer(&resize, json!({ "kind": "resize_accepted" }));
    pty.resize_signalled(132, 44, 10);
    peer.stream
        .set_read_timeout(Some(Duration::from_millis(150)))
        .unwrap();
    let error = peer.stream.read(&mut [0]).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    peer.stream.set_read_timeout(Some(BOUND)).unwrap();
    let output = b"\xff\0\x1b[6nfinal";
    for byte in display_frame(FIRST, output) {
        peer.write(&[byte]);
    }
    peer.exit(0);
    let (bytes, error) = pty.finish(0);
    assert_eq!(bytes, output);
    assert!(error.is_empty());
}

/// COM-01 Monitor parity case (#20): the caller's terminal file-status flags
/// (O_NONBLOCK) are restored on normal, error and signal exits when stdin and
/// stdout are separate open-file descriptions; the shared-description case is
/// checked by every other `Pty::finish`.
#[test]
fn separate_stdin_and_stdout_descriptions_both_get_their_flags_back() {
    for ending in ["exit", "error", "signal"] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::separate_output(home.path(), &metadata);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        pty.wait_raw();
        let expected = match ending {
            "exit" => {
                peer.exit(7);
                7
            }
            "error" => {
                peer.fatal("peer failed");
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

/// COM-01 Monitor parity case (#20): a read-only stdin whose file-status
/// flags are all clear and a write-only stdout still bridge both ways (input
/// is read from stdin, never stdout) and get their flags back.
#[test]
fn one_way_descriptors_are_bridged_and_restored() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::one_way(home.path(), &metadata);
    assert_eq!(pty.status_flags[0], 0, "stdin must have no status flags");
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    pty.master.write_all(b"in").unwrap();
    assert_eq!(peer.input(), b"in");
    peer.display(FIRST, b"out");
    peer.exit(0);
    let (bytes, error) = pty.finish(0);
    assert_eq!(bytes, b"out");
    assert!(error.is_empty(), "{error}");
}

/// COM-01 Monitor parity case (#20): input that another reader of the
/// terminal consumed between the client's poll and its read is not a
/// hang-up; later input still flows.
#[cfg(all(target_os = "linux", target_env = "gnu"))]
#[test]
fn input_consumed_by_another_reader_after_poll_is_not_a_hang_up() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    let terminal = fs::read_link(format!("/proc/self/fd/{}", pty.slave.as_raw_fd())).unwrap();
    let tracer = climon2_test_term::ptrace::Tracer::seize(pty.child.id(), Duration::from_secs(5));
    pty.master.write_all(b"taken").unwrap();
    // Stop at the client's terminal read: read(<terminal>, _, 32768).
    loop {
        let call = tracer.next_entry();
        if call.number == libc::SYS_read as u64
            && call.arguments[2] == 32768
            && tracer.descriptor(call.arguments[0]).as_ref() == Some(&terminal)
        {
            break;
        }
    }
    // The slave shares the client's non-blocking stdin description.
    let mut taken = Vec::new();
    let mut buffer = [0; 16];
    while taken.len() < 5 {
        wait_ready(&pty.slave, libc::POLLIN, 5);
        if let Ok(count) = pty.slave.read(&mut buffer) {
            taken.extend_from_slice(&buffer[..count]);
        }
    }
    assert_eq!(taken, b"taken");
    assert_eq!(tracer.finish_syscall(), -i64::from(libc::EAGAIN));
    tracer.detach();
    pty.master.write_all(b"live").unwrap();
    assert_eq!(peer.input(), b"live");
    peer.exit(0);
    let (bytes, error) = pty.finish(0);
    assert!(bytes.is_empty(), "{bytes:?}");
    assert!(error.is_empty(), "{error}");
}

/// COM-01 Monitor parity case (#20): blocked stdout backpressures the
/// session through the library, which stops reading once its fixed event
/// allowance is full, and the command's status cannot overtake output: 4 MiB
/// of Display, far beyond the allowance, the client's read-ahead and the
/// socket's buffers, all arrive, then the status.
#[test]
fn bounded_output_backpressure_drains_every_byte_before_exit() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    let producer = thread::spawn(move || {
        for _ in 0..128 {
            peer.display(FIRST, &[b'X'; 32768]);
        }
        peer.exit(51);
    });
    thread::sleep(Duration::from_millis(200));
    assert!(
        !producer.is_finished(),
        "blocked terminal did not backpressure the producer"
    );
    let (bytes, error) = pty.finish(51);
    producer.join().unwrap();
    assert_eq!(bytes.len(), 4 * 1_048_576);
    assert!(bytes.iter().all(|byte| *byte == b'X'));
    assert!(error.is_empty());
}

/// COM-01 Monitor parity case (#20): losing stdin is a silent HUP, without
/// sending a Leave: the connection just closes.
#[test]
fn terminal_eof_or_eio_is_silent_status_129_and_closes_connection() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    #[cfg(target_os = "linux")]
    let unrelated = {
        // Reproduce another thread forking before openpty's descriptors get CLOEXEC.
        assert_eq!(
            unsafe { libc::fcntl(pty.master.as_raw_fd(), libc::F_SETFD, 0) },
            0
        );
        let (home, metadata, listener) = setup();
        let other = Pty::spawn(home.path(), &metadata, None);
        assert_eq!(
            unsafe { libc::fcntl(pty.master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            0
        );
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        other.wait_raw();
        (home, other, peer)
    };
    // Linux wakes the client before it finishes hanging the terminal up; a stopped
    // client sees the hang-up only once it is complete.
    pty.while_stopped(|pty| {
        drop(std::mem::replace(
            &mut pty.master,
            File::open("/dev/null").unwrap(),
        ));
    });
    let deadline = Instant::now() + BOUND;
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
    peer.assert_closed();
    #[cfg(target_os = "linux")]
    {
        let (_home, mut other, peer) = unrelated;
        peer.exit(0);
        other.finish(0);
    }
}

/// COM-01 Monitor parity case (#20): a terminal read failing with EIO (lost
/// terminal) is the same silent HUP as EOF.
#[test]
fn terminal_eio_is_silent_status_129_and_restores_termios() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::background(home.path(), &metadata);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    pty.master.write_all(b"x").unwrap();
    let deadline = Instant::now() + BOUND;
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
    peer.assert_closed();
}

/// COM-01 Monitor parity case (ADR 0001/#19): a refused Open, or a record
/// before its answer, fails before raw mode with one diagnostic: the
/// daemon's own, or the library's.
#[test]
fn attach_refusal_or_unexpected_reply_fails_without_raw_mode() {
    for (refused, diagnosis) in [
        (true, "session refused attach"),
        (
            false,
            "the session daemon sent a record out of order or for an unselected family",
        ),
    ] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        let open = peer.open();
        if refused {
            let error = json!({ "code": "session_terminating", "message": diagnosis });
            peer.respond(&open["request_id"], "rejected", Value::Null, error);
        } else {
            peer.complete(FIRST);
        }
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

/// COM-01 Monitor parity case (#19/#20): while terminal output is blocked, a
/// session that hung up must not spin the client: it sleeps in poll, then
/// delivers every received byte once stdout drains, and finally honours the
/// command's status or reports the lost connection (125).
#[test]
fn hung_up_session_during_blocked_output_sleeps_then_delivers_everything() {
    for exit in [None, Some(51)] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        pty.wait_raw();
        peer.display(FIRST, &[b'X'; 60_000]);
        if let Some(code) = exit {
            peer.final_exit(code);
        }
        drop(peer);
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
        let mut expected = vec![b'X'; 60_000];
        if exit.is_none() {
            expected.extend(CONSERVATIVE_RESET);
        }
        assert!(bytes == expected, "{:?}", &bytes[59_990.min(bytes.len())..]);
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

/// COM-01 Monitor parity case (#20): an attached client with nothing to do
/// sleeps in poll, and so does one waiting for its Open's answer.
#[test]
fn an_idle_client_sleeps() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    let open = peer.open();
    let idle = || {
        let before = cpu_time(pty.child.id());
        thread::sleep(Duration::from_secs(1));
        cpu_time(pty.child.id()) - before
    };
    let used = idle();
    assert!(
        used < Duration::from_millis(200),
        "a client awaiting its Open used {used:?} CPU in 1 s"
    );
    peer.opened(&open);
    peer.begin();
    pty.wait_raw();
    let used = idle();
    assert!(
        used < Duration::from_millis(200),
        "idle client used {used:?} CPU in 1 s"
    );
    peer.exit(0);
    let (bytes, error) = pty.finish(0);
    assert!(bytes.is_empty(), "{bytes:?}");
    assert!(error.is_empty(), "{error}");
}

/// COM-01 Monitor parity case (#19/#20): a session that hangs up while
/// stdout is blocked and input is backpressured loses none of the output
/// already sent, and the client reads no more keys: what it can no longer
/// send stays for the next program.
#[test]
fn hang_up_behind_blocked_output_and_input_keeps_output_and_leaves_keys() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    peer.display(FIRST, &[b'X'; 60_000]);
    peer.display(FIRST, b"later");
    let typing = Typing::start(&pty.master);
    wait_until_backpressured(&peer.stream, &typing);
    drop(peer);
    thread::sleep(Duration::from_millis(300));
    let before = typing.accepted();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(
        typing.accepted(),
        before,
        "the client read keys after the session hung up"
    );
    drop(typing);
    let (bytes, error) = pty.finish(125);
    let mut expected = vec![b'X'; 60_000];
    expected.extend(b"later");
    expected.extend(CONSERVATIVE_RESET);
    assert!(bytes == expected, "{:?}", &bytes[59_990.min(bytes.len())..]);
    assert!(
        error.contains(&format!("lost connection to session {ID}")),
        "{error}"
    );
}

/// Types into the terminal from another thread until dropped.
struct Typing {
    accepted: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Typing {
    fn start(master: &File) -> Self {
        let mut writer = master.try_clone().unwrap();
        let accepted = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let thread = thread::spawn({
            let (accepted, stop) = (accepted.clone(), stop.clone());
            move || {
                while !stop.load(Ordering::SeqCst) {
                    match writer.write(&[b'k'; 4096]) {
                        Ok(count) => {
                            accepted.fetch_add(count, Ordering::SeqCst);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            wait_ready(&writer, libc::POLLOUT, 5);
                        }
                        Err(error) => panic!("{error}"),
                    }
                }
            }
        });
        Self {
            accepted,
            stop,
            thread: Some(thread),
        }
    }

    /// Bytes the terminal has taken so far.
    fn accepted(&self) -> usize {
        self.accepted.load(Ordering::SeqCst)
    }
}

/// Waits until the client's input fills the unread session socket and the
/// terminal stops taking keys, so its next frame is waiting to be sent.
fn wait_until_backpressured(stream: &std::os::unix::net::UnixStream, typing: &Typing) {
    let queued = || {
        let mut bytes: libc::c_int = 0;
        assert_eq!(
            unsafe { libc::ioctl(stream.as_raw_fd(), libc::FIONREAD, &mut bytes) },
            0
        );
        bytes
    };
    let deadline = Instant::now() + BOUND;
    let mut last = (queued(), typing.accepted());
    loop {
        thread::sleep(Duration::from_millis(300));
        let now = (queued(), typing.accepted());
        if now == last && now.0 > 0 {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the client never stopped reading"
        );
        last = now;
    }
}

impl Drop for Typing {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

/// Waits for the client without ever reading its terminal, so stdout stays blocked.
fn exit_while_blocked(pty: &mut Pty, bound: Duration) -> (i32, String) {
    let deadline = Instant::now() + bound;
    let status = loop {
        if let Some(status) = pty.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "client did not exit while blocked"
        );
        thread::sleep(Duration::from_millis(5));
    };
    pty.assert_restored();
    let mut error = String::new();
    pty.child
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut error)
        .unwrap();
    (status.code().unwrap(), error)
}

/// COM-01 Monitor parity case (section 6.3 blocked stdout): output pending,
/// more Display and the command's status queued, then the session
/// connection closes. The client gives up within the closing deadline that
/// the close starts, restores the terminal and reports the result as
/// unconfirmed.
#[test]
fn lost_session_while_stdout_is_blocked_ends_within_the_closing_deadline() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    peer.display(FIRST, &[b'X'; 60_000]);
    peer.display(FIRST, b"later");
    peer.final_exit(0);
    thread::sleep(Duration::from_millis(200));
    let closed = Instant::now();
    drop(peer);
    let (code, error) = exit_while_blocked(&mut pty, Duration::from_secs(7));
    assert!(closed.elapsed() >= Duration::from_millis(4900));
    assert_eq!(code, 125);
    assert!(error.contains("session result not confirmed"), "{error}");
}

/// COM-01 Monitor parity case (section 6.3 pending detach with blocked
/// stdout): without the Leave's answer the client reports the unconfirmed
/// detach within the bound.
#[test]
fn pending_detach_while_stdout_is_blocked_ends_within_the_bound() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    peer.display(FIRST, &[b'X'; 60_000]);
    thread::sleep(Duration::from_millis(200));
    pty.master.write_all(b"\x1dd").unwrap();
    peer.json(0x37);
    let (code, error) = exit_while_blocked(&mut pty, Duration::from_secs(7));
    assert_eq!(code, 125);
    assert!(error.contains("detach not confirmed"), "{error}");
}

/// COM-01 Monitor parity case (section 1.7.1): the end of the session's
/// stream starts the closing deadline. When stdout is already blocked, the
/// placeholder's end can never be written; the client gives up within the
/// bound.
#[test]
fn exit_read_while_stdout_is_blocked_ends_within_the_closing_deadline() {
    let (_home, mut pty, peer) = observing();
    fill_output_queue(&pty.slave);
    let parsed = Instant::now();
    peer.exit(5);
    let (code, error) = exit_while_blocked(&mut pty, Duration::from_secs(7));
    assert!(parsed.elapsed() >= Duration::from_millis(4900));
    assert_eq!(code, 125);
    assert!(error.contains("session result not confirmed"), "{error}");
}

/// COM-01 Monitor parity case (#19): a disconnect before the Open's answer
/// identifies the session, before entering raw mode.
#[test]
fn disconnect_before_attached_names_the_session() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.open();
    peer.stream.shutdown(std::net::Shutdown::Both).unwrap();
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert!(
        error.contains(&format!("lost connection to session {ID}")),
        "{error}"
    );
}

/// COM-01 Monitor parity case (ADR 0001): an invalid length prefix (zero, or
/// beyond the fixed `wire_frame_bytes`) fails without waiting for a type or
/// body byte.
#[test]
fn invalid_length_prefix_closes_immediately_without_a_type_byte() {
    for prefix in [[0, 0, 0, 0], [0, 16, 0, 1]] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        pty.wait_raw();
        peer.write(&prefix);
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
        assert!(
            error.contains("the session daemon's traffic breaks daemon wire revision 1"),
            "{error}"
        );
    }
}

/// COM-01 Monitor parity case (#20): while the session answers nothing, the
/// library admits at most the fixed `operations` of Input, so the terminal
/// stops being read; once answered, every byte is forwarded, in Inputs
/// within `chunk_bytes`.
#[test]
fn input_backpressure_is_bounded_and_lossless() {
    const TOTAL: usize = 2 * 1_048_576;
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    let mut writer = pty.master.try_clone().unwrap();
    let producer = thread::spawn(move || {
        let payload = vec![0xff; TOTAL];
        let mut position = 0;
        let deadline = Instant::now() + BOUND;
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
        "unanswered Input did not backpressure stdin"
    );
    let mut count = 0;
    while count < TOTAL {
        let bytes = peer.input();
        assert!(bytes.len() <= 65_536);
        assert!(bytes.iter().all(|byte| *byte == 0xff));
        count += bytes.len();
    }
    assert_eq!(count, TOTAL);
    producer.join().unwrap();
    peer.exit(0);
    let (bytes, error) = pty.finish(0);
    assert!(bytes.is_empty());
    assert!(error.is_empty());
}

/// #17/#25: connection errors are one-line failures with only written log
/// paths.
#[test]
fn nonexistent_socket_and_bad_metadata_fail_without_touching_termios() {
    let (home, metadata, listener) = setup();
    drop(listener);
    fs::remove_file(home.path().join("peer.sock")).unwrap();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert_eq!(error.lines().count(), 1);
    assert!(
        error.contains("the session daemon cannot be reached"),
        "{error}"
    );
    assert!(error.contains("(logs: "));
    fs::write(&metadata, b"{").unwrap();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert_eq!(error.lines().count(), 1);
    assert!(!error.contains("(logs: "));
}

/// LIB-02 executable case, attach mode: a missing library fails with 125
/// and the loader's diagnostic before raw mode and before anything connects.
#[test]
fn a_missing_library_fails_attach_before_any_connection() {
    let (home, metadata, listener) = setup();
    let folder = installed_client(home.path());
    let library = folder
        .parent()
        .unwrap()
        .join(client_library_path().file_name().unwrap());
    fs::remove_file(&library).unwrap();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert_eq!(error.lines().count(), 1);
    assert!(
        error.contains(&format!(
            "cannot load the client library {}: ",
            library.display()
        )),
        "{error}"
    );
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// #25/#28: client filtering is environment-only and defaults include DEBUG.
#[test]
fn log_filter_reaches_client_through_environment_without_cli_flags() {
    for (filter, debug) in [(None, true), (Some("warn"), false)] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::configured(home.path(), &metadata, None, filter);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        peer.exit(0);
        let (bytes, error) = pty.finish(0);
        assert!(bytes.is_empty());
        assert!(error.is_empty());
        let log = client_log(home.path());
        assert_eq!(log.contains("DEBUG"), debug, "{log}");
        assert!(!log.contains("CLIMON2_LOG"));
    }
}

/// Ordinary attach gives the client library a log file of its own, as kill
/// mode does: in the session-log directory beside the client's own log and
/// named like it, `library-<UTC second>-<pid>.log`, under the client's
/// `CLIMON2_LOG` filter: its info record of the instance's creation is
/// there by default and filtered out under `warn`.
#[test]
fn attach_gives_the_library_its_own_log_beside_the_clients_session_log() {
    for (filter, created) in [(None, true), (Some("warn"), false)] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::configured(home.path(), &metadata, None, filter);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        peer.exit(0);
        assert_eq!(pty.finish(0), (Vec::new(), String::new()));
        let suffix = format!("-{}.log", pty.child.id());
        let directory = home.path().join(format!("logs/{ID}"));
        let names: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(&suffix))
            .collect();
        let named = |role: &str| {
            let found: Vec<_> = names.iter().filter(|name| name.starts_with(role)).collect();
            assert_eq!(found.len(), 1, "{role}: {names:?}");
            fs::read_to_string(directory.join(found[0])).unwrap()
        };
        assert!(named("client-").contains("role=client"));
        let library = named("library-");
        assert_eq!(
            library.contains("client instance created"),
            created,
            "{filter:?}: {library}"
        );
    }
}

/// The client logs in the session-log directory, keyed by file name.
fn client_logs(home: &Path) -> Vec<(String, String)> {
    let directory = home.join(format!("logs/{ID}"));
    let mut logs: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.file_name().to_str().unwrap().starts_with("client"))
        .map(|entry| {
            let name = entry.file_name().into_string().unwrap();
            (name, fs::read_to_string(entry.path()).unwrap())
        })
        .collect();
    logs.sort();
    logs
}

/// The contents of the only client log.
fn client_log(home: &Path) -> String {
    let mut logs = client_logs(home);
    assert_eq!(logs.len(), 1, "{logs:?}");
    logs.remove(0).1
}

/// #25: each client invocation writes its own private log, so a second client on the
/// same session reaches hello while the first is still attached.
#[test]
fn concurrent_clients_on_one_session_each_write_their_own_log() {
    let (home, metadata, listener) = setup();
    let mut first = Pty::spawn(home.path(), &metadata, None);
    let mut first_peer = Peer::accept(&listener);
    first_peer.attach();
    first_peer.begin();
    first.wait_raw();
    let first_log = client_log(home.path());
    let mut second = Pty::spawn(home.path(), &metadata, None);
    let mut second_peer = Peer::accept(&listener);
    second_peer.attach();
    second_peer.begin();
    second.wait_raw();
    first_peer.exit(0);
    second_peer.exit(0);
    for pty in [&mut first, &mut second] {
        assert_eq!(pty.finish(0), (Vec::new(), String::new()));
    }
    let logs = client_logs(home.path());
    assert_eq!(logs.len(), 2, "{logs:?}");
    assert!(logs.iter().any(|(_, text)| text.starts_with(&first_log)));
    for pid in [first.child.id(), second.child.id()] {
        let (name, text) = logs
            .iter()
            .find(|(name, _)| name.ends_with(&format!("-{pid}.log")))
            .expect("client log named by pid");
        let stamp = &name["client-".len()..name.len() - format!("-{pid}.log").len()];
        assert_eq!(stamp.len(), "20261028T000000Z".len(), "{name}");
        assert!(text.contains(&format!("role=client pid={pid} ")), "{text}");
        let path = home.path().join(format!("logs/{ID}/{name}"));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

/// Stop the driver draining between writes, then fill even its last byte.
fn fill_output_queue(terminal: &File) {
    assert_eq!(
        unsafe { libc::tcflow(terminal.as_raw_fd(), libc::TCOOFF) },
        0
    );
    let mut writer = reopen(terminal);
    let flags = status_flags(&writer);
    assert_eq!(
        unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
        0
    );
    loop {
        match writer.write(b"Q") {
            Ok(0) => panic!("terminal write made no progress"),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return,
            Err(error) => panic!("{error}"),
        }
    }
}

/// CPU time (user + system) as reported by ps, e.g. `0:01.25`.
#[cfg(target_os = "macos")]
fn cpu_time(pid: u32) -> Duration {
    let output = climon2_test_term::process_command("/bin/ps")
        .args(["-o", "time=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8(output.stdout).unwrap();
    let (minutes, seconds) = text.trim().rsplit_once(':').unwrap();
    let minutes: u64 = minutes.rsplit(':').next().unwrap().parse().unwrap();
    Duration::from_secs(minutes * 60) + Duration::from_secs_f64(seconds.parse().unwrap())
}

#[cfg(target_os = "linux")]
fn cpu_time(pid: u32) -> Duration {
    // procps's time= truncates to seconds, which would hide a busy loop.
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
    let (_, fields) = stat.rsplit_once(')').unwrap();
    let fields: Vec<_> = fields.split_whitespace().collect();
    let user: u64 = fields[11].parse().unwrap();
    let system: u64 = fields[12].parse().unwrap();
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    assert!(ticks > 0);
    Duration::from_secs_f64((user + system) as f64 / ticks as f64)
}

/// The fixed `cm1_limits` table the client README publishes, field by field
/// in its row order.
fn published_limits() -> Vec<(String, u64)> {
    let readme = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).unwrap();
    let table = readme
        .split("| Field | Value |\n|---|---|\n")
        .nth(1)
        .expect("the fixed limits table");
    let rows = table.lines().take_while(|line| line.starts_with('|'));
    rows.flat_map(|row| {
        let cells: Vec<_> = row.split('|').map(str::trim).collect();
        let names = cells[1].split(" / ").map(|name| name.trim_matches('`'));
        let values = cells[2].split(" / ").map(|value| value.replace(',', ""));
        names
            .zip(values)
            .map(|(name, value)| (name.to_owned(), value.parse().unwrap()))
            .collect::<Vec<_>>()
    })
    .collect()
}

/// Derived scenario (spec #73, Client executable: fixed limits table): the
/// executable passes exactly the `cm1_limits` table its README publishes. A stand-in library beside it
/// records the limits its create receives, in the header's field order, and
/// refuses, so the attach fails before any connection with that refusal.
#[test]
fn attach_passes_exactly_the_published_limits_table() {
    let (home, metadata, listener) = setup();
    let folder = installed_client(home.path());
    let library = folder
        .parent()
        .unwrap()
        .join(client_library_path().file_name().unwrap());
    let recorded = home.path().join("limits");
    let record = format!("-DRECORD_STARTUP=\"{}\"", recorded.display());
    climon2_fixtures::stand_in::compile(&library, VERSION, &[&record]).unwrap();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let (bytes, error) = pty.finish(125);
    assert!(bytes.is_empty());
    assert!(
        error.contains("cannot start the client library: the stand-in recorded its startup"),
        "{error}"
    );
    let limits: Vec<(String, u64)> = fs::read_to_string(recorded)
        .unwrap()
        .lines()
        .map(|line| {
            let (name, value) = line.split_once(' ').unwrap();
            (name.to_owned(), value.parse().unwrap())
        })
        .collect();
    let mut published = published_limits();
    published.sort();
    let mut passed = limits.clone();
    passed.sort();
    assert_eq!(passed, published);
    assert_eq!(limits.len(), 27, "{limits:?}");
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

/// COM-01 Monitor parity case: a command that ended before this first
/// attachment is known from the Initial metadata; the client still shows its
/// final screen, confirms it Ready, and ends with its status when the session
/// ends, with no Command final.
#[test]
fn a_result_known_at_the_open_ends_the_client_with_that_status() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    let open = peer.open();
    let opened = json!({
        "kind": "opened", "connection_id": "00112233-4455-6677-8899-aabbccddeeff",
        "selected_capabilities": ["daemon-wire-v1"]
    });
    peer.answer(&open, opened);
    let unavailable = json!({ "available": false, "value": null });
    let metadata = json!({
        "title": unavailable, "status": unavailable, "progress": unavailable,
        "command_result": { "kind": "exit", "code": 3 }
    });
    peer.send(0x41, &metadata);
    peer.grant(FIRST, "attach");
    peer.display(FIRST, b"final screen");
    peer.complete(FIRST);
    peer.ready(FIRST);
    peer.revoke(FIRST, "session_ending");
    drop(peer);
    let (bytes, error) = pty.finish(3);
    assert_eq!(bytes, b"final screen");
    assert_eq!(error, "");
}

/// COM-01 Monitor parity case: a replacement restoration within the period
/// (a Restoration begin without a grant) is a new restoration: keys typed
/// during it are drained and dropped, and Ready and later Input name it.
#[test]
fn a_replacement_restoration_drops_typed_keys_until_ready() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    let replacement = (1, 2);
    let mut begin = ids(replacement);
    begin["geometry"] = json!({ "cols": 97, "rows": 31 });
    peer.send(0x48, &begin);
    peer.display(replacement, b"redrawn");
    pty.read_until(b"redrawn");
    pty.master.write_all(b"stale").unwrap();
    pty.wait_consumed();
    peer.complete(replacement);
    peer.ready(replacement);
    pty.master.write_all(b"live").unwrap();
    assert_eq!(peer.input_under(replacement), b"live");
    peer.exit(0);
    pty.finish(0);
}

/// Exit statuses are unchanged: a command killed by a signal ends the client
/// with 128 plus the signal's number, as today; a number that overflows that
/// status is climon2's own 125.
#[test]
fn a_signalled_command_ends_the_client_with_128_plus_the_signal() {
    for (number, status) in [(9_u32, 137), (u32::MAX, 125)] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        pty.wait_raw();
        let result = json!({ "kind": "signal", "number": number, "name": "SIGKILL" });
        peer.send(0x4c, &json!({ "command_result": result, "status": null }));
        drop(peer);
        assert_eq!(pty.finish(status), (Vec::new(), String::new()));
    }
}

/// COM-01 Monitor parity case: a session that closes inside a frame is a
/// lost connection: the conservative reset and the session named.
#[test]
fn a_close_inside_a_frame_is_a_lost_connection() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    peer.write(&display_frame(FIRST, b"partial")[..10]);
    drop(peer);
    let (bytes, error) = pty.finish(125);
    assert_eq!(bytes, CONSERVATIVE_RESET);
    assert!(
        error.contains(&format!("lost connection to session {ID}")),
        "{error}"
    );
}

/// The client reads ahead of a blocked terminal at most 65,536 bytes of
/// output: with 65,535 waiting it still sees the session end and gives up
/// within the closing deadline; with 65,536 waiting it reads nothing more,
/// sleeping, until the terminal drains, and then ends with the status.
#[test]
fn output_read_ahead_of_a_blocked_terminal_is_bounded() {
    for waiting in [65_535, 65_536] {
        let (home, metadata, listener) = setup();
        let mut pty = Pty::spawn(home.path(), &metadata, None);
        let mut peer = Peer::accept(&listener);
        peer.attach();
        peer.begin();
        pty.wait_raw();
        fill_output_queue(&pty.slave);
        peer.display(FIRST, &vec![b'X'; waiting]);
        peer.exit(0);
        let ended = Instant::now();
        if waiting < 65_536 {
            let (code, error) = exit_while_blocked(&mut pty, Duration::from_secs(7));
            assert!(ended.elapsed() >= Duration::from_millis(4900));
            assert_eq!(code, 125);
            assert!(error.contains("session result not confirmed"), "{error}");
            continue;
        }
        let before = cpu_time(pty.child.id());
        thread::sleep(Duration::from_secs(6));
        assert!(
            cpu_time(pty.child.id()) - before < Duration::from_millis(300),
            "the client spun while its output was blocked"
        );
        assert_eq!(pty.child.try_wait().unwrap(), None, "the client read ahead");
        assert_eq!(
            unsafe { libc::tcflow(pty.slave.as_raw_fd(), libc::TCOON) },
            0
        );
        let (bytes, error) = pty.finish(0);
        assert!(bytes.ends_with(&vec![b'X'; waiting]));
        assert_eq!(error, "");
    }
}

/// COM-01 Monitor parity case: an observer can detach too; once it asked
/// to, a resize no longer redraws the placeholder, and the confirmed Leave
/// ends the placeholder below itself and detaches.
#[test]
fn an_observer_detaching_ignores_resizes_and_ends_the_placeholder() {
    let (_home, mut pty, mut peer) = observing();
    pty.master.write_all(b"\x1dd").unwrap();
    let leave = peer.json(0x37);
    pty.resize(132, 44);
    thread::sleep(Duration::from_millis(300));
    peer.answer(
        &leave,
        json!({ "kind": "left", "attachment_detached": true }),
    );
    drop(peer);
    let (bytes, error) = pty.finish(0);
    assert_eq!(bytes, b"\x1b[31;1H\r\n");
    assert!(
        error.starts_with("climon2: detached from session"),
        "{error}"
    );
}

/// COM-01 Monitor parity case: Ready waits until the restoration has been
/// written to the terminal, not only received.
#[test]
fn ready_waits_until_the_restoration_is_written() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    pty.wait_raw();
    fill_output_queue(&pty.slave);
    peer.grant(FIRST, "attach");
    peer.display(FIRST, b"restored-screen");
    peer.complete(FIRST);
    peer.stream
        .set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let error = peer.stream.read(&mut [0]).unwrap_err();
    assert_eq!(
        error.kind(),
        std::io::ErrorKind::WouldBlock,
        "Ready came first"
    );
    peer.stream.set_read_timeout(Some(BOUND)).unwrap();
    assert_eq!(
        unsafe { libc::tcflow(pty.slave.as_raw_fd(), libc::TCOON) },
        0
    );
    assert!(
        pty.read_until(b"restored-screen")
            .ends_with(b"restored-screen")
    );
    peer.ready(FIRST);
    peer.exit(0);
    pty.finish(0);
}

/// COM-01 Monitor parity case (section 6.3): a session that hangs up while
/// more output than the client reads ahead waits behind a blocked terminal,
/// and while unanswered input holds the client's next command, is still
/// noticed: the client gives up within the closing deadline.
#[test]
fn a_hang_up_behind_blocked_output_and_held_input_ends_within_the_closing_deadline() {
    let (home, metadata, listener) = setup();
    let mut pty = Pty::spawn(home.path(), &metadata, None);
    let mut peer = Peer::accept(&listener);
    peer.attach();
    peer.begin();
    pty.wait_raw();
    fill_output_queue(&pty.slave);
    peer.display(FIRST, &vec![b'X'; 4 * 65_536]);
    let typing = Typing::start(&pty.master);
    wait_until_backpressured(&peer.stream, &typing);
    let closed = Instant::now();
    drop(peer);
    let (code, error) = exit_while_blocked(&mut pty, Duration::from_secs(7));
    assert!(closed.elapsed() >= Duration::from_millis(4900));
    drop(typing);
    assert_eq!(code, 125);
    assert!(error.contains("session result not confirmed"), "{error}");
}
