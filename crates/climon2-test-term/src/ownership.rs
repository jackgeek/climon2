//! Which live processes belong to a test, found without trusting their environment (#21.5).
//!
//! A test owns the terminal roots it spawns, every process carrying its environment marker,
//! every process forked by an owned process image, and every member of a session led by an
//! owned process. macOS supplies kernel unique ids and image versions; Linux supplies pid and
//! start ticks. On both, terminal roots exec through `climon2-test-owned`, so every
//! descendant inherits a test-owned descriptor that any later sample finds: ownership of an
//! orphan never depends on a sample having seen its short-lived parents (#21.5).

#[cfg(target_os = "macos")]
use nix::libc;
use std::{
    collections::{HashMap, HashSet},
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex, MutexGuard, Once},
    thread,
    time::Duration,
};
#[cfg(target_os = "macos")]
use std::{ffi::CStr, mem};

#[cfg(target_os = "linux")]
#[path = "ownership/linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
use linux::{marker_of, pids, process};

pub(crate) const MARKER: &str = "CLIMON2_TEST_ID";
const SAMPLE_INTERVAL: Duration = Duration::from_millis(25);
// <sys/proc_info.h>: PROC_UID_ONLY and PROC_PIDT_BSDINFOWITHUNIQID.
#[cfg(target_os = "macos")]
const USER_ONLY: u32 = 4;
#[cfg(target_os = "macos")]
const BSD_INFO_WITH_UNIQUE_IDS: libc::c_int = 18;
// <sys/proc_info.h>: PROC_PIDFDVNODEINFO.
#[cfg(target_os = "macos")]
const FD_VNODE_INFO: libc::c_int = 1;

/// A file every descendant of a test's terminal roots holds open, inherited across exec,
/// environment clearing, new sessions and reparenting; identified by device and inode.
struct Lease {
    path: PathBuf,
    identity: (u64, u64),
}

impl Lease {
    fn new(directory: &Path) -> Self {
        let path = directory.join("process-ownership");
        let file = fs::File::create(&path).expect("create process ownership lease");
        let metadata = file.metadata().expect("process ownership lease identity");
        Self {
            path,
            identity: (metadata.dev(), metadata.ino()),
        }
    }
}

/// `struct proc_uniqidentifierinfo` from <sys/proc_info.h>.
#[repr(C)]
#[cfg(target_os = "macos")]
struct UniqueIdentifiers {
    uuid: [u8; 16],
    unique: u64,
    parent_unique: u64,
    image: i32,
    parent_image: i32,
    reserved: [u64; 2],
}

/// `struct proc_bsdinfowithuniqid` from <sys/proc_info.h>.
#[repr(C)]
#[cfg(target_os = "macos")]
struct BsdInfoWithUniqueIds {
    bsd: libc::proc_bsdinfo,
    ids: UniqueIdentifiers,
}

/// One live process; zombies are excluded. macOS image ids change on exec, and the parent
/// image id survives reparenting. Linux has no equivalent image ids.
#[derive(Debug)]
struct Process {
    pid: i32,
    parent: i32,
    session: i32,
    unique: (u64, i32),
    image: Option<i32>,
    parent_image: Option<i32>,
    name: String,
    marker: Option<String>,
}

#[derive(Default)]
struct Owned {
    processes: HashSet<(u64, i32)>,
    images: HashSet<i32>,
    sessions: HashSet<i32>,
    lease: Option<Lease>,
}

impl Owned {
    fn take(&mut self, process: &Process) -> bool {
        let mut grew = self.processes.insert(process.unique);
        if let Some(image) = process.image {
            grew |= self.images.insert(image);
        }
        if process.session == process.pid {
            grew |= self.sessions.insert(process.pid);
        }
        grew
    }

    fn claim(&mut self, marker: &str, table: &[Process]) {
        // An empty session is gone, and its id may be reused by a stranger.
        self.sessions
            .retain(|session| table.iter().any(|process| process.session == *session));
        let unique_by_pid: HashMap<_, _> = table.iter().map(|p| (p.pid, p.unique)).collect();
        loop {
            let mut grew = false;
            for process in table {
                let parent_owned = unique_by_pid
                    .get(&process.parent)
                    .is_some_and(|parent| self.processes.contains(parent));
                if self.processes.contains(&process.unique)
                    || parent_owned
                    || process
                        .parent_image
                        .is_some_and(|image| self.images.contains(&image))
                    || self.sessions.contains(&process.session)
                    || process.marker.as_deref() == Some(marker)
                {
                    grew |= self.take(process);
                }
            }
            if !grew {
                return;
            }
        }
    }

    fn live<'a>(&self, table: &'a [Process]) -> Vec<&'a Process> {
        table
            .iter()
            .filter(|process| self.processes.contains(&process.unique))
            .collect()
    }
}

#[derive(Default)]
struct Tracker {
    tests: HashMap<String, Owned>,
    #[cfg(target_os = "macos")]
    markers: HashMap<i32, Option<String>>,
    arguments: Vec<u8>,
}

impl Tracker {
    fn scan(&mut self) -> Vec<Process> {
        let mut table: Vec<_> = pids().into_iter().filter_map(process).collect();
        let leases: HashMap<_, _> = self
            .tests
            .iter()
            .filter_map(|(marker, owned)| {
                owned
                    .lease
                    .as_ref()
                    .map(|lease| (lease.identity, marker.clone()))
            })
            .collect();
        for process in &mut table {
            #[cfg(target_os = "macos")]
            {
                process.marker = self
                    .markers
                    .entry(process.image.expect("macOS process image"))
                    .or_insert_with(|| marker_of(process.pid, &mut self.arguments))
                    .clone()
                    .or_else(|| lease_marker(process.pid, &leases));
            }
            #[cfg(target_os = "linux")]
            {
                process.marker = marker_of(process.pid, &mut self.arguments)
                    .or_else(|| linux::lease_marker(process.pid, &leases));
            }
        }
        #[cfg(target_os = "macos")]
        {
            let images: HashSet<_> = table.iter().filter_map(|process| process.image).collect();
            self.markers.retain(|image, _| images.contains(image));
        }
        for (marker, owned) in &mut self.tests {
            owned.claim(marker, &table);
        }
        table
    }
}

static TRACKER: LazyLock<Mutex<Tracker>> = LazyLock::new(Mutex::default);
static SAMPLER: Once = Once::new();

fn tracker() -> MutexGuard<'static, Tracker> {
    TRACKER.lock().expect("process tracker lock")
}

pub(crate) fn register(marker: &str, directory: &Path) {
    let owned = Owned {
        lease: Some(Lease::new(directory)),
        ..Owned::default()
    };
    tracker().tests.insert(marker.to_owned(), owned);
    SAMPLER.call_once(|| {
        thread::spawn(|| {
            loop {
                {
                    let mut tracker = tracker();
                    if !tracker.tests.is_empty() {
                        tracker.scan();
                    }
                }
                thread::sleep(SAMPLE_INTERVAL);
            }
        });
    });
}

/// Execs a marked terminal root through `climon2-test-owned`, which hands it the test's lease.
pub(crate) fn prepare(mut command: portable_pty::CommandBuilder) -> portable_pty::CommandBuilder {
    let lease = command
        .get_env(MARKER)
        .and_then(|marker| marker.to_str())
        .and_then(|marker| {
            tracker()
                .tests
                .get(marker)
                .and_then(|owned| owned.lease.as_ref().map(|lease| lease.path.clone()))
        });
    if let Some(lease) = lease {
        let mut arguments = vec![
            crate::isolation::bin_path("climon2-test-owned").into_os_string(),
            lease.into_os_string(),
        ];
        arguments.append(command.get_argv_mut());
        *command.get_argv_mut() = arguments;
    }
    command
}

pub(crate) fn forget(marker: &str) {
    tracker().tests.remove(marker);
}

/// Records a terminal root spawned with `marker`; it leads a new session.
pub(crate) fn adopt(marker: &str, pid: i32) {
    let mut tracker = tracker();
    if let Some(owned) = tracker.tests.get_mut(marker) {
        owned.sessions.insert(pid);
        if let Some(root) = process(pid) {
            owned.take(&root);
        }
    }
}

/// The live processes owned by `marker`, as `(pid, name)`.
pub(crate) fn live(marker: &str) -> Vec<(i32, String)> {
    let mut tracker = tracker();
    let table = tracker.scan();
    tracker
        .tests
        .get(marker)
        .map(|owned| {
            owned
                .live(&table)
                .into_iter()
                .map(|process| (process.pid, process.name.clone()))
                .collect()
        })
        .unwrap_or_default()
}

/// Pids of this user's processes; test processes never change user.
#[cfg(target_os = "macos")]
fn pids() -> Vec<i32> {
    let user = nix::unistd::geteuid().as_raw();
    let pid_size = mem::size_of::<i32>();
    loop {
        // A null buffer asks how many bytes the current list needs.
        let needed = unsafe { libc::proc_listpids(USER_ONLY, user, std::ptr::null_mut(), 0) };
        assert!(
            needed > 0,
            "count processes: {}",
            std::io::Error::last_os_error()
        );
        let mut pids = vec![0_i32; needed as usize / pid_size + 64];
        let bytes = i32::try_from(mem::size_of_val(pids.as_slice())).expect("pid buffer size");
        // The buffer holds `bytes` bytes of pids; the result is how many bytes were written.
        let written =
            unsafe { libc::proc_listpids(USER_ONLY, user, pids.as_mut_ptr().cast(), bytes) };
        assert!(
            written >= 0,
            "list processes: {}",
            std::io::Error::last_os_error()
        );
        let count = written as usize / pid_size;
        if count < pids.len() {
            pids.truncate(count);
            pids.retain(|pid| *pid > 0);
            return pids;
        }
    }
}

/// The process, or `None` once it has exited.
#[cfg(target_os = "macos")]
fn process(pid: i32) -> Option<Process> {
    let mut info = mem::MaybeUninit::<BsdInfoWithUniqueIds>::zeroed();
    let size = i32::try_from(mem::size_of::<BsdInfoWithUniqueIds>()).expect("info size");
    // proc_pidinfo writes at most `size` bytes and returns how many it wrote.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            BSD_INFO_WITH_UNIQUE_IDS,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if written != size {
        return None;
    }
    // The kernel filled the whole structure, and every field is plain data.
    let info = unsafe { info.assume_init() };
    // getsid only reads the kernel's process table.
    let session = unsafe { libc::getsid(pid) };
    // pbi_name is a NUL-terminated array the kernel filled.
    let name = unsafe { CStr::from_ptr(info.bsd.pbi_name.as_ptr()) };
    Some(Process {
        pid,
        parent: info.bsd.pbi_ppid as i32,
        session,
        unique: (info.ids.unique, pid),
        image: Some(info.ids.image),
        parent_image: Some(info.ids.parent_image),
        name: name.to_string_lossy().into_owned(),
        marker: None,
    })
}

/// The value of the environment marker this process image was exec'd with.
#[cfg(target_os = "macos")]
fn marker_of(pid: i32, buffer: &mut Vec<u8>) -> Option<String> {
    if buffer.is_empty() {
        buffer.resize(argument_limit(), 0);
    }
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size = buffer.len();
    // sysctl writes at most `size` bytes into the buffer and updates `size`.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if result != 0 {
        return None;
    }
    environment_marker(&buffer[..size])
}

/// KERN_PROCARGS2 layout: argc, the exec path, NUL padding, argv, then the environment.
#[cfg(target_os = "macos")]
fn environment_marker(arguments: &[u8]) -> Option<String> {
    let argc = i32::from_ne_bytes(arguments.get(..4)?.try_into().ok()?);
    let rest = &arguments[4..];
    let rest = &rest[rest.iter().position(|byte| *byte == 0)?..];
    let rest = &rest[rest.iter().position(|byte| *byte != 0)?..];
    let strings = rest.split(|byte| *byte == 0).skip(argc.try_into().ok()?);
    let prefix = format!("{MARKER}=");
    strings
        .take_while(|entry| !entry.is_empty())
        .find_map(|entry| entry.strip_prefix(prefix.as_bytes()))
        .map(|value| String::from_utf8_lossy(value).into_owned())
}

/// The test whose lease this process holds open, if any.
#[cfg(target_os = "macos")]
fn lease_marker(pid: i32, leases: &HashMap<(u64, u64), String>) -> Option<String> {
    if leases.is_empty() {
        return None;
    }
    let entry = mem::size_of::<libc::proc_fdinfo>();
    // A null buffer asks how many bytes the current descriptor list needs.
    let needed =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDLISTFDS, 0, std::ptr::null_mut(), 0) };
    let count = usize::try_from(needed).ok()? / entry + 16;
    let mut descriptors: Vec<libc::proc_fdinfo> = Vec::with_capacity(count);
    let bytes = i32::try_from(count * entry).ok()?;
    // proc_pidinfo writes at most `bytes` bytes of proc_fdinfo records and returns how many.
    let written = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDLISTFDS,
            0,
            descriptors.as_mut_ptr().cast(),
            bytes,
        )
    };
    // The kernel initialised the first `written` bytes, whole records only.
    unsafe { descriptors.set_len(usize::try_from(written).ok()? / entry) };
    descriptors
        .iter()
        .filter(|descriptor| descriptor.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
        .find_map(|descriptor| {
            let identity = vnode_identity(pid, descriptor.proc_fd)?;
            leases.get(&identity).cloned()
        })
}

/// `struct proc_fileinfo` from <sys/proc_info.h>.
#[repr(C)]
#[cfg(target_os = "macos")]
struct FileInfo {
    open_flags: u32,
    status: u32,
    offset: libc::off_t,
    kind: i32,
    guard_flags: u32,
}

/// `struct vnode_fdinfo` from <sys/proc_info.h>.
#[repr(C)]
#[cfg(target_os = "macos")]
struct VnodeDescriptorInfo {
    file: FileInfo,
    vnode: libc::vnode_info,
}

/// Device and inode of the file open at `fd` in `pid`, comparable with `MetadataExt`.
#[cfg(target_os = "macos")]
fn vnode_identity(pid: i32, fd: i32) -> Option<(u64, u64)> {
    let mut info = mem::MaybeUninit::<VnodeDescriptorInfo>::zeroed();
    let size = i32::try_from(mem::size_of::<VnodeDescriptorInfo>()).ok()?;
    // proc_pidfdinfo writes at most `size` bytes and returns how many it wrote.
    let written =
        unsafe { libc::proc_pidfdinfo(pid, fd, FD_VNODE_INFO, info.as_mut_ptr().cast(), size) };
    if written != size {
        return None;
    }
    // The kernel filled the whole structure, and every field is plain data.
    let stat = unsafe { info.assume_init() }.vnode.vi_stat;
    // std widens the signed dev_t to u64; vinfo_stat stores the same bits unsigned.
    Some((stat.vst_dev as i32 as u64, stat.vst_ino))
}

#[cfg(target_os = "macos")]
fn argument_limit() -> usize {
    let mut mib = [libc::CTL_KERN, libc::KERN_ARGMAX];
    let mut limit: libc::c_int = 0;
    let mut size = mem::size_of_val(&limit);
    // sysctl writes one c_int into `limit`.
    let result = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            2,
            (&raw mut limit).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    assert_eq!(
        result,
        0,
        "read kern.argmax: {}",
        std::io::Error::last_os_error()
    );
    usize::try_from(limit).expect("positive kern.argmax")
}
