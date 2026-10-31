//! PROTOTYPE client: `attach` performs proof + locked graph check + admission;
//! `hello`, `raw-nest-attach` and `list` are probes used by the experiment.
use climon2_core::metadata::SessionMetadata;
use nesting_proof_prototype::{
    graph::{self, Reject},
    proof, runtime, wire,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    time::Duration,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("attach") => attach(&args[1..]),
        Some("hello") => hello(&args[1]),
        Some("raw-nest-attach") => raw_nest_attach(&args[1]),
        Some("list") => list(Path::new(&args[2])),
        _ => 2,
    };
    std::process::exit(code);
}

struct AttachArgs {
    home: PathBuf,
    target: String,
    report: Option<PathBuf>,
}

fn attach_args(args: &[String]) -> AttachArgs {
    let mut iter = args.iter();
    let (mut home, mut target, mut report) = (None, None, None);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--home" => home = iter.next().map(PathBuf::from),
            "--report" => report = iter.next().map(PathBuf::from),
            other => target = Some(other.to_owned()),
        }
    }
    AttachArgs { home: home.expect("--home"), target: target.expect("target"), report }
}

fn report(path: &Option<PathBuf>, line: &str) {
    if let Some(path) = path {
        let staging = path.with_extension("staging");
        fs::write(&staging, format!("{line}\n")).unwrap();
        fs::rename(&staging, path).unwrap();
    }
}

fn attach(args: &[String]) -> i32 {
    let args = attach_args(args);
    match admit(&args) {
        Ok((stream, enclosers)) => {
            report(&args.report, &format!("attached {} enclosers={enclosers}", std::process::id()));
            hold(stream);
            0
        }
        Err(message) => {
            eprintln!("climon2: error: {message}");
            report(&args.report, &format!("rejected 125 {message}"));
            125
        }
    }
}

fn admit(args: &AttachArgs) -> Result<(UnixStream, String), String> {
    if unsafe { libc::isatty(0) != 1 || libc::isatty(1) != 1 } {
        return Err("attach requires a terminal on stdin and stdout".into());
    }
    // Kernel facts first; nothing below reads CLIMON2_SESSION_* or any other env.
    let candidates = proof::candidates();
    let unsafe_no_lock = std::env::var_os("NEST_UNSAFE_NO_LOCK").is_some();
    let lock = if unsafe_no_lock { None } else { Some(runtime::lock().map_err(|e| e.to_string())?) };
    let snapshot = runtime::snapshot().map_err(|e| e.to_string())?;
    let nodes: Vec<_> = snapshot.iter().map(|(_, node)| node.clone()).collect();
    let enclosers = proof::resolve(&candidates, &nodes);
    let described = enclosers
        .sessions
        .iter()
        .map(|(sid, sources)| format!("{sid}(source={})", proof::describe(sources)))
        .collect::<Vec<_>>()
        .join(";");
    let fail = |reason: String| format!("{reason} enclosers={described}");
    if !enclosers.unreachable.is_empty() {
        let why = format!("enclosing daemon(s) {:?} are not reachable", enclosers.unreachable);
        return Err(fail(Reject::Unprovable(why).to_string()));
    }
    let directory = runtime::runtime_dir(&args.home).map_err(|e| e.to_string())?;
    let target_socket = directory.join(format!("{}.sock", args.target));
    if !snapshot.iter().any(|(socket, _)| *socket == target_socket) {
        return Err(fail(Reject::UnknownTarget.to_string()));
    }
    let sids: BTreeSet<String> = enclosers.sessions.keys().cloned().collect();
    graph::admit(&args.target, &sids, &nodes).map_err(|reject| fail(reject.to_string()))?;
    if let Some(ms) = std::env::var("NEST_ADMIT_DELAY_MS").ok().and_then(|v| v.parse().ok()) {
        std::thread::sleep(Duration::from_millis(ms));
    }
    let stream = send_attach(&target_socket, &args.target, &sids).map_err(fail)?;
    drop(lock);
    Ok((stream, described))
}

fn send_attach(socket: &Path, target: &str, enclosers: &BTreeSet<String>) -> Result<UnixStream, String> {
    let mut stream = UnixStream::connect(socket).map_err(|e| e.to_string())?;
    stream.set_read_timeout(Some(runtime::HELLO_TIMEOUT)).map_err(|e| e.to_string())?;
    wire::write(&mut stream, wire::HELLO, &wire::hello_request()).map_err(|e| e.to_string())?;
    match wire::read(&mut stream) {
        Ok((wire::HELLO, reply)) if reply["session_id"] == target && reply.get("nesting").is_some() => {}
        _ => return Err("target changed during admission".into()),
    }
    wire::write(&mut stream, wire::NEST_ATTACH, &json!({ "enclosers": enclosers })).map_err(|e| e.to_string())?;
    match wire::read(&mut stream) {
        Ok((wire::ATTACHED, _)) => {}
        _ => return Err("target did not confirm attachment".into()),
    }
    stream.set_read_timeout(None).map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Stays attached until the daemon goes away or this client is terminated.
fn hold(mut stream: UnixStream) {
    while wire::read(&mut stream).is_ok() {}
}

fn find_socket(sid: &str) -> PathBuf {
    runtime::all_sockets()
        .unwrap()
        .into_iter()
        .find(|s| s.file_stem().is_some_and(|stem| stem == sid))
        .expect("session socket")
}

fn hello(sid: &str) -> i32 {
    let (_, reply) = runtime::hello(&find_socket(sid)).expect("hello");
    println!("{reply}");
    0
}

fn raw_nest_attach(sid: &str) -> i32 {
    let mut stream = UnixStream::connect(find_socket(sid)).unwrap();
    stream.set_read_timeout(Some(runtime::HELLO_TIMEOUT)).unwrap();
    wire::write(&mut stream, wire::HELLO, &wire::hello_request()).unwrap();
    let _ = wire::read(&mut stream).unwrap();
    let _ = wire::write(&mut stream, wire::NEST_ATTACH, &json!({ "enclosers": [] }));
    match wire::read(&mut stream) {
        Ok((wire::ATTACHED, _)) => println!("attached"),
        _ => println!("closed"),
    }
    0
}

/// A new `list --json`: metadata unchanged plus nullable dynamic state.
fn list(home: &Path) -> i32 {
    let mut rows = Vec::new();
    for entry in fs::read_dir(home.join("sessions")).unwrap().flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::read(&path).map_err(|_| ()).and_then(|b| SessionMetadata::from_json(&path, &b).map_err(|_| ())) else {
            continue;
        };
        let Ok((_, reply)) = runtime::hello(Path::new(metadata.socket_path.as_str())) else {
            continue;
        };
        if reply["session_id"] != metadata.session_id.as_str() {
            continue;
        }
        let mut row = serde_json::to_value(&metadata).unwrap();
        row["state"] = reply.get("state").cloned().unwrap_or(Value::Null);
        rows.push(row);
    }
    println!("{}", Value::Array(rows));
    0
}
