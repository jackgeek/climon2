//! PROTOTYPE: runtime directories, the per-user nesting lock and graph discovery.
//!
//! Production would use `/tmp`; the prototype takes the root from
//! `NEST_RUNTIME_ROOT` so experiments never touch real runtime directories.
use crate::{graph::Node, sys, wire};
use climon2_core::layout::RuntimeLayout;
use std::{
    fs::{self, File, OpenOptions},
    io,
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub const HELLO_TIMEOUT: Duration = Duration::from_secs(2);
pub const LOCK_TIMEOUT: Duration = Duration::from_secs(10);

pub fn root() -> io::Result<PathBuf> {
    std::env::var_os("NEST_RUNTIME_ROOT")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("NEST_RUNTIME_ROOT is required by the prototype"))
}

pub fn runtime_dir(home: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(home)?;
    let canonical = fs::canonicalize(home)?;
    let layout = RuntimeLayout::new(&root()?, sys::euid(), &canonical).map_err(io::Error::other)?;
    Ok(layout.directory().to_owned())
}

/// Every session socket of this user, across every climon2 home.
pub fn all_sockets() -> io::Result<Vec<PathBuf>> {
    let prefix = format!("climon2-{}-", sys::euid());
    let mut sockets = Vec::new();
    for entry in fs::read_dir(root()?)? {
        let path = entry?.path();
        let is_runtime = path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with(&prefix));
        if !is_runtime || !path.is_dir() {
            continue;
        }
        for socket in fs::read_dir(&path)? {
            let socket = socket?.path();
            if socket.extension().is_some_and(|e| e == "sock") {
                sockets.push(socket);
            }
        }
    }
    Ok(sockets)
}

/// Held from graph snapshot until the target daemon has recorded the edge.
pub struct NestingLock(#[allow(dead_code)] File);

pub fn lock() -> io::Result<NestingLock> {
    let path = root()?.join(format!("climon2-{}.nesting.lock", sys::euid()));
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(path)?;
    let deadline = Instant::now() + LOCK_TIMEOUT;
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(NestingLock(file));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "nesting lock timed out"));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

/// Raw hello reply plus the daemon's PID from the kernel, never from the reply.
pub fn hello(socket: &Path) -> io::Result<(u32, serde_json::Value)> {
    let mut stream = UnixStream::connect(socket)?;
    let pid = sys::peer_pid(stream.as_raw_fd())?;
    stream.set_read_timeout(Some(HELLO_TIMEOUT))?;
    stream.set_write_timeout(Some(HELLO_TIMEOUT))?;
    wire::write(&mut stream, wire::HELLO, &wire::hello_request())?;
    match wire::read(&mut stream)? {
        (wire::HELLO, reply) => Ok((pid, reply)),
        _ => Err(io::Error::other("unexpected hello reply")),
    }
}

pub fn node(pid: u32, reply: &serde_json::Value) -> Option<Node> {
    let sid = reply.get("session_id")?.as_str()?.to_owned();
    let incoming = reply.get("nesting").and_then(|n| n.get("incoming")).and_then(|i| i.as_array());
    Some(match incoming {
        Some(list) => Node::Capable {
            sid,
            pid,
            incoming: list.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect(),
        },
        None => Node::Legacy { sid, pid },
    })
}

/// Probes every socket in parallel. A refused socket is a dead daemon and is not a
/// node; anything else that fails to identify itself is `Unresponsive`.
pub fn snapshot() -> io::Result<Vec<(PathBuf, Node)>> {
    let sockets = all_sockets()?;
    let nodes = thread::scope(|scope| {
        let workers: Vec<_> = sockets
            .iter()
            .map(|socket| scope.spawn(move || probe(socket)))
            .collect();
        workers.into_iter().map(|w| w.join().ok().flatten()).collect::<Vec<_>>()
    });
    Ok(sockets.into_iter().zip(nodes).filter_map(|(s, n)| n.map(|n| (s, n))).collect())
}

fn probe(socket: &Path) -> Option<Node> {
    match hello(socket) {
        Ok((pid, reply)) => Some(node(pid, &reply).unwrap_or(Node::Unresponsive { pid: Some(pid) })),
        Err(error) if matches!(error.raw_os_error(), Some(libc::ECONNREFUSED | libc::ENOENT)) => None,
        Err(_) => Some(Node::Unresponsive { pid: None }),
    }
}
