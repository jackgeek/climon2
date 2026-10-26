use std::{env, fs, path::PathBuf, process};
use serde_json::Value;
fn exists(pid: u32) -> bool {
    #[cfg(unix)] unsafe { extern "C" { fn kill(pid: i32, signal: i32) -> i32; } kill(pid as i32, 0) == 0 }
    #[cfg(windows)] unsafe {
        use std::ffi::c_void;
        #[link(name="kernel32")]
        extern "system" { fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void; fn CloseHandle(handle: *mut c_void) -> i32; }
        let handle = OpenProcess(0x1000, 0, pid);
        if handle.is_null() { false } else { CloseHandle(handle); true }
    }
}
fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    if args != ["list", "--json"] && args != ["list"] { eprintln!("climon2: usage: list [--json]"); process::exit(125); }
    let directory = PathBuf::from(env::var_os("CLIMON2_HOME").unwrap()).join("sessions");
    let mut records: Vec<Value> = Vec::new();
    if directory.exists() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|x| x.to_str()) != Some("pty") { continue; }
            let value: Value = match fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()) {
                Some(value) => value,
                None => { eprintln!("climon2: ignoring session metadata: {}", path.display()); continue; }
            };
            if value["format_version"] != 1 { eprintln!("climon2: ignoring session metadata: {}", path.display()); continue; }
            if value["session_id"].as_str() != path.file_stem().and_then(|x| x.to_str()) { continue; }
            // Deliberate defect: PID existence replaces the required socket hello.
            if value["daemon_pid"].as_u64().is_some_and(|pid| exists(pid as u32)) { records.push(value); }
        }
    }
    records.sort_by(|a,b| b["created_at"].as_str().cmp(&a["created_at"].as_str()).then_with(|| a["session_id"].as_str().cmp(&b["session_id"].as_str())));
    if args.len() == 2 { println!("{}", serde_json::to_string(&records).unwrap()); }
    else if records.is_empty() { println!("No climon2 sessions running."); }
    else { println!("SESSION CREATED COMMAND ARGS"); for record in records { println!("{} {} {}", record["session_id"].as_str().unwrap(), record["created_at"].as_str().unwrap(), record["command"].as_str().unwrap()); } }
}
