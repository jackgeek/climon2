//! A fake daemon peer: the Library layer's adversarial fixture (#93). It
//! serves one connection on its own Unix socket, as a scripted session
//! daemon would, with session metadata that names it. It can send
//! malformed, slow, out-of-order, oversized or truncated traffic and lose
//! responses, and it records every frame the library sent it, so tests see
//! exactly what reached the daemon (and what was never replayed).
//!
//! A script is JSON:
//! - `session`: the GUID as 32 hex digits; `release`: the release its
//!   metadata and hello name (default the library's); `metadata`: overrides
//!   of the metadata's fields;
//! - `hello`: the reply (`{"json": ...}` merged over a valid revision-1
//!   reply, less the members `without` names, or `{"frame": action}`), or
//!   `"close"` or `"hold"`;
//! - `open` and `leave`: the actions after the library's Open, and after
//!   each Leave; `leaves`, a list of action lists, overrides `leave` for the
//!   first Leaves in turn.
//!
//! An action is `{"send": {"type": n, "json": value}}` (a JSON body in which
//! the string `"$open"` or `"$leave"` is that request's ID) or
//! `{"send": {"type": n, "hex": body}}`, `{"raw": hex}` (bytes as they are),
//! `{"together": [action, ...]}` (several actions' bytes in one write),
//! `{"bytewise": action}` (an action's bytes one at a time),
//! `{"wait": "<path>"}` (until that file exists, bounded; an expired wait is
//! recorded as `wait_expired`), `"deaf"` (shut down the
//! reading side, so the library's next write fails, and record `deaf`),
//! `"close"`, or `"hold"` (stop answering; keep the connection until the
//! library closes it).

use super::{abi::hex, probe};
use climon2_core::{
    protocol::{self, ClientMessage},
    wire::{FrameLimit, read_frame},
};
use serde_json::{Map, Value, json};
use std::{
    fs,
    io::Write,
    net::Shutdown,
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const DEFAULT_SESSION: &str = "0123456789abcdef0123456789abcdef";

/// One fake daemon, serving one connection on its own thread.
pub struct Peer {
    pub metadata: PathBuf,
    pub session: String,
    received: Arc<Mutex<Vec<Value>>>,
    /// Set once the scenario is over: a peer never connected stops waiting.
    over: Arc<AtomicBool>,
    thread: Option<JoinHandle<Value>>,
}

impl Peer {
    pub fn start(root: &Path, name: &str, release: &str, script: &Value) -> Self {
        let session = script["session"]
            .as_str()
            .unwrap_or(DEFAULT_SESSION)
            .to_owned();
        let guid = canonical(&session);
        let socket = root.join(format!("{name}.sock"));
        let _ = fs::remove_file(&socket);
        let listener = UnixListener::bind(&socket).expect("the fake daemon binds its socket");
        let directory = root.join(format!("{name}-metadata"));
        fs::create_dir_all(&directory).expect("the fake metadata directory");
        let metadata = directory.join(format!("{guid}.pty"));
        let release = script["release"].as_str().unwrap_or(release).to_owned();
        let mut record = json!({
            "format_version": 1,
            "session_id": guid,
            "socket_path": socket.to_str().expect("a UTF-8 socket path"),
            "daemon_pid": std::process::id(),
            "daemon_start_time": 0,
            "command": "fake-daemon",
            "args": [],
            "cwd": "/",
            "depth": 1,
            "created_at": "2026-11-13T00:00:00.000Z",
            "climon2_version": release,
        });
        merge(&mut record, &script["metadata"]);
        fs::write(&metadata, record.to_string()).expect("the fake metadata is written");
        let received = Arc::new(Mutex::new(Vec::new()));
        let over = Arc::new(AtomicBool::new(false));
        let waiting = Arc::clone(&over);
        let serving = Serving {
            script: script.clone(),
            guid,
            release,
            received: Arc::clone(&received),
            ids: Map::new(),
        };
        let thread = thread::Builder::new()
            .name(format!("peer-{name}"))
            .spawn(move || serving.serve(&listener, &waiting))
            .expect("the fake daemon starts");
        Self {
            metadata,
            session,
            received,
            over,
            thread: Some(thread),
        }
    }

    /// How many frames the library has sent so far.
    pub fn frames(&self) -> usize {
        self.received.lock().unwrap().len()
    }

    /// Waits until the library has sent `count` frames, bounded.
    pub fn await_frames(&self, count: usize) -> bool {
        let deadline = probe::deadline();
        while self.frames() < count && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        self.frames() >= count
    }

    /// Everything the connection carried, once the library closed it.
    pub fn report(mut self) -> Value {
        self.over.store(true, Ordering::SeqCst);
        let thread = self.thread.take().expect("reported once");
        let deadline = probe::deadline();
        while !thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if !thread.is_finished() {
            return json!({ "finished": false, "frames": *self.received.lock().unwrap() });
        }
        thread.join().expect("the fake daemon ends")
    }
}

fn merge(base: &mut Value, overrides: &Value) {
    if let (Some(base), Some(overrides)) = (base.as_object_mut(), overrides.as_object()) {
        for (key, value) in overrides {
            base.insert(key.clone(), value.clone());
        }
    }
}

/// The canonical text of 32 hex digits.
fn canonical(hex: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

struct Serving {
    script: Value,
    guid: String,
    release: String,
    received: Arc<Mutex<Vec<Value>>>,
    /// The ID of each request kind the library sent, for `"$open"` and
    /// `"$leave"`.
    ids: Map<String, Value>,
}

/// How the connection went on after an action.
#[derive(PartialEq)]
enum Next {
    Continue,
    Close,
    Hold,
}

impl Serving {
    fn serve(mut self, listener: &UnixListener, over: &AtomicBool) -> Value {
        let Some(mut stream) = accept(listener, over) else {
            return json!({ "accepted": false, "frames": [] });
        };
        let next = self.hello(&mut stream);
        let closed_by_library = match next {
            Next::Continue => self.converse(&mut stream),
            Next::Hold => hold(&mut stream, &self.received),
            Next::Close => false,
        };
        json!({
            "accepted": true,
            "finished": true,
            "frames": *self.received.lock().unwrap(),
            "closed_by_library": closed_by_library,
        })
    }

    fn record(&self, entry: Value) {
        self.received.lock().unwrap().push(entry);
    }

    /// The frozen hello: the library's request, then the scripted reply.
    fn hello(&mut self, stream: &mut UnixStream) -> Next {
        let Ok(request) = protocol::read_frame(stream) else {
            return Next::Close;
        };
        let protocol = ClientMessage::from_frame(request).ok();
        self.record(json!({ "hello": format!("{protocol:?}") }));
        let reply = self.script["hello"].clone();
        match &reply {
            Value::String(word) if word == "close" => Next::Close,
            Value::String(word) if word == "hold" => Next::Hold,
            Value::Object(reply) if reply.contains_key("frame") => {
                self.act(stream, &reply["frame"])
            }
            reply => {
                let mut body = json!({
                    "session_id": self.guid,
                    "climon2_version": self.release,
                    "protocol": 1,
                    "capabilities": ["attach", "kill", "state-v1", "client-startup-v1",
                        "daemon-wire-v1", "monitor-v1", "title-v1"],
                    "state": { "lifecycle": "attached", "attachments": 1, "controlled": true },
                    "daemon_wire_v1": {
                        "max_request_frame_bytes": 1_048_576,
                        "max_server_frame_bytes": 1_048_576,
                    },
                });
                merge(&mut body, &reply["json"]);
                for member in reply["without"].as_array().into_iter().flatten() {
                    body.as_object_mut()
                        .expect("an object")
                        .remove(member.as_str().expect("a member name"));
                }
                self.write(stream, &frame(1, body.to_string().as_bytes()), false)
            }
        }
    }

    /// Revision-1 frames until the library closes the connection or the
    /// script ends it: whether the library closed it.
    fn converse(&mut self, stream: &mut UnixStream) -> bool {
        let limit = FrameLimit::new(u32::MAX).expect("positive");
        loop {
            let frame = match read_frame(stream, limit) {
                Ok(Some(frame)) => frame,
                Ok(None) => return true,
                Err(_) => return false,
            };
            let kind = match frame.code() {
                0x30 => "open",
                0x37 => "leave",
                _ => "other",
            };
            let body: Value = serde_json::from_slice(frame.body())
                .unwrap_or_else(|_| json!({ "hex": hex(frame.body()) }));
            self.ids.insert(kind.to_owned(), body["request_id"].clone());
            self.record(json!({ "type": frame.code(), "kind": kind, "body": body }));
            let actions = self.actions(kind);
            match self.script_actions(stream, &actions) {
                Next::Continue => {}
                Next::Close => return false,
                Next::Hold => return hold(stream, &self.received),
            }
        }
    }

    /// The actions after a frame of `kind`: the n-th Leave's own, if the
    /// script lists one.
    fn actions(&self, kind: &str) -> Value {
        let received = self.received.lock().unwrap();
        let leaves = received
            .iter()
            .filter(|frame| frame["kind"] == "leave")
            .count();
        let own = (kind == "leave")
            .then(|| self.script["leaves"].get(leaves.saturating_sub(1)).cloned())
            .flatten();
        own.unwrap_or_else(|| self.script[kind].clone())
    }

    fn script_actions(&mut self, stream: &mut UnixStream, actions: &Value) -> Next {
        for action in actions.as_array().into_iter().flatten() {
            let next = self.act(stream, action);
            if next != Next::Continue {
                return next;
            }
        }
        Next::Continue
    }

    fn act(&mut self, stream: &mut UnixStream, action: &Value) -> Next {
        match action {
            Value::String(word) if word == "close" => Next::Close,
            Value::String(word) if word == "hold" => Next::Hold,
            Value::String(word) if word == "deaf" => {
                let shut = stream.shutdown(Shutdown::Read);
                self.record(json!({ "deaf": shut.is_ok() }));
                Next::Continue
            }
            action => {
                if let Some(path) = action["wait"].as_str() {
                    if !wait_file(Path::new(path)) {
                        self.record(json!({ "wait_expired": path }));
                    }
                    return Next::Continue;
                }
                let bytewise = action.get("bytewise").is_some();
                let action = action.get("bytewise").unwrap_or(action);
                let bytes = self.bytes(action);
                self.write(stream, &bytes, bytewise)
            }
        }
    }

    fn bytes(&self, action: &Value) -> Vec<u8> {
        if let Some(raw) = action["raw"].as_str() {
            return from_hex(raw);
        }
        if let Some(actions) = action["together"].as_array() {
            return actions
                .iter()
                .flat_map(|action| self.bytes(action))
                .collect();
        }
        let send = &action["send"];
        let code = send["type"].as_u64().expect("a frame type") as u8;
        let body = match send["hex"].as_str() {
            Some(hex) => from_hex(hex),
            None => self.substitute(&send["json"]).to_string().into_bytes(),
        };
        frame(code, &body)
    }

    /// `"$open"` and `"$leave"` become the IDs the library sent.
    fn substitute(&self, value: &Value) -> Value {
        match value {
            Value::String(text) if text.starts_with('$') => {
                self.ids.get(&text[1..]).cloned().unwrap_or(Value::Null)
            }
            Value::Array(items) => {
                Value::Array(items.iter().map(|item| self.substitute(item)).collect())
            }
            Value::Object(members) => Value::Object(
                members
                    .iter()
                    .map(|(key, member)| (key.clone(), self.substitute(member)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn write(&self, stream: &mut UnixStream, bytes: &[u8], bytewise: bool) -> Next {
        let written = if bytewise {
            bytes.chunks(1).try_for_each(|byte| {
                stream.write_all(byte)?;
                stream.flush()?;
                thread::sleep(Duration::from_millis(2));
                Ok::<(), std::io::Error>(())
            })
        } else {
            stream.write_all(bytes)
        };
        match written {
            Ok(()) => Next::Continue,
            Err(_) => Next::Close,
        }
    }
}

fn frame(code: u8, body: &[u8]) -> Vec<u8> {
    let length = (body.len() + 1) as u32;
    [&length.to_be_bytes()[..], &[code], body].concat()
}

fn from_hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex byte"))
        .collect()
}

/// The library's connection, with every read bounded, unless the scenario
/// ends without one.
fn accept(listener: &UnixListener, over: &AtomicBool) -> Option<UnixStream> {
    listener.set_nonblocking(true).ok()?;
    let deadline = probe::deadline();
    while Instant::now() < deadline && !over.load(Ordering::SeqCst) {
        if let Ok((stream, _)) = listener.accept() {
            stream.set_nonblocking(false).ok()?;
            stream
                .set_read_timeout(Some(Duration::from_secs(15)))
                .ok()?;
            return Some(stream);
        }
        thread::sleep(Duration::from_millis(5));
    }
    None
}

/// Answers nothing more and keeps the connection until the library closes
/// it, bounded by the stream's read timeout: whether it did. A frame the
/// library sends meanwhile is still recorded, so a replay would show.
fn hold(stream: &mut UnixStream, received: &Mutex<Vec<Value>>) -> bool {
    let limit = FrameLimit::new(u32::MAX).expect("positive");
    loop {
        match read_frame(stream, limit) {
            Ok(Some(frame)) => received
                .lock()
                .unwrap()
                .push(json!({ "type": frame.code(), "held": hex(frame.body()) })),
            Ok(None) => return true,
            Err(_) => return false,
        }
    }
}

/// Waits, bounded, for `path` to exist: whether it does.
fn wait_file(path: &Path) -> bool {
    let deadline = probe::deadline();
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    path.exists()
}
