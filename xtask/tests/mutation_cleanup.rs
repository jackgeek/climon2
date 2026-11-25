//! A mutant's tests can start a daemon that leaves their session and outlives its parent,
//! as `climon2-daemon` does. Such a daemon once held the mutator's output pipe after a run
//! ended, so the mutator waited for it for hours (#401). `tools/test-mutation.py` must end
//! a hung run at its timeout, and either way leave no process behind.

use std::{
    env, fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// Set for the stand-in daemon, which then never ends by itself.
const STAND_IN: &str = "MUTATION_CLEANUP_STAND_IN";

/// A stand-in for Cargo. Its test run starts the stand-in daemon through a process that
/// exits at once, so the daemon has no live parent; on Unix the daemon leads a session of
/// its own and keeps the run's output, as `climon2-daemon` keeps its stderr. Then the run
/// hangs when `MUTATION_CLEANUP_MODE` is `hang`. When it is `reap`, the run kills the
/// daemon and fails unless it is gone within five seconds, as `climon2-fixtures`' leak
/// tests wait for the orphans they kill. Otherwise the run passes.
const CARGO: &str = r#"
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

if "--no-run" in sys.argv:
    raise SystemExit(0)
if sys.argv[1:] == ["start"]:
    if os.name == "nt":
        options = {"creationflags": 0x00000008 | 0x00000200}  # DETACHED_PROCESS, new group
    else:
        options = {"start_new_session": True}
    daemon = subprocess.Popen(
        [os.environ["MUTATION_CLEANUP_DAEMON"], "--exact", "stand_in_daemon", "--ignored"],
        **options,
    )
    Path("daemon.pid").write_text(str(daemon.pid))
    raise SystemExit(0)
subprocess.run([sys.executable, __file__, "start"], check=True)
if os.environ["MUTATION_CLEANUP_MODE"] == "hang":
    time.sleep(600)
if os.environ["MUTATION_CLEANUP_MODE"] == "reap":
    daemon = int(Path("daemon.pid").read_text())
    os.kill(daemon, signal.SIGKILL)
    deadline = time.monotonic() + 5
    while True:
        try:
            # Succeeds for a zombie, as Rust's `kill(pid, None)` does.
            os.kill(daemon, 0)
        except ProcessLookupError:
            break
        if time.monotonic() > deadline:
            raise SystemExit(f"killed orphan {daemon} survived")
        time.sleep(0.01)
"#;

/// Not a test: the stand-in daemon, a copy of this binary, run with `STAND_IN` set.
#[test]
#[ignore = "the stand-in daemon that the tests below start"]
fn stand_in_daemon() {
    if env::var_os(STAND_IN).is_some() {
        loop {
            thread::sleep(Duration::from_secs(60));
        }
    }
}

fn python() -> PathBuf {
    env::var_os("CLIMON2_QUALITY_PYTHON")
        .unwrap_or_else(|| if cfg!(windows) { "python" } else { "python3" }.into())
        .into()
}

/// A workspace whose warm build holds the stand-in daemon's executable, and a Cargo that
/// starts it.
struct Fixture {
    temp: tempfile::TempDir,
    cargo: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let build = temp.path().join("workspace/target/mutation-tests");
        fs::create_dir_all(&build).unwrap();
        fs::copy(
            env::current_exe().unwrap(),
            build.join(format!("stand-in-daemon{}", env::consts::EXE_SUFFIX)),
        )
        .unwrap();
        let script = temp.path().join("cargo.py");
        fs::write(&script, CARGO).unwrap();
        let cargo = cargo_running(&script);
        Self { temp, cargo }
    }

    fn workspace(&self) -> PathBuf {
        self.temp.path().join("workspace")
    }

    /// A mutant's run in `mode`, under `timeouts` when given, with its output piped as the
    /// mutator pipes it.
    fn run(&self, mode: &str, timeouts: Option<&str>) -> Run {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tools/test-mutation.py");
        let daemon = self.workspace().join(format!(
            "target/mutation-tests/stand-in-daemon{}",
            env::consts::EXE_SUFFIX
        ));
        let mut command = Command::new(python());
        command
            .arg(script)
            .current_dir(self.workspace())
            .env("CLIMON2_MUTATION_CARGO", &self.cargo)
            .env("MUTATION_CLEANUP_DAEMON", daemon)
            .env("MUTATION_CLEANUP_MODE", mode)
            .env(STAND_IN, "1")
            .env_remove("CLIMON2_MUTATION_SUITE")
            .env_remove("CLIMON2_MUTATION_STAGE_TIMEOUTS")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(timeouts) = timeouts {
            let path = self.temp.path().join("stage-timeouts.json");
            fs::write(&path, timeouts).unwrap();
            command.env("CLIMON2_MUTATION_STAGE_TIMEOUTS", path);
        }
        in_own_group(&mut command);
        let started = Instant::now();
        let mut child = command.spawn().unwrap();
        let output = drain(&mut child);
        let status = wait(&mut child, started + Duration::from_secs(60));
        let seconds = started.elapsed().as_secs_f64();
        // The mutator reads until every holder of its pipe has closed it.
        let output = output.recv_timeout(Duration::from_secs(10));
        let daemon = fs::read_to_string(self.workspace().join("daemon.pid")).unwrap();
        let daemon_survived = !gone(&daemon);
        let status = status.expect("the run outlived its deadline");
        let output = output.unwrap_or_else(|_| {
            panic!("a process still holds the run's output; daemon {daemon} survived: {daemon_survived}")
        });
        assert!(
            !daemon_survived,
            "the stand-in daemon {daemon} outlived the run: {output}"
        );
        Run {
            status,
            seconds,
            output,
        }
    }
}

struct Run {
    status: ExitStatus,
    seconds: f64,
    output: String,
}

/// Reads the child's stdout and stderr to their ends, on threads, and sends both once done.
fn drain(child: &mut Child) -> mpsc::Receiver<String> {
    let (sender, receiver) = mpsc::channel();
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    thread::spawn(move || {
        let errors = thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = sender.send(text + &errors.join().unwrap());
    });
    receiver
}

/// The child's status, or None once it is killed at `deadline`.
fn wait(child: &mut Child, deadline: Instant) -> Option<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            return None;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Waits up to ten seconds for `pid` to be gone, and kills it if it is not.
fn gone(pid: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while exists(pid) {
        if Instant::now() >= deadline {
            terminate(pid);
            return false;
        }
        thread::sleep(Duration::from_millis(50));
    }
    true
}

#[cfg(unix)]
fn in_own_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

#[cfg(windows)]
fn in_own_group(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    command.creation_flags(0x0000_0200); // CREATE_NEW_PROCESS_GROUP
}

/// An executable beside the Python `script` that runs it with its arguments.
#[cfg(unix)]
fn cargo_running(script: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let cargo = script.with_extension("");
    let staged = script.with_extension("staged");
    let text = format!(
        "#!/bin/sh\nexec \"{}\" \"{}\" \"$@\"\n",
        python().display(),
        script.display()
    );
    fs::write(&staged, text).unwrap();
    fs::set_permissions(&staged, fs::Permissions::from_mode(0o755)).unwrap();
    // Only an external `cp` opens the executable for writing, so no fork inherits a write
    // descriptor for it (ETXTBSY).
    assert!(
        Command::new("cp")
            .arg(&staged)
            .arg(&cargo)
            .status()
            .unwrap()
            .success()
    );
    cargo
}

/// A batch file beside the Python `script` that runs it with its arguments.
#[cfg(windows)]
fn cargo_running(script: &Path) -> PathBuf {
    let cargo = script.with_extension("cmd");
    let text = format!(
        "@\"{}\" \"{}\" %*\r\n",
        python().display(),
        script.display()
    );
    fs::write(&cargo, text).unwrap();
    cargo
}

#[cfg(unix)]
fn exists(pid: &str) -> bool {
    let status = Command::new("ps")
        .args(["-o", "stat=", "-p", pid])
        .output()
        .unwrap();
    // A killed process may stay a zombie until its new parent reaps it.
    status.status.success()
        && !String::from_utf8_lossy(&status.stdout)
            .trim()
            .starts_with('Z')
}

#[cfg(unix)]
fn terminate(pid: &str) {
    Command::new("kill").args(["-KILL", pid]).status().unwrap();
}

#[cfg(windows)]
fn exists(pid: &str) -> bool {
    let listing = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&listing.stdout).contains(&format!("\"{pid}\""))
}

#[cfg(windows)]
fn terminate(pid: &str) {
    Command::new("taskkill")
        .args(["/F", "/PID", pid])
        .status()
        .unwrap();
}

/// #401: a run that hangs ends at the suite's timeout, counts as killed, and takes the
/// detached daemon its tests started with it.
#[test]
fn a_hung_run_ends_at_its_timeout_with_every_process_it_started() {
    let fixture = Fixture::new();
    let run = fixture.run("hang", Some(r#"{"suite": 3.0}"#));
    assert_eq!(run.status.code(), Some(124), "{}", run.output);
    assert!(run.seconds >= 3.0, "cut early at {}s", run.seconds);
    assert!(run.seconds < 20.0, "cut at {}s", run.seconds);
    assert!(
        run.output
            .contains("`--workspace` outlived its 3.0s stage timeout"),
        "{}",
        run.output
    );
}

/// #401: a run whose tests pass but leave a detached daemon behind passes, with the daemon
/// gone; with no stage timeouts, as the gate's own probes run, too.
#[test]
fn a_passing_run_leaves_no_process_behind() {
    for timeouts in [Some(r#"{"suite": 60.0}"#), None] {
        let fixture = Fixture::new();
        let run = fixture.run("pass", timeouts);
        assert!(run.status.success(), "{timeouts:?}: {}", run.output);
        assert!(run.seconds < 20.0, "{timeouts:?}: took {}s", run.seconds);
    }
}

/// #401: a test may kill a process it orphaned and wait until it is gone, as
/// `climon2-fixtures`' leak tests do. On Linux the script adopts orphans, so it must reap
/// them while the tests run, or the killed orphan stays a zombie and the test fails; with
/// no stage timeouts, as the gate's measurement runs, too.
#[cfg(unix)]
#[test]
fn an_orphan_the_tests_kill_is_reaped_while_they_run() {
    for timeouts in [Some(r#"{"suite": 60.0}"#), None] {
        let run = Fixture::new().run("reap", timeouts);
        assert!(run.status.success(), "{timeouts:?}: {}", run.output);
    }
}
