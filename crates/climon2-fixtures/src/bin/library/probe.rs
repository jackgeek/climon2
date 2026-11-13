//! What the host observes outside the function table: the process state a
//! library must never change, and the borrowed notification descriptor.

use serde_json::{Value, json};
use std::{
    io,
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    time::{Duration, Instant},
};

/// Every signal disposition, the calling thread's signal mask, the
/// environment, the working directory and the standard streams.
pub fn ambient() -> Value {
    json!({
        "environment": std::env::vars_os()
            .map(|(name, value)| format!("{}={}", name.to_string_lossy(), value.to_string_lossy()))
            .collect::<Vec<_>>(),
        "cwd": std::env::current_dir().map(|path| path.as_os_str().as_bytes().to_vec()).ok(),
        "signals": (1..signals()).map(disposition).collect::<Vec<_>>(),
        "mask": mask(),
        "streams": (0..3).map(stream).collect::<Vec<_>>(),
    })
}

#[cfg(target_os = "linux")]
fn signals() -> i32 {
    libc::SIGRTMAX() + 1
}

#[cfg(target_os = "macos")]
fn signals() -> i32 {
    32
}

fn disposition(signal: i32) -> Value {
    let mut action = MaybeUninit::<libc::sigaction>::zeroed();
    // SAFETY: a null new action only reads the current one into `action`.
    if unsafe { libc::sigaction(signal, std::ptr::null(), action.as_mut_ptr()) } == -1 {
        return json!({ "signal": signal, "error": io::Error::last_os_error().raw_os_error() });
    }
    // SAFETY: sigaction succeeded and filled `action`.
    let action = unsafe { action.assume_init() };
    json!({
        "signal": signal,
        "handler": action.sa_sigaction,
        "flags": action.sa_flags,
    })
}

fn mask() -> Vec<i32> {
    let mut set = MaybeUninit::<libc::sigset_t>::zeroed();
    // SAFETY: a null new set only reads the calling thread's mask.
    unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, std::ptr::null(), set.as_mut_ptr()) };
    // SAFETY: pthread_sigmask filled the set.
    let set = unsafe { set.assume_init() };
    // SAFETY: sigismember only reads the set.
    (1..signals())
        .filter(|signal| unsafe { libc::sigismember(&set, *signal) } == 1)
        .collect()
}

fn stream(descriptor: i32) -> Value {
    let mut stat = MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: fstat only writes the supplied stat.
    let status = unsafe { libc::fstat(descriptor, stat.as_mut_ptr()) };
    // SAFETY: zeroed or filled by fstat.
    let stat = unsafe { stat.assume_init() };
    json!({
        "descriptor": descriptor,
        "status": status,
        "device": stat.st_dev,
        "inode": stat.st_ino,
        "mode": stat.st_mode,
        // SAFETY: fcntl queries take no pointers.
        "status_flags": unsafe { libc::fcntl(descriptor, libc::F_GETFL) },
        // SAFETY: as above.
        "descriptor_flags": unsafe { libc::fcntl(descriptor, libc::F_GETFD) },
    })
}

/// Whether the descriptor is readable now, without waiting.
pub fn readable(descriptor: i32) -> bool {
    poll(descriptor, 0)
}

fn poll(descriptor: i32, milliseconds: i32) -> bool {
    let mut entry = libc::pollfd {
        fd: descriptor,
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd entry.
    let ready = unsafe { libc::poll(&mut entry, 1, milliseconds) };
    ready == 1 && entry.revents & libc::POLLIN != 0
}

/// Polls until readable or `deadline`; returns whether it became readable.
pub fn wait_readable(descriptor: i32, deadline: Instant) -> bool {
    while Instant::now() < deadline {
        if poll(descriptor, 50) {
            return true;
        }
    }
    false
}

/// The descriptor's open-file properties, as the library leaves them.
pub fn descriptor(descriptor: i32) -> Value {
    // SAFETY (both): fcntl queries take no pointers.
    let status = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFD) };
    json!({
        "open": flags != -1,
        "nonblocking": status != -1 && status & libc::O_NONBLOCK != 0,
        "close_on_exec": flags != -1 && flags & libc::FD_CLOEXEC != 0,
        "kind": kind(descriptor),
    })
}

/// What the descriptor refers to: Linux names an eventfd `anon_inode:[eventfd]`
/// and reports its counter in fdinfo.
#[cfg(target_os = "linux")]
fn kind(descriptor: i32) -> Value {
    let link = std::fs::read_link(format!("/proc/self/fd/{descriptor}")).ok();
    let info = std::fs::read_to_string(format!("/proc/self/fdinfo/{descriptor}")).ok();
    json!({
        "link": link.map(|link| link.to_string_lossy().into_owned()),
        "eventfd_count": info.and_then(|info| {
            info.lines()
                .find_map(|line| line.strip_prefix("eventfd-count:").map(|count| count.trim().to_owned()))
        }),
    })
}

#[cfg(target_os = "macos")]
fn kind(descriptor: i32) -> Value {
    let mut stat = MaybeUninit::<libc::stat>::zeroed();
    // SAFETY: fstat only writes the supplied stat.
    let status = unsafe { libc::fstat(descriptor, stat.as_mut_ptr()) };
    // SAFETY: zeroed or filled by fstat.
    let mode = unsafe { stat.assume_init() }.st_mode;
    json!({ "fifo": status == 0 && mode & libc::S_IFMT == libc::S_IFIFO })
}

/// The open descriptors, lowest first.
pub fn open_descriptors() -> Vec<i32> {
    // SAFETY: fcntl queries take no pointers.
    (0..1024)
        .filter(|descriptor| unsafe { libc::fcntl(*descriptor, libc::F_GETFD) } != -1)
        .collect()
}

/// Lowers the descriptor limit so that no new descriptor can be created, runs
/// `work`, and restores the limit.
pub fn without_descriptors<T>(work: impl FnOnce() -> T) -> T {
    let mut limit = MaybeUninit::<libc::rlimit>::zeroed();
    // SAFETY: getrlimit only writes the supplied rlimit.
    assert_eq!(
        unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, limit.as_mut_ptr()) },
        0
    );
    // SAFETY: filled by getrlimit.
    let original = unsafe { limit.assume_init() };
    let lowest_free = (0..)
        // SAFETY: fcntl queries take no pointers.
        .find(|descriptor| unsafe { libc::fcntl(*descriptor, libc::F_GETFD) } == -1)
        .expect("a free descriptor number");
    let exhausted = libc::rlimit {
        rlim_cur: lowest_free as libc::rlim_t,
        rlim_max: original.rlim_max,
    };
    // SAFETY: setrlimit reads the supplied rlimit.
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &exhausted) },
        0
    );
    let result = work();
    // SAFETY: as above.
    assert_eq!(
        unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &original) },
        0
    );
    result
}

/// The calling thread's deadline for anything the host waits on.
pub fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}
