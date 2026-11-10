use super::{MARKER, Process};
use std::{
    collections::HashMap,
    fs,
    io::Read,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

pub(super) struct Lease {
    pub(super) path: PathBuf,
    pub(super) identity: (u64, u64),
}

impl Lease {
    pub(super) fn new(directory: &Path) -> Self {
        let path = directory.join("process-ownership");
        let file = fs::File::create(&path).expect("create process ownership lease");
        let metadata = file.metadata().expect("process ownership lease identity");
        Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
        }
    }
}

pub(super) fn lease_marker(pid: i32, leases: &HashMap<(u64, u64), String>) -> Option<String> {
    fs::read_dir(format!("/proc/{pid}/fd"))
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| fs::metadata(entry.path()).ok())
        .find_map(|metadata| leases.get(&(metadata.dev(), metadata.ino())).cloned())
}

pub(super) fn pids() -> Vec<i32> {
    let uid = nix::unistd::geteuid().as_raw();
    fs::read_dir("/proc")
        .expect("list Linux processes")
        .filter_map(Result::ok)
        .filter(|entry| entry.metadata().is_ok_and(|metadata| metadata.uid() == uid))
        .filter_map(|entry| entry.file_name().to_str()?.parse().ok())
        .collect()
}

pub(super) fn process(pid: i32) -> Option<Process> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm may contain spaces and parentheses, so only the last ')' ends it.
    let (head, fields) = stat.rsplit_once(')')?;
    let name = head.split_once('(')?.1.to_owned();
    let fields: Vec<_> = fields.split_whitespace().collect();
    if matches!(*fields.first()?, "Z" | "X" | "x") {
        return None;
    }
    Some(Process {
        pid,
        parent: fields.get(1)?.parse().ok()?,
        session: fields.get(3)?.parse().ok()?,
        unique: (fields.get(19)?.parse().ok()?, pid),
        image: None,
        parent_image: None,
        name,
        marker: None,
    })
}

pub(super) fn marker_of(pid: i32, buffer: &mut Vec<u8>) -> Option<String> {
    // Unlike macOS's image id, start ticks do not change on exec. Never cache an
    // environment by Linux process identity: an exec can replace its marker.
    buffer.clear();
    fs::File::open(format!("/proc/{pid}/environ"))
        .ok()?
        .read_to_end(buffer)
        .ok()?;
    let prefix = format!("{MARKER}=");
    buffer
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(prefix.as_bytes()))
        .map(|value| String::from_utf8_lossy(value).into_owned())
}
