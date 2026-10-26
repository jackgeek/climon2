use crate::{home, platform, protocol, Result};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::{fs, io, path::{Path, PathBuf}};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Metadata {
    pub format_version: u32,
    pub session_id: String,
    pub socket_path: PathBuf,
    pub daemon_pid: u32,
    pub daemon_start_time: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub depth: u32,
    pub created_at: String,
    pub climon2_version: String,
}
impl Metadata {
    pub fn valid(&self) -> bool {
        self.format_version == 1 && self.daemon_pid > 0 && self.depth > 0
            && guid(&self.session_id) && self.socket_path.is_absolute() && self.cwd.is_absolute()
            && stamp(&self.created_at).is_ok() && stamp(&self.daemon_start_time).is_ok()
            && semver::Version::parse(&self.climon2_version).is_ok()
    }
}
fn stamp(s: &str) -> std::result::Result<DateTime<FixedOffset>, chrono::ParseError> { DateTime::parse_from_rfc3339(s) }
fn guid(s: &str) -> bool { uuid::Uuid::parse_str(s).map(|id| id.to_string() == s).unwrap_or(false) }
pub fn read(path: &Path) -> Result<Metadata> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let record: Metadata = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !record.valid() { return Err("invalid session metadata".into()); }
    Ok(record)
}
pub fn list(json: bool) -> Result<i32> {
    let directory = home()?.join("sessions");
    #[cfg(windows)]
    match platform::check_directory_access(&directory) {
        Ok(()) => (),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return output(&[], json),
        Err(e) => return Err(format!("cannot enumerate {}: {e}", directory.display())),
    }
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return output(&[], json),
        Err(e) => return Err(format!("cannot enumerate {}: {e}", directory.display())),
    };
    let mut records = Vec::new();
    for entry in entries {
        let path = entry.map_err(|e| format!("cannot enumerate {}: {e}", directory.display()))?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("pty") { continue; }
        let record = match read(&path) {
            Ok(record) => record,
            Err(e) => {
                if fs::metadata(&path).is_err_and(|e| e.kind() == io::ErrorKind::NotFound) { continue; }
                eprintln!("climon2: ignoring session metadata: {}: {e}", path.display());
                continue;
            }
        };
        if path.file_stem().and_then(|s| s.to_str()) != Some(&record.session_id) { continue; }
        if protocol::live(&record.socket_path, &record.session_id) { records.push(record); }
    }
    records.sort_by(|a, b| stamp(&b.created_at).unwrap().cmp(&stamp(&a.created_at).unwrap()).then_with(|| a.session_id.cmp(&b.session_id)));
    output(&records, json)
}
fn output(records: &[Metadata], json: bool) -> Result<i32> {
    if json {
        let value = serde_json::to_string(records).map_err(|e| e.to_string())?;
        let value: String = value.chars().map(|c| if ('\u{7f}'..='\u{9f}').contains(&c) { format!("\\u{:04x}", c as u32) } else { c.to_string() }).collect();
        println!("{value}");
    } else if records.is_empty() { println!("No climon2 sessions running."); }
    else {
        println!("SESSION  CREATED  COMMAND  ARGS");
        for r in records {
            let date = platform::local_stamp(stamp(&r.created_at).unwrap().timestamp())?;
            let args = r.args.iter().map(|arg| format!("'{}'", escape(arg).replace('\'', "'\\''"))).collect::<Vec<_>>().join(" ");
            let args = if args.chars().count() > 80 { format!("{}\u{2026}", args.chars().take(79).collect::<String>()) } else { args };
            println!("{}  {}  {}  {}", r.session_id, date, escape(&r.command), args);
        }
    }
    Ok(0)
}
fn escape(s: &str) -> String {
    s.chars().map(|c| match c {
        '\\' => "\\\\".into(),
        c if c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c) => format!("\\u{:04x}", c as u32),
        c => c.to_string(),
    }).collect()
}
