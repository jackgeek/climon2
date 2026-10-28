use climon2_platform::pty;
use std::{
    env,
    process::{Command, Stdio},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const CHILD: &str = "CLIMON2_PTY_ALLOCATION_CHILD";
const PROCESSES: usize = 32;

fn now_micros() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros()
}

fn allocate_and_hold(start_micros: u128) {
    let wait = start_micros.saturating_sub(now_micros());
    thread::sleep(Duration::from_micros(wait.try_into().unwrap()));
    let (mut master, mut slave) = (-1, -1);
    let result = {
        let _allocation = pty::allocation().unwrap();
        // The pointers refer to live locals; no name, termios or size is requested.
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        }
    };
    assert_eq!(result, 0, "openpty: {}", std::io::Error::last_os_error());
    thread::sleep(Duration::from_millis(200));
    // Both descriptors were returned by the successful openpty above.
    unsafe {
        libc::close(master);
        libc::close(slave);
    }
}

/// #21.5/#38: concurrent climon2 processes never collide in Darwin PTY allocation.
#[test]
fn concurrent_processes_allocate_ptys_without_kernel_races() {
    if let Some(start) = env::var_os(CHILD) {
        allocate_and_hold(start.to_str().unwrap().parse().unwrap());
        return;
    }
    let start = (now_micros() + 1_000_000).to_string();
    let children: Vec<_> = (0..PROCESSES)
        .map(|_| {
            Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "concurrent_processes_allocate_ptys_without_kernel_races",
                ])
                .env(CHILD, &start)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap()
        })
        .collect();
    let failures: Vec<_> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .filter(|output| !output.status.success())
        .map(|output| {
            let text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.lines()
                .find(|line| line.contains("openpty"))
                .unwrap_or(&text)
                .to_owned()
        })
        .collect();
    assert!(failures.is_empty(), "{failures:#?}");
}
