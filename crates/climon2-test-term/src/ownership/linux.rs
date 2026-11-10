use super::{MARKER, Process};
use std::{fs, io::Read, os::unix::fs::MetadataExt};

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
