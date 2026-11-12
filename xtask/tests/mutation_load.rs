#![cfg(unix)]

use std::{
    ffi::OsString,
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    directory: tempfile::TempDir,
}

impl Fixture {
    fn new(test: &str) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        fs::create_dir(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"load-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n[workspace]\n",
        )
        .unwrap();
        fs::write(
            root.join("Cargo.lock"),
            "version = 4\n[[package]]\nname = \"load-fixture\"\nversion = \"0.0.0\"\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"), test).unwrap();
        Self { directory }
    }

    fn root(&self) -> &Path {
        self.directory.path()
    }

    fn command(&self, workers: usize, rounds: usize, timeout_factor: &str) -> Command {
        self.configure(Command::new(python()), workers, rounds, timeout_factor)
    }

    /// Gives `command`, which runs Python, the measurement script and its arguments.
    fn configure(
        &self,
        mut command: Command,
        workers: usize,
        rounds: usize,
        timeout_factor: &str,
    ) -> Command {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tools/check-mutation-load.py");
        let forbidden = self.root().join("forbidden-target");
        command
            .arg(script)
            .arg(self.root())
            .arg(env!("CARGO"))
            .args(["--workers", &workers.to_string()])
            .args(["--rounds", &rounds.to_string()])
            .args(["--timeout-factor", timeout_factor])
            .env("CARGO_TARGET_DIR", &forbidden)
            .env("CARGO_BUILD_TARGET_DIR", &forbidden)
            .env("CARGO_BUILD_BUILD_DIR", &forbidden);
        command
    }

    /// Measures with a timeout far beyond any fixture suite's duration under load.
    fn measure(&self, workers: usize, rounds: usize) -> (bool, String) {
        self.measure_with_timeout_factor(workers, rounds, "1000")
    }

    fn measure_with_timeout_factor(
        &self,
        workers: usize,
        rounds: usize,
        timeout_factor: &str,
    ) -> (bool, String) {
        let output = self.root().join("output");
        let mut child = spawn(&mut self.command(workers, rounds, timeout_factor), &output);
        let deadline = Instant::now() + Duration::from_secs(120);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                stop(&mut child);
                panic!(
                    "load measurement timed out: {}",
                    fs::read_to_string(&output).unwrap()
                );
            }
            thread::sleep(Duration::from_millis(20));
        };
        (status.success(), fs::read_to_string(output).unwrap())
    }

    /// Everything a measurement leaves in the workspace's target directory.
    fn target_entries(&self) -> Vec<String> {
        let mut entries: Vec<_> = fs::read_dir(self.root().join("target"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        entries.sort();
        entries
    }

    fn logs(&self) -> Vec<PathBuf> {
        let runs = fs::read_dir(self.root().join("target/flake-logs")).unwrap();
        runs.flat_map(|run| fs::read_dir(run.unwrap().path()).unwrap())
            .map(|entry| entry.unwrap().path())
            .collect()
    }
}

fn python() -> OsString {
    std::env::var_os("CLIMON2_QUALITY_PYTHON").unwrap_or_else(|| "python3".into())
}

/// Starts a measurement in a process group of its own. Its suites start sessions of their
/// own, so signals to that group reach only the measurement.
fn spawn(command: &mut Command, output: &Path) -> Child {
    let file = fs::File::create(output).unwrap();
    command
        .process_group(0)
        .stdout(Stdio::from(file.try_clone().unwrap()))
        .stderr(Stdio::from(file))
        .spawn()
        .unwrap()
}

/// Runs the Python `action` with `pid` bound, unless that process (group) is already gone.
fn signal(action: &str, pid: &str) {
    let script = format!(
        "import os, signal, sys\npid = int(sys.argv[1])\ntry:\n    {action}\nexcept ProcessLookupError:\n    pass"
    );
    let status = Command::new(python())
        .args(["-c", &script])
        .arg(pid)
        .status()
        .unwrap();
    assert!(status.success());
}

/// SIGKILLs the measurement's process group, then reaps the measurement.
fn kill_group(child: &mut Child) {
    signal("os.killpg(pid, signal.SIGKILL)", &child.id().to_string());
    child.wait().unwrap();
}

/// Stops a measurement with SIGTERM, which kills its suites and removes its worker builds;
/// SIGKILLs its process group if it has not exited by the deadline.
fn stop(child: &mut Child) {
    signal("os.kill(pid, signal.SIGTERM)", &child.id().to_string());
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            kill_group(child);
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn process_exists(pid: &str) -> bool {
    Command::new(python())
        .args(["-c", "import os, sys\nos.kill(int(sys.argv[1]), 0)"])
        .arg(pid)
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

/// Waits for each process to exit; SIGKILLs any that outlive the deadline, with any
/// process group they lead, and fails.
fn assert_exited(pids: &[String]) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while pids.iter().any(|pid| process_exists(pid)) {
        if Instant::now() >= deadline {
            for pid in pids {
                signal("os.killpg(pid, signal.SIGKILL)", pid);
                signal("os.kill(pid, signal.SIGKILL)", pid);
            }
            panic!("processes {pids:?} outlived their measurement");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

/// Fixture tests that block in every worker, recording their suite's process group.
const BLOCKED: &str = r#"
#[test]
fn blocked_under_load() {
    use std::{fs, thread, time::{Duration, Instant}};
    let root = std::env::current_dir().unwrap();
    let executable = std::env::current_exe().unwrap();
    let worker = executable.ancestors().nth(3).unwrap().file_name().unwrap();
    if worker == "mutation-tests" {
        return;
    }
    // Cargo leads its suite's session, so Cargo's process ID names the suite's process group.
    let recording = root.join(worker).with_extension("suite");
    fs::write(&recording, std::os::unix::process::parent_id().to_string()).unwrap();
    fs::create_dir_all(root.join("suites")).unwrap();
    fs::rename(recording, root.join("suites").join(worker)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the measurement was not interrupted");
}
"#;

/// Starts `measurement` of the `BLOCKED` fixture with two workers and waits until both
/// block. Returns the measurement and its suites' process groups.
fn start_blocked(fixture: &Fixture, mut measurement: Command) -> (Child, Vec<String>) {
    let output = fixture.root().join("blocked-output");
    let mut child = spawn(&mut measurement, &output);
    let suites = fixture.root().join("suites");
    let deadline = Instant::now() + Duration::from_secs(120);
    while fs::read_dir(&suites).map_or(0, Iterator::count) < 2 {
        if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            stop(&mut child);
            panic!(
                "both workers did not start: {}",
                fs::read_to_string(&output).unwrap()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    let groups = fs::read_dir(&suites)
        .unwrap()
        .map(|entry| fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    (child, groups)
}

#[test]
fn clean_workspace_passes_and_keeps_each_run_log() {
    let fixture = Fixture::new(
        r#"
#[test]
fn clean_fixture() {
    use std::{fs, thread, time::{Duration, Instant}};
    let executable = std::env::current_exe().unwrap();
    let build = executable.ancestors().nth(3).unwrap();
    let worker = build.file_name().unwrap().to_str().unwrap();
    if worker == "mutation-tests" {
        return;
    }
    let previous = fs::read_to_string(build.join("warm-proof")).unwrap();
    assert!(previous == "warm seed" || previous == worker, "{previous}");
    fs::write(build.join("warm-proof"), worker).unwrap();
    let round_file = build.join("round");
    let round = if round_file.exists() {
        fs::read_to_string(&round_file).unwrap().parse::<usize>().unwrap() + 1
    } else {
        1
    };
    fs::write(&round_file, round.to_string()).unwrap();
    let arrivals = std::env::current_dir().unwrap().join(
        format!("{}-round-{round}", build.parent().unwrap().file_name().unwrap().to_str().unwrap())
    );
    fs::create_dir_all(&arrivals).unwrap();
    fs::write(arrivals.join(worker), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(30);
    while fs::read_dir(&arrivals).unwrap().count() != 2 {
        assert!(Instant::now() < deadline, "the two workers did not overlap");
        thread::sleep(Duration::from_millis(10));
    }
}
"#,
    );
    let warm = fixture.root().join("target/mutation-tests");
    fs::create_dir_all(&warm).unwrap();
    fs::write(warm.join("warm-proof"), "warm seed").unwrap();
    let (success, output) = fixture.measure(2, 2);
    assert!(success, "{output}");
    let logs = fixture.logs();
    assert_eq!(logs.len(), 6, "{logs:?}");
    for log in logs
        .iter()
        .filter(|log| log.file_name().unwrap() != "warm.log")
    {
        let text = fs::read_to_string(log).unwrap();
        assert!(text.contains("test clean_fixture ... ok"), "{text}");
        assert!(!text.contains("Compiling load-fixture"), "{text}");
        assert!(text.contains("exit=0"), "{text}");
    }
    assert_eq!(
        fs::read_to_string(warm.join("warm-proof")).unwrap(),
        "warm seed"
    );
    assert!(
        !warm.join("round").exists(),
        "workers shared the warm build"
    );
    assert!(!fixture.root().join("forbidden-target").exists());
    let (success, output) = fixture.measure(2, 2);
    assert!(success, "{output}");
    assert_eq!(
        fixture.logs().len(),
        12,
        "a later check discarded earlier logs"
    );
}

/// Fixture library code: whether this test binary runs in a worker's copy of the warm build.
const UNDER_LOAD: &str = r#"
pub fn under_load() -> bool {
    let executable = std::env::current_exe().unwrap();
    let build = executable.ancestors().nth(3).unwrap();
    build.file_name().unwrap().to_str().unwrap().starts_with("worker-")
}
"#;

#[test]
fn load_sensitive_failures_are_named_concisely_without_retrying() {
    let fixture = Fixture::new(&format!(
        "{UNDER_LOAD}\
         #[test]\nfn first_failure() {{ assert!(!under_load(), \"first defect\"); }}\n\
         #[test]\nfn second_failure() {{ assert!(!under_load(), \"second defect\"); }}\n\
         #[test]\nfn passing_neighbor() {{}}\n"
    ));
    fs::create_dir(fixture.root().join("tests")).unwrap();
    fs::write(
        fixture.root().join("tests/other.rs"),
        "#[test]\nfn integration_failure() { assert!(!load_fixture::under_load(), \"integration defect\"); }\n",
    )
    .unwrap();
    fs::write(
        fixture.root().join("tests/crash.rs"),
        "#[test]\nfn crashing_test() { if load_fixture::under_load() { std::process::abort(); } }\n",
    )
    .unwrap();
    let (success, output) = fixture.measure(2, 3);
    assert!(!success, "{output}");
    for name in [
        "first_failure",
        "second_failure",
        "integration_failure",
        "--test crash",
    ] {
        assert!(output.contains(name), "missing {name}: {output}");
    }
    assert!(
        !output.contains("passing_neighbor"),
        "the report reproduced passing output: {output}"
    );
    let logs = fixture.logs();
    let mut names: Vec<_> = logs
        .iter()
        .map(|log| log.file_name().unwrap().to_str().unwrap())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "baseline.log",
            "r1-worker-1.log",
            "r1-worker-2.log",
            "warm.log"
        ],
        "a failing round must not be retried"
    );
    for log in logs.iter().filter(|log| {
        log.file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("r1-")
    }) {
        let text = fs::read_to_string(log).unwrap();
        assert!(text.contains("first defect"), "{text}");
        assert!(text.contains("second defect"), "{text}");
        assert!(text.contains("integration defect"), "{text}");
        assert!(text.contains("SIGABRT"), "{text}");
        assert!(output.contains(&log.display().to_string()), "{output}");
    }
    assert_eq!(fixture.target_entries(), ["flake-logs", "mutation-tests"]);
}

#[test]
fn the_next_measurement_removes_worker_builds_a_killed_one_left_even_if_it_cannot_build() {
    let fixture = Fixture::new(BLOCKED);
    let (mut child, suites) = start_blocked(&fixture, fixture.command(2, 1, "1000"));
    kill_group(&mut child);
    // Nothing is left to stop suites in sessions of their own after SIGKILL.
    for suite in &suites {
        signal("os.killpg(pid, signal.SIGKILL)", suite);
    }
    assert_exited(&suites);
    fs::write(
        fixture.root().join("src/lib.rs"),
        "compile_error!(\"work in progress\");\n",
    )
    .unwrap();
    let (success, output) = fixture.measure(2, 1);
    assert!(!success, "{output}");
    assert_eq!(
        fixture.target_entries(),
        ["flake-logs", "mutation-tests"],
        "worker builds from the killed measurement remain"
    );
}

#[test]
fn a_terminated_measurement_stops_its_suites_and_removes_their_worker_builds() {
    let fixture = Fixture::new(BLOCKED);
    let (mut child, suites) = start_blocked(&fixture, fixture.command(2, 1, "1000"));
    signal("os.kill(pid, signal.SIGTERM)", &child.id().to_string());
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            kill_group(&mut child);
            panic!("the measurement ignored SIGTERM");
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert!(!status.success());
    assert_exited(&suites);
    assert_eq!(fixture.target_entries(), ["flake-logs", "mutation-tests"]);
}

#[test]
fn a_measurement_started_under_nohup_survives_a_hangup() {
    let fixture = Fixture::new(BLOCKED);
    let mut nohup = Command::new("nohup");
    nohup.arg(python());
    let (mut child, suites) = start_blocked(&fixture, fixture.configure(nohup, 2, 1, "1000"));
    signal("os.kill(pid, signal.SIGHUP)", &child.id().to_string());
    thread::sleep(Duration::from_secs(1));
    let survived = child.try_wait().unwrap().is_none();
    stop(&mut child);
    assert_exited(&suites);
    assert!(survived, "a hangup ended a measurement started under nohup");
}

#[test]
fn a_workspace_that_fails_unloaded_stops_before_loading_workers() {
    for (source, failure, expected) in [
        (
            "compile_error!(\"broken fixture build\");\n",
            "broken fixture build",
            &["warm.log"][..],
        ),
        (
            "#[test]\nfn unloaded_defect() { panic!(\"fails without load\"); }\n",
            "unloaded_defect",
            &["baseline.log", "warm.log"],
        ),
    ] {
        let fixture = Fixture::new(source);
        let (success, output) = fixture.measure(2, 2);
        assert!(!success, "{output}");
        assert!(output.contains(failure), "{output}");
        let mut logs: Vec<_> = fixture
            .logs()
            .iter()
            .map(|log| log.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        logs.sort();
        assert_eq!(logs, expected, "{output}");
    }
}

/// The mutator counts a run that outlives its timeout as a killed mutant, hiding survivors.
#[test]
fn a_suite_outliving_the_mutation_timeout_is_killed_and_names_its_stuck_test() {
    let fixture = Fixture::new(&format!(
        r#"{UNDER_LOAD}
/// libtest's own warning about a test running over 60 seconds, without the wait.
fn warn_running_long(name: &str) {{
    use std::io::Write;
    writeln!(std::io::stdout(), "\ntest {{name}} has been running for over 60 seconds").unwrap();
}}

#[test]
fn slow_but_passing() {{
    if under_load() {{
        warn_running_long("slow_but_passing");
    }}
}}

#[test]
fn hung_under_load() {{
    if !under_load() {{
        return;
    }}
    warn_running_long("hung_under_load");
    let pid = std::env::current_dir().unwrap().join("hung.pid");
    std::fs::write(pid, std::process::id().to_string()).unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}}
"#
    ));
    let (success, output) = fixture.measure_with_timeout_factor(1, 2, "2");
    assert!(!success, "{output}");
    assert!(output.contains("mutation timeout"), "{output}");
    assert!(output.contains("hung_under_load"), "{output}");
    assert!(
        !output.contains("slow_but_passing"),
        "a test that passed was reported as stuck: {output}"
    );
    let logs = fixture.logs();
    let hung = logs
        .iter()
        .find(|log| log.file_name().unwrap() == "r1-worker-1.log")
        .unwrap();
    assert!(output.contains(&hung.display().to_string()), "{output}");
    assert!(
        !logs
            .iter()
            .any(|log| log.file_name().unwrap() == "r2-worker-1.log"),
        "{logs:?}"
    );
    assert_exited(&[fs::read_to_string(fixture.root().join("hung.pid")).unwrap()]);
}
