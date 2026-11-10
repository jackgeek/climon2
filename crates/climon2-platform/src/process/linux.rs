use super::unix::{ProcessList, pid};
use climon2_core::metadata::{LinuxProcessIdentity, SessionMetadata};
use std::{fs, io, num::NonZeroU32};

struct Identity {
    zombie: bool,
    group: u32,
    terminal: u32,
    ticks: u64,
}

fn identity(value: u32) -> io::Result<Option<Identity>> {
    let path = format!("/proc/{}/stat", pid(value)?);
    match fs::read(path) {
        Ok(stat) => parse(&String::from_utf8_lossy(&stat)).map(Some),
        Err(error) if matches!(error.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => {
            absent_process()
        }
        Err(error) => Err(error),
    }
}

fn absent_process() -> io::Result<Option<Identity>> {
    fs::metadata("/proc/self/stat")?;
    Ok(None)
}

pub fn linux_identity(value: u32) -> io::Result<Option<LinuxProcessIdentity>> {
    let Some(info) = identity(value)? else {
        return Ok(None);
    };
    if info.zombie {
        return Ok(None);
    }
    let boot_id = fs::read_to_string("/proc/sys/kernel/random/boot_id")?;
    Ok(Some(LinuxProcessIdentity {
        boot_id: boot_id.trim().to_owned(),
        start_ticks: info.ticks,
    }))
}

pub fn is_same_daemon(metadata: &SessionMetadata) -> io::Result<bool> {
    let expected = metadata
        .daemon_linux_identity
        .as_ref()
        .ok_or_else(|| io::Error::other("Linux daemon identity unavailable"))?;
    Ok(linux_identity(metadata.daemon_pid)?.as_ref() == Some(expected))
}

fn parse(stat: &str) -> io::Result<Identity> {
    // comm is parenthesized, but can itself contain spaces, newlines and parentheses.
    let (_, fields) = stat
        .rsplit_once(") ")
        .ok_or_else(|| io::Error::other("invalid process identity"))?;
    parse_fields(&fields.split_whitespace().collect::<Vec<_>>())
}

fn parse_fields(fields: &[&str]) -> io::Result<Identity> {
    Ok(Identity {
        zombie: field(fields, 0)? == "Z",
        group: number(fields, 2)?,
        terminal: terminal(number(fields, 4)?),
        ticks: ticks(fields)?,
    })
}

fn ticks(fields: &[&str]) -> io::Result<u64> {
    field(fields, 19)?.parse().map_err(io::Error::other)
}

fn field<'a>(fields: &[&'a str], index: usize) -> io::Result<&'a str> {
    fields
        .get(index)
        .copied()
        .ok_or_else(|| io::Error::other("incomplete process identity"))
}

fn number(fields: &[&str], index: usize) -> io::Result<u32> {
    // tty_nr is a signed 32-bit representation of the kernel's device encoding.
    let value: i32 = field(fields, index)?.parse().map_err(io::Error::other)?;
    Ok(value as u32)
}

fn terminal(encoded: u32) -> u32 {
    let major = (encoded >> 8) & 0xfff;
    let minor = (encoded & 0xff) | ((encoded >> 12) & 0xfff00);
    libc::makedev(major, minor) as u32
}

pub fn start_time(value: u32) -> io::Result<Option<u64>> {
    let Some(info) = identity(value)? else {
        return Ok(None);
    };
    if info.zombie {
        return Ok(None);
    }
    started_micros(info.ticks).map(Some)
}

fn started_micros(ticks: u64) -> io::Result<u64> {
    let rate = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    let rate = NonZeroU32::new(u32::try_from(rate).map_err(io::Error::other)?)
        .ok_or_else(|| io::Error::other("invalid clock tick rate"))?;
    let micros = u128::from(boot_seconds()?) * 1_000_000
        + u128::from(ticks) * 1_000_000 / u128::from(rate.get());
    u64::try_from(micros).map_err(io::Error::other)
}

fn boot_seconds() -> io::Result<u64> {
    fs::read_to_string("/proc/stat")?
        .lines()
        .find_map(|line| line.strip_prefix("btime "))
        .ok_or_else(|| io::Error::other("boot time unavailable"))?
        .parse()
        .map_err(io::Error::other)
}

pub(super) fn listed_pids(kind: ProcessList, which: u32) -> io::Result<Vec<i32>> {
    fs::read_dir("/proc")?
        .filter_map(|entry| matching_pid(entry, kind, which).transpose())
        .collect()
}

fn matching_pid(
    entry: io::Result<fs::DirEntry>,
    kind: ProcessList,
    which: u32,
) -> io::Result<Option<i32>> {
    let Some(value) = entry?
        .file_name()
        .to_str()
        .and_then(|name| name.parse().ok())
    else {
        return Ok(None);
    };
    if matches(identity(value)?, kind, which) {
        return pid(value).map(Some);
    }
    Ok(None)
}

fn matches(info: Option<Identity>, kind: ProcessList, which: u32) -> bool {
    let Some(info) = info else {
        return false;
    };
    if info.zombie {
        return false;
    }
    match kind {
        ProcessList::Group => info.group == which,
        ProcessList::Terminal => info.terminal == which,
    }
}
