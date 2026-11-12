//! Forked children inherit every descriptor of the test process, including pipe
//! ends that other tests close to provoke write failures, so this test runs in its
//! own test binary.
mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use support::Workspace;

/// #33: an install releases its lock on return, even while another thread's
/// forked child still shares the lock descriptor before exec.
#[test]
fn completed_install_releases_its_lock_despite_concurrent_forks() {
    let workspace = Workspace::new();
    let source = workspace.payload("payload", "1.2.3", 17);
    let payload = climon2_install::Payload::from_directory(&source, "1.2.3").unwrap();
    let destination = workspace.root.join("installed");
    let stop = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !stop.load(Ordering::Relaxed) {
                // The child only sleeps and exits: both are async-signal-safe.
                let pid = unsafe { libc::fork() };
                if pid == 0 {
                    let delay = libc::timespec {
                        tv_sec: 0,
                        tv_nsec: 20_000_000,
                    };
                    unsafe {
                        libc::nanosleep(&delay, std::ptr::null_mut());
                        libc::_exit(0);
                    }
                }
                assert!(pid > 0, "fork: {}", std::io::Error::last_os_error());
                let mut status = 0;
                unsafe { libc::waitpid(pid, &mut status, 0) };
            }
        });
        let result =
            (0..200).try_for_each(|_| climon2_install::install(&payload, &destination).map(|_| ()));
        stop.store(true, Ordering::Relaxed);
        result.unwrap();
    });
}
