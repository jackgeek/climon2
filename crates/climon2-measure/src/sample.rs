//! Process sampling: processor time, resident memory, threads and
//! descriptors of each measured process, read from the operating system
//! (Linux procfs, macOS `proc_pidinfo`), and the window's usage summary.
//! The parsers and the summary are pure.

use std::{collections::BTreeMap, time::Duration};

/// One process at one instant.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Sample {
    /// User and system processor time since the process started.
    pub cpu: Duration,
    /// Resident memory in bytes.
    pub resident: u64,
    pub threads: u32,
    pub descriptors: u32,
}

/// A group of processes sampled at one instant since the window began.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Tick {
    pub at: Duration,
    pub processes: BTreeMap<i32, Sample>,
}

/// A group's usage over a window.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    /// Processor time over wall time, in percent of one processor.
    pub cpu_percent: f64,
    pub resident_mean: u64,
    pub resident_max: u64,
    pub threads_max: u32,
    pub descriptors_max: u32,
}

/// The usage `ticks` show: each process's processor time between its first
/// and last sample over the window's wall time, and the group's summed
/// resident memory, threads and descriptors at each tick.
pub fn usage(ticks: &[Tick]) -> Usage {
    let (Some(first), Some(last)) = (ticks.first(), ticks.last()) else {
        return Usage::default();
    };
    let mut spans: BTreeMap<i32, (Duration, Duration)> = BTreeMap::new();
    for tick in ticks {
        for (&pid, sample) in &tick.processes {
            spans.entry(pid).or_insert((sample.cpu, sample.cpu)).1 = sample.cpu;
        }
    }
    let cpu: Duration = spans
        .values()
        .map(|(start, end)| end.saturating_sub(*start))
        .sum();
    let wall = last.at.saturating_sub(first.at);
    let sums: Vec<Sample> = ticks.iter().map(|tick| total(&tick.processes)).collect();
    let resident_total: u64 = sums.iter().map(|sum| sum.resident).sum();
    Usage {
        cpu_percent: percent(cpu, wall),
        resident_mean: resident_total / sums.len() as u64,
        resident_max: sums.iter().map(|sum| sum.resident).max().unwrap_or(0),
        threads_max: sums.iter().map(|sum| sum.threads).max().unwrap_or(0),
        descriptors_max: sums.iter().map(|sum| sum.descriptors).max().unwrap_or(0),
    }
}

fn percent(cpu: Duration, wall: Duration) -> f64 {
    if wall.is_zero() {
        return 0.0;
    }
    cpu.as_secs_f64() * 100.0 / wall.as_secs_f64()
}

/// The processes' summed resident memory, threads and descriptors.
pub fn total(processes: &BTreeMap<i32, Sample>) -> Sample {
    processes
        .values()
        .fold(Sample::default(), |sum, sample| Sample {
            cpu: sum.cpu + sample.cpu,
            resident: sum.resident + sample.resident,
            threads: sum.threads + sample.threads,
            descriptors: sum.descriptors + sample.descriptors,
        })
}

/// Fields of a Linux `/proc/<pid>/stat` line: processor ticks, threads and
/// resident pages, as `(utime + stime, num_threads, rss)`.
pub fn parse_stat(line: &str) -> Option<(u64, u32, u64)> {
    // The command name may hold spaces and parentheses; it ends at the last ')'.
    let fields: Vec<&str> = line
        .get(line.rfind(')')? + 1..)?
        .split_whitespace()
        .collect();
    // Field n of proc(5) is fields[n - 3] here: state is field 3.
    let field = |n: usize| fields.get(n - 3)?.parse::<u64>().ok();
    let ticks = field(14)? + field(15)?;
    Some((ticks, u32::try_from(field(20)?).ok()?, field(24)?))
}

/// Where a process's resident memory lies, in bytes.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Components {
    /// The process's own executable image.
    pub image: u64,
    /// Client library images.
    pub client_library: u64,
    /// Every other shared library.
    pub other_libraries: u64,
    /// Heap, thread stacks and other anonymous memory.
    pub anonymous: u64,
    /// Other file mappings.
    pub other_files: u64,
    /// Distinct client library files mapped: the retained releases.
    pub client_libraries_mapped: u32,
}

/// The client library's file name prefix in every version folder.
const CLIENT_LIBRARY: &str = "libclimon2_client.";

/// The resident components of a Linux `/proc/<pid>/smaps` for the process
/// whose executable is `image`.
pub fn parse_smaps(smaps: &str, image: &str) -> Components {
    let mut components = Components::default();
    let mut libraries = std::collections::BTreeSet::new();
    let mut path = String::new();
    for line in smaps.lines() {
        if let Some(kilobytes) = line.strip_prefix("Rss:") {
            let bytes = 1024
                * kilobytes
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .unwrap_or(0);
            *component(&mut components, &path, image) += bytes;
        } else if is_mapping(line) {
            path = line.split_whitespace().nth(5).unwrap_or("").to_owned();
            if file_name(&path).starts_with(CLIENT_LIBRARY) {
                libraries.insert(path.clone());
            }
        }
    }
    components.client_libraries_mapped = libraries.len() as u32;
    components
}

/// A mapping header: `start-end perms offset dev inode [path]`.
fn is_mapping(line: &str) -> bool {
    line.split_whitespace().next().is_some_and(|range| {
        range.contains('-') && range.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
    })
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn component<'a>(components: &'a mut Components, path: &str, image: &str) -> &'a mut u64 {
    if path == image {
        &mut components.image
    } else if file_name(path).starts_with(CLIENT_LIBRARY) {
        &mut components.client_library
    } else if file_name(path).contains(".so") || file_name(path).ends_with(".dylib") {
        &mut components.other_libraries
    } else if path.is_empty() || path.starts_with('[') {
        &mut components.anonymous
    } else {
        &mut components.other_files
    }
}

/// Samples `pid`, or none once it has gone.
pub fn sample(pid: i32) -> Option<Sample> {
    platform::sample(pid)
}

/// The resident components of `pid`, where the platform shows them.
pub fn components(pid: i32) -> Option<Components> {
    platform::components(pid)
}

/// `roots` and every live descendant of them.
pub fn descendants(roots: &[i32]) -> Vec<i32> {
    let parents = platform::parents();
    let mut found = roots.to_vec();
    let mut index = 0;
    while let Some(&parent) = found.get(index) {
        found.extend(
            parents
                .iter()
                .filter(|(_, p)| *p == parent)
                .map(|(child, _)| *child),
        );
        index += 1;
    }
    found
}

#[cfg(target_os = "linux")]
mod platform {
    use super::{Components, Sample, parse_smaps, parse_stat};
    use std::{fs, time::Duration};

    fn ticks_per_second() -> u64 {
        // SAFETY: sysconf has no preconditions.
        u64::try_from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }).unwrap_or(100)
    }

    fn page_size() -> u64 {
        // SAFETY: sysconf has no preconditions.
        u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).unwrap_or(4096)
    }

    pub(super) fn sample(pid: i32) -> Option<Sample> {
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let (ticks, threads, pages) = parse_stat(&stat)?;
        let descriptors = fs::read_dir(format!("/proc/{pid}/fd")).ok()?.count();
        Some(Sample {
            cpu: Duration::from_nanos(ticks * 1_000_000_000 / ticks_per_second()),
            resident: pages * page_size(),
            threads,
            descriptors: u32::try_from(descriptors).ok()?,
        })
    }

    pub(super) fn components(pid: i32) -> Option<Components> {
        let smaps = fs::read_to_string(format!("/proc/{pid}/smaps")).ok()?;
        let image = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        Some(parse_smaps(&smaps, &image.to_string_lossy()))
    }

    /// Every process's `(pid, parent pid)`.
    pub(super) fn parents() -> Vec<(i32, i32)> {
        let Ok(entries) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<i32>().ok())
            .filter_map(|pid| {
                let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                let parent = stat
                    .get(stat.rfind(')')? + 1..)?
                    .split_whitespace()
                    .nth(1)?;
                Some((pid, parent.parse().ok()?))
            })
            .collect()
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{Components, Sample};
    use std::{mem, time::Duration};

    fn info<T>(pid: i32, flavor: libc::c_int, value: &mut T) -> Option<usize> {
        let size = libc::c_int::try_from(mem::size_of::<T>()).ok()?;
        // SAFETY: `value` is writable for `size` bytes.
        let written = unsafe { libc::proc_pidinfo(pid, flavor, 0, (value as *mut T).cast(), size) };
        usize::try_from(written).ok().filter(|&written| written > 0)
    }

    #[repr(C)]
    struct Timebase {
        numer: u32,
        denom: u32,
    }

    unsafe extern "C" {
        fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
    }

    /// Mach absolute time units, which task times are counted in, to
    /// nanoseconds.
    fn nanos(units: u64) -> u64 {
        let mut timebase = Timebase { numer: 0, denom: 0 };
        // SAFETY: the argument is a valid, writable timebase.
        unsafe { mach_timebase_info(&mut timebase) };
        if timebase.denom == 0 {
            return units;
        }
        u64::try_from(u128::from(units) * u128::from(timebase.numer) / u128::from(timebase.denom))
            .unwrap_or(u64::MAX)
    }

    pub(super) fn sample(pid: i32) -> Option<Sample> {
        // SAFETY: an all-zero proc_taskinfo is a valid value.
        let mut task: libc::proc_taskinfo = unsafe { mem::zeroed() };
        info(pid, libc::PROC_PIDTASKINFO, &mut task)?;
        let mut descriptors = [libc::proc_fdinfo {
            proc_fd: 0,
            proc_fdtype: 0,
        }; 4096];
        let written = info(pid, libc::PROC_PIDLISTFDS, &mut descriptors)?;
        Some(Sample {
            cpu: Duration::from_nanos(nanos(task.pti_total_user + task.pti_total_system)),
            resident: task.pti_resident_size,
            threads: u32::try_from(task.pti_threadnum).ok()?,
            descriptors: u32::try_from(written / mem::size_of::<libc::proc_fdinfo>()).ok()?,
        })
    }

    /// macOS shows no per-mapping residency without inspecting the task.
    pub(super) fn components(_pid: i32) -> Option<Components> {
        None
    }

    pub(super) fn parents() -> Vec<(i32, i32)> {
        let output = std::process::Command::new("/bin/ps")
            .args(["-A", "-o", "pid=,ppid="])
            .output();
        let Ok(output) = output else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace();
                Some((fields.next()?.parse().ok()?, fields.next()?.parse().ok()?))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tick(at_ms: u64, processes: &[(i32, u64, u64, u32, u32)]) -> Tick {
        Tick {
            at: Duration::from_millis(at_ms),
            processes: processes
                .iter()
                .map(|&(pid, cpu_ms, resident, threads, descriptors)| {
                    (
                        pid,
                        Sample {
                            cpu: Duration::from_millis(cpu_ms),
                            resident,
                            threads,
                            descriptors,
                        },
                    )
                })
                .collect(),
        }
    }

    /// Two processes over two seconds: 500 ms and 100 ms of processor time
    /// are 30% of one processor; resident memory, threads and descriptors
    /// are summed at each tick. A process seen once adds no processor time;
    /// a process that left keeps what it used.
    #[test]
    fn usage_sums_processor_time_and_each_ticks_resident_memory() {
        let ticks = [
            tick(0, &[(1, 1000, 100, 2, 5), (2, 50, 300, 1, 3)]),
            tick(
                1000,
                &[(1, 1200, 200, 3, 6), (2, 150, 300, 1, 3), (3, 7, 50, 1, 1)],
            ),
            tick(2000, &[(1, 1500, 100, 2, 5)]),
        ];
        let window = usage(&ticks);
        assert!((window.cpu_percent - 30.0).abs() < 1e-9, "{window:?}");
        assert_eq!(
            (
                window.resident_mean,
                window.resident_max,
                window.threads_max,
                window.descriptors_max
            ),
            ((400 + 550 + 100) / 3, 550, 5, 10)
        );
        assert_eq!(usage(&[]), Usage::default());
        assert_eq!(usage(&ticks[..1]).cpu_percent, 0.0);
    }

    /// proc(5): utime is field 14, stime 15, num_threads 20 and rss 24;
    /// a command name holding spaces and parentheses does not shift them.
    #[test]
    fn a_stat_line_yields_processor_ticks_threads_and_resident_pages() {
        let line = "4242 (climon2 (mon) x) S 1 4242 4242 0 -1 4194560 300 0 0 0 \
                    17 4 0 0 20 0 9 0 12345 104857600 2560 18446744073709551615";
        assert_eq!(parse_stat(line), Some((21, 9, 2560)));
        assert_eq!(parse_stat("4242 (x) S 1"), None);
        assert_eq!(parse_stat("no parenthesis"), None);
    }

    /// smaps residency by mapping: the executable image, the client
    /// library (two releases mapped), other libraries, anonymous memory
    /// (heap, stacks, unnamed) and other files.
    #[test]
    fn smaps_attributes_resident_memory_to_its_components() {
        let smaps = "\
55d0c0000000-55d0c0100000 r-xp 00000000 08:01 11 /h/0.1.0/climon2-monitor
Size:               1024 kB
Rss:                 800 kB
7f0000000000-7f0000100000 r-xp 00000000 08:01 12 /h/0.1.0/libclimon2_client.so
Rss:                 300 kB
7f0000100000-7f0000200000 r-xp 00000000 08:01 13 /h/0.0.97/libclimon2_client.so
Rss:                 200 kB
7f0000200000-7f0000300000 r-xp 00000000 08:01 14 /usr/lib/libc.so.6
Rss:                  64 kB
55d0c1000000-55d0c2000000 rw-p 00000000 00:00 0 [heap]
Rss:                1000 kB
7f0000300000-7f0000400000 rw-p 00000000 00:00 0
Rss:                 100 kB
7ffd00000000-7ffd00100000 rw-p 00000000 00:00 0 [stack]
Rss:                   8 kB
7f0000400000-7f0000500000 r--p 00000000 08:01 15 /usr/share/locale/x
Rss:                  16 kB
";
        assert_eq!(
            parse_smaps(smaps, "/h/0.1.0/climon2-monitor"),
            Components {
                image: 800 * 1024,
                client_library: 500 * 1024,
                other_libraries: 64 * 1024,
                anonymous: 1108 * 1024,
                other_files: 16 * 1024,
                client_libraries_mapped: 2,
            }
        );
    }

    /// The sampler reads this very process.
    #[test]
    fn the_running_process_samples_itself() {
        let own = sample(std::process::id() as i32).unwrap();
        assert!(
            own.resident > 0 && own.threads >= 1 && own.descriptors >= 3,
            "{own:?}"
        );
        assert!(descendants(&[std::process::id() as i32]).contains(&(std::process::id() as i32)));
        assert_eq!(sample(-1), None);
    }
}
