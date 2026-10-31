//! PROTOTYPE session daemon: owns a real PTY, speaks the frozen hello, and in `new`
//! mode holds one nesting edge per active attachment until that connection closes.
//! `legacy` mode answers with the shipped first-release hello and frame parser.
use climon2_core::protocol::{ClientMessage, HelloReply, ServerMessage, read_frame, write_frame};
use nesting_proof_prototype::{runtime, sys, wire};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{fs::PermissionsExt, net::{UnixListener, UnixStream}, process::CommandExt},
    },
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
};

const VERSION: &str = "0.1.0";

#[derive(Default)]
struct State {
    next: u64,
    attachments: BTreeMap<u64, (u32, Vec<String>)>,
    /// Control history: the most recent still-attached entry is the controller.
    history: Vec<u64>,
}

impl State {
    fn controller(&self) -> Option<u32> {
        self.history.iter().rev().find_map(|id| self.attachments.get(id).map(|(pid, _)| *pid))
    }

    fn hello(&self, sid: &str) -> Value {
        let mut incoming: Vec<&String> = self.attachments.values().flat_map(|(_, e)| e).collect();
        incoming.sort();
        incoming.dedup();
        json!({
            "session_id": sid,
            "climon2_version": VERSION,
            "protocol": 1,
            "nesting": { "incoming": incoming },
            "state": { "attachments": self.attachments.len(), "controlled": self.controller().is_some() },
            "prototype_debug": { "controller_pid": self.controller() },
        })
    }
}

struct Args {
    home: PathBuf,
    legacy: bool,
    ready: PathBuf,
    command: Vec<String>,
}

fn args() -> Args {
    let mut raw = std::env::args().skip(1);
    let (mut home, mut legacy, mut ready, mut command) = (None, false, None, Vec::new());
    while let Some(arg) = raw.next() {
        match arg.as_str() {
            "--home" => home = raw.next().map(PathBuf::from),
            "--mode" => legacy = raw.next().as_deref() == Some("legacy"),
            "--ready" => ready = raw.next().map(PathBuf::from),
            "--" => command = raw.by_ref().collect(),
            other => panic!("unknown argument {other}"),
        }
    }
    Args { home: home.expect("--home"), legacy, ready: ready.expect("--ready"), command }
}

fn main() {
    let args = args();
    let dir = runtime::runtime_dir(&args.home).expect("runtime dir");
    fs::create_dir_all(&dir).unwrap();
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
    let sid = uuid::Uuid::new_v4().to_string();
    let socket = dir.join(format!("{sid}.sock"));
    let listener = UnixListener::bind(&socket).expect("bind");
    let master = spawn_on_pty(&args.command, &sid);
    let metadata = write_metadata(&args.home, &sid, &socket, &args.command);
    let state = Arc::new(Mutex::new(State::default()));
    let (legacy, accept_sid) = (args.legacy, sid.clone());
    thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (state, sid) = (state.clone(), accept_sid.clone());
            thread::spawn(move || {
                let _ = if legacy { legacy_connection(stream, &sid) } else { connection(stream, &sid, &state) };
            });
        }
    });
    publish(&args.ready, &format!("{sid} {}\n", std::process::id()));
    let mut master = fs::File::from(master);
    let mut sink = [0u8; 4096];
    let mut log = std::env::var_os("NEST_PTY_LOG").map(|p| fs::File::create(p).unwrap());
    // The session ends when its terminal hangs up (the command and its jobs left).
    while let Ok(n @ 1..) = master.read(&mut sink) {
        if let Some(log) = log.as_mut() {
            let _ = std::io::Write::write_all(log, &sink[..n]);
        }
    }
    let _ = fs::remove_file(&socket);
    let _ = fs::remove_file(&metadata);
}

fn spawn_on_pty(command: &[String], sid: &str) -> OwnedFd {
    let (mut master, mut slave) = (0, 0);
    let mut size = libc::winsize { ws_row: 24, ws_col: 80, ws_xpixel: 0, ws_ypixel: 0 };
    let result = unsafe {
        libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &mut size)
    };
    assert_eq!(result, 0, "openpty");
    let (master, slave) = unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
    for fd in [&master, &slave] {
        unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) };
    }
    let mut child = Command::new(&command[0]);
    child
        .args(&command[1..])
        .env("CLIMON2_SESSION_ID", sid)
        .env("CLIMON2_SESSION_DEPTH", "1")
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave.try_clone().unwrap()));
    unsafe {
        child.pre_exec(|| {
            if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    // The daemon stays the leader's parent, exactly as climon2-daemon does.
    let process = child.spawn().expect("spawn command");
    std::mem::forget(process);
    master
}

fn write_metadata(home: &Path, sid: &str, socket: &Path, command: &[String]) -> PathBuf {
    let sessions = home.join("sessions");
    fs::create_dir_all(&sessions).unwrap();
    let start = sys::proc_info(std::process::id()).map(|i| i.start_us).unwrap_or(0);
    let body = json!({
        "format_version": 1,
        "session_id": sid,
        "socket_path": socket,
        "daemon_pid": std::process::id(),
        "daemon_start_time": start,
        "command": command[0],
        "args": &command[1..],
        "cwd": std::env::current_dir().unwrap(),
        "depth": 1,
        "created_at": "2026-10-31T00:00:00.000Z",
        "climon2_version": VERSION,
    });
    let path = sessions.join(format!("{sid}.pty"));
    publish(&path, &body.to_string());
    path
}

fn publish(path: &Path, contents: &str) {
    let staging = path.with_extension("staging");
    fs::write(&staging, contents).unwrap();
    fs::rename(&staging, path).unwrap();
}

fn same_user(stream: &UnixStream) -> std::io::Result<()> {
    if sys::peer_uid(stream.as_raw_fd())? != sys::euid() {
        return Err(std::io::Error::other("peer UID mismatch"));
    }
    Ok(())
}

fn connection(mut stream: UnixStream, sid: &str, state: &Mutex<State>) -> std::io::Result<()> {
    same_user(&stream)?;
    stream.set_read_timeout(Some(runtime::HELLO_TIMEOUT))?;
    match wire::read(&mut stream)? {
        (wire::HELLO, body) if body["protocol"] == 1 => {}
        _ => return Ok(()),
    }
    let reply = state.lock().unwrap().hello(sid);
    wire::write(&mut stream, wire::HELLO, &reply)?;
    stream.set_read_timeout(None)?;
    let (code, body) = wire::read(&mut stream)?;
    if code != wire::NEST_ATTACH {
        return Ok(());
    }
    let enclosers: Vec<String> = body["enclosers"]
        .as_array()
        .map(|list| list.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    let client = sys::peer_pid(stream.as_raw_fd())?;
    let id = {
        let mut state = state.lock().unwrap();
        state.next += 1;
        let id = state.next;
        state.attachments.insert(id, (client, enclosers));
        state.history.push(id);
        id
    };
    // The edge is recorded before the acknowledgement, so the admitting client
    // releases the nesting lock only once the graph already contains it.
    let result = wire::write(&mut stream, wire::ATTACHED, &json!({}));
    if result.is_ok() {
        while wire::read(&mut stream).is_ok() {}
    }
    let mut state = state.lock().unwrap();
    state.attachments.remove(&id);
    state.history.retain(|entry| *entry != id);
    result
}

/// Byte-for-byte the first-release behaviour: core's frame parser and hello reply.
fn legacy_connection(mut stream: UnixStream, sid: &str) -> std::io::Result<()> {
    same_user(&stream)?;
    stream.set_read_timeout(Some(runtime::HELLO_TIMEOUT))?;
    match ClientMessage::from_frame(read_frame(&mut stream)?)? {
        ClientMessage::Hello { protocol: 1 } => {}
        _ => return Ok(()),
    }
    let reply = HelloReply {
        session_id: sid.parse().map_err(std::io::Error::other)?,
        climon2_version: VERSION.parse().map_err(std::io::Error::other)?,
        protocol: 1,
    };
    write_frame(&mut stream, &ServerMessage::Hello(reply).to_frame()?)?;
    stream.set_read_timeout(None)?;
    // Any unknown post-hello type is a protocol error: the connection closes.
    let _ = ClientMessage::from_frame(read_frame(&mut stream)?)?;
    Ok(())
}
