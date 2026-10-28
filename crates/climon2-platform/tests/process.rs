#![cfg(target_os = "macos")]

use climon2_platform::process;
use std::os::unix::process::ExitStatusExt;
use std::{
    fs::File,
    io::{BufRead, BufReader, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

fn fixture(mode: &str) -> (Child, File) {
    let (child, master, _, _) = reporting_fixture(mode);
    (child, master)
}

/// The terminal device `fd` refers to, as the daemon captures its PTY slave.
fn terminal_of(fd: i32) -> process::Terminal {
    let mut name = [0 as libc::c_char; 128];
    assert_eq!(
        unsafe { libc::ttyname_r(fd, name.as_mut_ptr(), name.len()) },
        0
    );
    let name = unsafe { std::ffi::CStr::from_ptr(name.as_ptr()) };
    process::Terminal::from_path(std::path::Path::new(name.to_str().unwrap())).unwrap()
}

/// Also returns the fixture's PTY slave device and every line it printed before
/// `fixture-ready`. The fixture leads a new session controlled by that PTY.
fn reporting_fixture(mode: &str) -> (Child, File, String, process::Terminal) {
    // `openpty` descriptors are inheritable until marked close-on-exec, so no test thread may
    // fork in that window: a fixture inheriting another test's master would hold it open,
    // delaying that test's hangup until the inheriting fixture dies.
    static FORKS: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let forks = FORKS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let (mut master, mut slave) = (-1, -1);
    let allocation = climon2_platform::pty::allocation().unwrap();
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    drop(allocation);
    let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
    let terminal = terminal_of(slave.as_raw_fd());
    for file in [&master, &slave] {
        assert_ne!(
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) },
            -1
        );
    }
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--ignored",
        "--exact",
        "owned_process_fixture",
        "--nocapture",
    ]);
    command
        .env("CLIMON2_PLATFORM_FIXTURE", mode)
        .stdin(slave)
        .stdout(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::ioctl(0, libc::TIOCSCTTY.into(), 0) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    drop(forks);
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("fixture-ready") {
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
    }
    (child, master, line, terminal)
}

/// A HUP-ignoring child of the fixture in its own process group (a job) or its own session.
fn spawn_member(new_session: bool) -> Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--ignored",
            "--exact",
            "owned_process_fixture",
            "--nocapture",
        ])
        .env("CLIMON2_PLATFORM_FIXTURE", "ignore")
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            let result = if new_session {
                libc::setsid()
            } else {
                libc::setpgid(0, 0)
            };
            if result == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().unwrap();
    let mut reader = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    while !line.contains("fixture-ready") {
        assert_ne!(reader.read_line(&mut line).unwrap(), 0);
    }
    child
}

fn reported(lines: &str, key: &str) -> u32 {
    lines
        .split_whitespace()
        .find_map(|field| field.strip_prefix(key))
        .unwrap()
        .parse()
        .unwrap()
}

fn gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while process::start_time(pid).unwrap().is_some() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

#[test]
#[ignore = "owned subprocess fixture"]
fn owned_process_fixture() {
    let mode = std::env::var("CLIMON2_PLATFORM_FIXTURE").unwrap();
    if mode == "session" {
        let mut members = [spawn_member(false), spawn_member(true)];
        println!("job={} outsider={}", members[0].id(), members[1].id());
        println!("fixture-ready");
        std::io::stdout().flush().unwrap();
        // The test kills this leader first; waiting only keeps the members owned until then.
        for member in &mut members {
            let _ = member.wait();
        }
        std::process::exit(0);
    }
    if mode == "self-kill" {
        let mut job = spawn_member(false);
        process::TerminalSession::new(std::process::id(), terminal_of(0))
            .unwrap()
            .kill()
            .unwrap();
        let status = job.wait().unwrap();
        println!("caller-survived job-signal={:?}", status.signal());
    }
    if mode == "ignore" {
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
    }
    println!("fixture-ready");
    std::io::stdout().flush().unwrap();
    match mode.as_str() {
        "exit" => std::process::exit(37),
        "signal" => unsafe {
            libc::raise(libc::SIGTERM);
        },
        _ => loop {
            std::thread::sleep(Duration::from_secs(60));
        },
    }
}

#[test]
fn issue26_identity_uses_start_time_and_refuses_zero_or_pid_reuse() {
    let (mut child, _master) = fixture("sleep");
    let pid = child.id();
    let started = process::start_time(pid).unwrap().unwrap();
    assert!(started > 1_000_000_000_000_000);
    assert!(process::is_same_process(pid, started).unwrap());
    assert!(!process::is_same_process(pid, started + 1).unwrap());
    assert!(process::start_time(0).is_err());
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(!process::is_same_process(pid, started).unwrap());
}

#[test]
fn issue19_waitpid_preserves_exit_and_real_signal_status() {
    let (mut child, _master) = fixture("exit");
    assert_eq!(process::wait(child.id()).unwrap().code().unwrap(), 37);
    let _ = child.wait();
    let (mut child, _master) = fixture("signal");
    assert_eq!(process::wait(child.id()).unwrap().code().unwrap(), 143);
    let _ = child.wait();
}

#[test]
fn issue19_waitpid_refuses_non_child_instead_of_fabricating_exit_status() {
    let error = process::wait(std::process::id()).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(libc::ECHILD));
}

#[test]
fn issue19_hangup_escalates_group_after_real_five_seconds() {
    let (mut child, master, _, terminal) = reporting_fixture("ignore");
    let started = Instant::now();
    let mut session = process::TerminalSession::new(child.id(), terminal).unwrap();
    let status = process::hangup(&mut session, || drop(master)).unwrap();
    let _ = child.wait();
    assert_eq!(status.code().unwrap(), 137);
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(started.elapsed() < Duration::from_secs(8));
}

/// #19: `signal_group` signals the whole group, tolerates a nonexistent one and refuses
/// out-of-range IDs. (0 and 1 are refused too, but are never passed here: under a mutated
/// guard they would signal the test's own group or every process.)
#[test]
fn issue19_signal_group_signals_the_group_and_tolerates_a_nonexistent_one() {
    let (mut child, _master) = fixture("sleep");
    process::signal_group(child.id(), process::Signal::Terminate).unwrap();
    assert_eq!(process::wait(child.id()).unwrap().code().unwrap(), 143);
    let _ = child.wait();
    // Above macOS's PID range, so no real group can be hit.
    process::signal_group(i32::MAX as u32, process::Signal::Kill).unwrap();
    assert!(process::signal_group(u32::MAX, process::Signal::Kill).is_err());
}

/// #19/#27: the leave policy's kill reaches every job-control process group of the session,
/// even after the leader died (which detaches the terminal), provided the group was observed
/// while the leader lived; never a process that started its own session.
#[test]
fn issue19_kill_session_kills_every_job_but_not_new_sessions() {
    let (mut child, _master, lines, terminal) = reporting_fixture("session");
    let (job, outsider) = (reported(&lines, "job="), reported(&lines, "outsider="));
    assert_ne!(unsafe { libc::getpgid(job as i32) }, child.id() as i32);
    let mut session = process::TerminalSession::new(child.id(), terminal).unwrap();
    session.observe().unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    session.kill().unwrap();
    let job_gone = gone(job);
    let outsider_alive = process::start_time(outsider).unwrap().is_some();
    unsafe {
        libc::kill(job as i32, libc::SIGKILL);
        libc::kill(outsider as i32, libc::SIGKILL);
    }
    assert!(job_gone, "job in another process group survived");
    assert!(outsider_alive, "process in its own session was killed");
    assert!(process::TerminalSession::new(0, terminal).is_err());
}

/// #19/#27: candidates come only from the given terminal, and must also be in the given
/// session. Another session on another PTY (same user) and a process that setsid'd away are
/// never signalled, whichever of the two constraints is mismatched.
#[test]
fn issue19_kill_session_only_signals_session_members_on_the_given_terminal() {
    let (mut first, _first_master, lines, first_terminal) = reporting_fixture("session");
    let (job, outsider) = (reported(&lines, "job="), reported(&lines, "outsider="));
    let (mut second, _second_master, _, second_terminal) = reporting_fixture("sleep");
    let alive = |pid: u32| process::start_time(pid).unwrap().is_some();
    let session = |leader: u32, terminal| process::TerminalSession::new(leader, terminal).unwrap();
    session(second.id(), first_terminal).kill().unwrap();
    session(first.id(), second_terminal).kill().unwrap();
    let mismatched_survived = [first.id(), job, outsider, second.id()].map(alive);
    session(first.id(), first_terminal).kill().unwrap();
    let first_signal = first.wait().unwrap().signal();
    let job_gone = gone(job);
    let others_survived = [outsider, second.id()].map(alive);
    let _ = second.kill();
    let _ = second.wait();
    unsafe {
        libc::kill(job as i32, libc::SIGKILL);
        libc::kill(outsider as i32, libc::SIGKILL);
    }
    assert_eq!(
        mismatched_survived, [true; 4],
        "a mismatched kill signalled something"
    );
    assert_eq!(first_signal, Some(libc::SIGKILL));
    assert!(job_gone, "job of the session survived");
    assert_eq!(others_survived, [true; 2], "a non-member was signalled");
}

/// #19/#27: candidates come only from the kernel's terminal and recorded-group lists, never
/// from every process: a real session member that the detached terminal no longer lists, and
/// whose group was never observed, is not signalled even though `getsid` matches.
#[test]
fn issue19_kill_session_never_enumerates_beyond_terminal_and_observed_groups() {
    let (mut child, _master, lines, terminal) = reporting_fixture("session");
    let (job, outsider) = (reported(&lines, "job="), reported(&lines, "outsider="));
    child.kill().unwrap();
    child.wait().unwrap();
    let member = unsafe { libc::getsid(job as i32) } == child.id() as i32;
    process::TerminalSession::new(child.id(), terminal)
        .unwrap()
        .kill()
        .unwrap();
    let job_alive = process::start_time(job).unwrap().is_some();
    unsafe {
        libc::kill(job as i32, libc::SIGKILL);
        libc::kill(outsider as i32, libc::SIGKILL);
    }
    assert!(member, "the job must still be a session member");
    assert!(job_alive, "an unlisted, unobserved member was signalled");
}

/// #19: only a character device can name the terminal that scopes a session kill.
#[test]
fn issue19_terminal_must_be_a_character_device() {
    let file = tempfile::NamedTempFile::new_in(env!("CARGO_MANIFEST_DIR")).unwrap();
    assert!(process::Terminal::from_path(file.path()).is_err());
    assert!(
        process::Terminal::from_path(std::path::Path::new("/nonexistent-climon2-tty")).is_err()
    );
}

/// #19: a session member ending its own session kills the other members but not itself.
#[test]
fn issue19_kill_session_spares_the_calling_process() {
    let (mut child, _master, lines, _) = reporting_fixture("self-kill");
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        lines.contains("caller-survived job-signal=Some(9)"),
        "{lines}"
    );
}

#[test]
fn issue19_closing_real_controlling_pty_delivers_hangup() {
    let (mut child, master, _, terminal) = reporting_fixture("sleep");
    let mut session = process::TerminalSession::new(child.id(), terminal).unwrap();
    let status = process::hangup(&mut session, || drop(master)).unwrap();
    let _ = child.wait();
    assert_eq!(status.code().unwrap(), 129);
}

#[test]
fn issue15_bad_cwd_is_rejected_before_pty_can_fall_back() {
    assert!(process::validate_cwd(std::path::Path::new("/nonexistent-climon2-cwd")).is_err());
    let cwd = std::env::current_dir().unwrap();
    process::validate_cwd(&cwd).unwrap();
}

/// #15: a cwd that is a file, or a directory the user cannot search, is rejected up front.
#[test]
fn issue15_file_and_unsearchable_cwd_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file");
    std::fs::write(&file, b"").unwrap();
    let error = process::validate_cwd(&file).unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::NotADirectory);
    let locked = directory.path().join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o600)).unwrap();
    let error = process::validate_cwd(&locked).unwrap_err();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error.raw_os_error(), Some(libc::EACCES));
}
