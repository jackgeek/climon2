use crate::{absolute, discovery::{self, Metadata}, executable, home, launch_error, nonempty, platform, protocol::{self, Stream}, real_exe, Result};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{env, fs, io::{self, BufRead, BufReader, IsTerminal, Read, Write}, path::PathBuf, process::{Command, Stdio}, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}, mpsc}, thread, time::{Duration, Instant}};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub session_id: String,
    pub command: String,
    pub target: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub metadata: PathBuf,
    pub socket: PathBuf,
    pub depth: u32,
    pub version: String,
    pub cols: u16,
    pub rows: u16,
}
fn depth() -> Result<u32> {
    if nonempty("CLIMON2_SESSION_ID").is_none() { return Ok(1); }
    let inherited = env::var("CLIMON2_SESSION_DEPTH").ok().filter(|s| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).and_then(|s| s.parse::<u32>().ok()).filter(|n| *n > 0).unwrap_or(1);
    inherited.checked_add(1).ok_or_else(|| "session depth overflow".into())
}
fn ordinal(n: u32) -> String {
    let suffix = if (11..=13).contains(&(n % 100)) { "th" } else { match n % 10 { 1 => "st", 2 => "nd", 3 => "rd", _ => "th" } };
    format!("{n}{suffix}")
}
pub fn exec(command: &str, args: &[String]) -> Result<i32> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() { return Err("exec requires a terminal".into()); }
    let home = home()?;
    let depth = depth()?;
    let runtime = real_exe()?.parent().unwrap().to_owned();
    let daemon_path = runtime.join(executable("climon2-daemon"));
    let client_path = runtime.join(executable("climon2-client"));
    platform::check_executable(&daemon_path)?;
    platform::check_executable(&client_path)?;
    let target = platform::resolve(command, args)?;
    let directory = platform::runtime_dir()?;
    platform::verify_transport()?;
    let session_id = uuid::Uuid::new_v4().to_string();
    let socket = directory.join(format!("{}.sock", session_id.replace('-', "")));
    let metadata = home.join("sessions").join(format!("{session_id}.pty"));
    let (cols, rows) = platform::size();
    let version = runtime.file_name().and_then(|s| s.to_str()).filter(|s| semver::Version::parse(s).is_ok()).unwrap_or(env!("CARGO_PKG_VERSION")).to_owned();
    let config = Config { session_id, command: command.into(), args: args.to_vec(), target, cwd: env::current_dir().map_err(|e| e.to_string())?, metadata, socket, depth, version, cols, rows };
    let terminal = platform::TerminalGuard::new()?;
    let mut launch = Command::new(&daemon_path);
    launch.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    platform::prepare_daemon(&mut launch);
    let mut daemon = launch.spawn().map_err(|e| launch_error(&daemon_path, e))?;
    let mut daemon_output = BufReader::new(daemon.stdout.take().unwrap());
    let startup = (|| -> Result<()> {
        let input = daemon.stdin.as_mut().unwrap();
        serde_json::to_writer(&mut *input, &config).map_err(|e| e.to_string())?;
        input.write_all(b"\n").map_err(|e| e.to_string())?;
        let mut line = String::new();
        daemon_output.read_line(&mut line).map_err(|e| launch_error(&daemon_path, e))?;
        let reply: Value = serde_json::from_str(&line).map_err(|e| launch_error(&daemon_path, e))?;
        if let Some(error) = reply["error"].as_str() { return Err(error.to_owned()); }
        if reply["ready"] != true { return Err(launch_error(&daemon_path, "invalid startup response")); }
        terminal.raw()?;
        Ok(())
    })();
    if let Err(error) = startup {
        drop(daemon.stdin.take());
        let _ = daemon.wait();
        return Err(error);
    }
    let result = match Command::new(&client_path).args(["--protocol", "1", "attach", "--metadata"]).arg(&config.metadata).spawn() {
        Ok(mut client) => {
            if depth > 1 { eprintln!("Starting climon2 session nested for the {} time", ordinal(depth - 1)); }
            client.wait()
        }
        Err(error) => Err(error),
    };
    drop(terminal);
    drop(daemon.stdin.take());
    let cleanup = daemon.wait();
    match result {
        Err(e) => Err(launch_error(&client_path, e)),
        Ok(status) if status.code().is_none() => Err("client disconnected".into()),
        Ok(status) => {
            cleanup.map_err(|e| format!("session daemon: {e}"))?;
            let mut line = String::new();
            daemon_output.read_line(&mut line).map_err(|e| format!("client disconnected: {e}"))?;
            let completion: Value = serde_json::from_str(&line).map_err(|_| "client disconnected".to_owned())?;
            let code = platform::status(status);
            if let Some(error) = completion["error"].as_str() { return Err(format!("client disconnected: {error}")); }
            if completion["exit"].as_i64() != Some(code as i64) { return Err("client disconnected".into()); }
            Ok(code)
        }
    }
}

struct Publication { metadata: PathBuf, socket: PathBuf, temporary: PathBuf }
impl Drop for Publication {
    fn drop(&mut self) {
        for path in [&self.metadata, &self.socket, &self.temporary] {
            if let Err(e) = fs::remove_file(path) { if e.kind() != io::ErrorKind::NotFound { eprintln!("climon2: cannot remove {}: {e}", path.display()); } }
        }
    }
}
enum Event { Output(Vec<u8>), Eof, ReadError(String), Attached(Stream), Message(Value), Disconnected, OwnerGone }

pub fn daemon() -> Result<i32> {
    platform::daemon_init()?;
    let mut input = BufReader::new(io::stdin());
    let mut line = String::new();
    input.read_line(&mut line).map_err(|e| e.to_string())?;
    let config: Config = serde_json::from_str(&line).map_err(|e| format!("invalid session startup: {e}"))?;
    let result = run_daemon(config, input);
    match result {
        Err(error) => { println!("{}", json!({"error":error})); Ok(125) }
        Ok(code) => { println!("{}", json!({"exit":code})); Ok(0) }
    }
}
fn run_daemon(config: Config, mut input: BufReader<io::Stdin>) -> Result<i32> {
    let temporary = config.metadata.with_extension("tmp");
    let _publication = Publication { metadata: config.metadata.clone(), socket: config.socket.clone(), temporary: temporary.clone() };
    let listener = protocol::listener(&config.socket).map_err(|e| format!("socket path {}: {e}", config.socket.display()))?;
    platform::private_socket(&config.socket)?;
    let mut pty = platform::Pty::spawn(&config)?;
    let (tx, rx) = mpsc::sync_channel(64);
    let mut reader = pty.reader.take().unwrap();
    let output = tx.clone();
    thread::spawn(move || {
        let mut buffer = [0u8; 16384];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => { let _ = output.send(Event::Eof); break; }
                Err(e) if platform::is_pty_eof(&e) => { let _ = output.send(Event::Eof); break; }
                Err(e) => { let _ = output.send(Event::ReadError(e.to_string())); break; }
                Ok(n) => { let _ = output.send(Event::Output(buffer[..n].to_vec())); }
            }
        }
    });
    let connections = tx.clone();
    let sid = config.session_id.clone();
    let attached = Arc::new(AtomicBool::new(false));
    thread::spawn(move || {
        while let Ok((socket, _)) = listener.accept() {
            let accepted = Instant::now();
            let tx = connections.clone();
            let sid = sid.clone();
            let attached = attached.clone();
            thread::spawn(move || {
                if !platform::peer(&socket) { return; }
                let stream = Stream(socket);
                let deadline = accepted + Duration::from_secs(2);
                let request = match stream.receive(Some(deadline)) { Ok(v) => v, Err(_) => return };
                if request["protocol"] != 1 { return; }
                if request["type"] == "hello" {
                    if let Some(remaining) = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()) {
                        if stream.0.set_write_timeout(Some(remaining)).is_ok() { let _ = stream.send(&json!({"type":"hello", "protocol":1, "session_id":sid})); }
                    }
                    return;
                }
                if request["type"] != "attach" || attached.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() { return; }
                let writer = match stream.clone() { Ok(s) => s, Err(_) => return };
                if tx.send(Event::Attached(writer)).is_err() { return; }
                loop {
                    match stream.receive(None) {
                        Ok(value) => { if tx.send(Event::Message(value)).is_err() { return; } }
                        Err(_) => { let _ = tx.send(Event::Disconnected); return; }
                    }
                }
            });
        }
    });
    let owner = tx.clone();
    thread::spawn(move || {
        let mut buffer = [0; 1];
        let _ = input.read(&mut buffer);
        let _ = owner.send(Event::OwnerGone);
    });
    let now = Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true);
    let record = Metadata {
        format_version: 1, session_id: config.session_id.clone(), socket_path: config.socket.clone(), daemon_pid: std::process::id(),
        daemon_start_time: platform::process_start()?, created_at: now, command: config.command, args: config.args,
        cwd: config.cwd, depth: config.depth, climon2_version: config.version,
    };
    fs::create_dir_all(config.metadata.parent().unwrap()).map_err(|e| format!("cannot create {}: {e}", config.metadata.display()))?;
    let mut file = platform::private_file(&temporary).map_err(|e| format!("cannot publish {}: {e}", config.metadata.display()))?;
    serde_json::to_writer(&mut file, &record).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&temporary, &config.metadata).map_err(|e| format!("cannot publish {}: {e}", config.metadata.display()))?;
    println!("{}", json!({"ready":true}));
    io::stdout().flush().map_err(|e| e.to_string())?;
    let mut client: Option<Stream> = None;
    let mut pending = Vec::new();
    let mut eof = false;
    let mut status = None;
    let started = Instant::now();
    loop {
        if status.is_none() {
            status = pty.try_wait().map_err(|e| e.to_string())?;
            if status.is_some() { pty.terminate(); }
        }
        if let (Some(code), true, Some(stream)) = (status, eof, client.as_ref()) {
            stream.send(&json!({"type":"exit", "code":code})).map_err(|e| e.to_string())?;
            return Ok(code);
        }
        if client.is_none() && started.elapsed() > Duration::from_secs(10) { return Err("client disconnected".into()); }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Event::Output(data)) => {
                if let Some(stream) = &client { stream.send(&json!({"type":"output", "data":data})).map_err(|e| e.to_string())?; }
                else { pending.push(data); }
            }
            Ok(Event::Eof) => eof = true,
            Ok(Event::ReadError(error)) => return Err(format!("cannot read terminal: {error}")),
            Ok(Event::Attached(stream)) => {
                for data in pending.drain(..) { stream.send(&json!({"type":"output", "data":data})).map_err(|e| e.to_string())?; }
                client = Some(stream);
            }
            Ok(Event::Message(value)) => {
                match value["type"].as_str() {
                    Some("input") => {
                        let data: Vec<u8> = serde_json::from_value(value["data"].clone()).map_err(|e| e.to_string())?;
                        if !eof { pty.writer.write_all(&data).map_err(|e| e.to_string())?; }
                    }
                    Some("resize") => {
                        let cols = value["cols"].as_u64().filter(|v| *v > 0 && *v <= u16::MAX as u64).ok_or("invalid terminal size")? as u16;
                        let rows = value["rows"].as_u64().filter(|v| *v > 0 && *v <= u16::MAX as u64).ok_or("invalid terminal size")? as u16;
                        pty.resize(cols, rows).map_err(|e| e.to_string())?;
                    }
                    _ => return Err("invalid client message".into()),
                }
            }
            Ok(Event::Disconnected | Event::OwnerGone) => return Err("client disconnected".into()),
            Err(mpsc::RecvTimeoutError::Timeout) => (),
            Err(e) => return Err(e.to_string()),
        }
    }
}

pub fn client(args: &[String]) -> Result<i32> {
    let path = match args {
        [a, b, c, d, path] if a == "--protocol" && b == "1" && c == "attach" && d == "--metadata" => absolute(path)?,
        _ => return Err("usage: climon2-client --protocol 1 attach --metadata <path>".into()),
    };
    let record = discovery::read(&path)?;
    let stream = Stream::connect(&record.socket_path, Instant::now() + Duration::from_secs(2)).map_err(|e| format!("client disconnected: {e}"))?;
    stream.send(&json!({"type":"attach", "protocol":1})).map_err(|e| format!("client disconnected: {e}"))?;
    let writer = Arc::new(Mutex::new(stream.clone().map_err(|e| e.to_string())?));
    let input = writer.clone();
    thread::spawn(move || {
        let mut buffer = [0; 4096];
        loop {
            match io::stdin().read(&mut buffer) {
                Ok(0) | Err(_) => { let _ = input.lock().unwrap().0.shutdown(std::net::Shutdown::Both); return; }
                Ok(n) => { if input.lock().unwrap().send(&json!({"type":"input", "data":&buffer[..n]})).is_err() { return; } }
            }
        }
    });
    thread::spawn(move || {
        let mut previous = platform::size();
        loop {
            thread::sleep(Duration::from_millis(30));
            let size = platform::size();
            if size != previous {
                if writer.lock().unwrap().send(&json!({"type":"resize", "cols":size.0, "rows":size.1})).is_err() { return; }
                previous = size;
            }
        }
    });
    let mut output = io::stdout().lock();
    let mut pending = Vec::new();
    loop {
        let value = stream.receive(None).map_err(|e| format!("client disconnected: {e}"))?;
        match value["type"].as_str() {
            Some("output") => {
                let bytes: Vec<u8> = serde_json::from_value(value["data"].clone()).map_err(|e| format!("client disconnected: {e}"))?;
                platform::relay_output(&mut output, &bytes, &mut pending).map_err(|e| format!("client disconnected: {e}"))?;
            }
            Some("exit") => {
                if !pending.is_empty() { return Err("client disconnected: incomplete terminal encoding".into()); }
                return value["code"].as_i64().and_then(|n| i32::try_from(n).ok()).ok_or_else(|| "client disconnected: invalid exit status".into());
            }
            _ => return Err("client disconnected: invalid message".into()),
        }
    }
}
