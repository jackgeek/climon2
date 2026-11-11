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

    fn command(&self, workers: usize, rounds: usize) -> Command {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tools/check-mutation-load.py");
        let forbidden = self.root().join("forbidden-target");
        let mut command = Command::new(python());
        command
            .arg(script)
            .arg(self.root())
            .arg(env!("CARGO"))
            .arg(workers.to_string())
            .arg(rounds.to_string())
            .env("CARGO_TARGET_DIR", &forbidden)
            .env("CARGO_BUILD_TARGET_DIR", &forbidden)
            .env("CARGO_BUILD_BUILD_DIR", &forbidden);
        command
    }

    fn measure(&self, workers: usize, rounds: usize) -> (bool, String) {
        let output = self.root().join("output");
        let mut child = spawn(&mut self.command(workers, rounds), &output);
        let deadline = Instant::now() + Duration::from_secs(120);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                kill_group(&mut child);
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

/// Starts a measurement as its own process group, so its whole tree can be killed.
fn spawn(command: &mut Command, output: &Path) -> Child {
    let file = fs::File::create(output).unwrap();
    command
        .process_group(0)
        .stdout(Stdio::from(file.try_clone().unwrap()))
        .stderr(Stdio::from(file))
        .spawn()
        .unwrap()
}

/// SIGKILLs the measurement, its Cargo runs and their tests, then reaps the measurement.
fn kill_group(child: &mut Child) {
    let status = Command::new(python())
        .args([
            "-c",
            "import os, signal, sys\ntry:\n    os.killpg(int(sys.argv[1]), signal.SIGKILL)\nexcept ProcessLookupError:\n    pass",
        ])
        .arg(child.id().to_string())
        .status()
        .unwrap();
    assert!(status.success());
    child.wait().unwrap();
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
    assert_eq!(logs.len(), 5, "{logs:?}");
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
        10,
        "a later check discarded earlier logs"
    );
}

#[test]
fn failing_workspace_names_each_failure_concisely_without_retrying() {
    let fixture = Fixture::new(
        "#[test]\nfn first_failure() { panic!(\"first defect\"); }\n\
         #[test]\nfn second_failure() { panic!(\"second defect\"); }\n\
         #[test]\nfn passing_neighbor() {}\n",
    );
    fs::create_dir(fixture.root().join("tests")).unwrap();
    fs::write(
        fixture.root().join("tests/other.rs"),
        "#[test]\nfn integration_failure() { panic!(\"integration defect\"); }\n",
    )
    .unwrap();
    fs::write(
        fixture.root().join("tests/crash.rs"),
        "#[test]\nfn crashing_test() { std::process::abort(); }\n",
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
    assert_eq!(
        logs.len(),
        3,
        "a failing round must not be retried: {logs:?}"
    );
    for log in logs
        .iter()
        .filter(|log| log.file_name().unwrap() != "warm.log")
    {
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
fn an_interrupted_measurement_leaves_no_worker_builds_after_the_next() {
    let fixture = Fixture::new(
        r#"
#[test]
fn interrupted_fixture() {
    use std::{fs, thread, time::{Duration, Instant}};
    let root = std::env::current_dir().unwrap();
    if root.join("released").exists() {
        return;
    }
    let executable = std::env::current_exe().unwrap();
    let worker = executable.ancestors().nth(3).unwrap().file_name().unwrap();
    fs::create_dir_all(root.join("arrivals")).unwrap();
    fs::write(root.join("arrivals").join(worker), "").unwrap();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    panic!("the measurement was not interrupted");
}
"#,
    );
    let output = fixture.root().join("interrupted-output");
    let mut child = spawn(&mut fixture.command(2, 1), &output);
    let arrivals = fixture.root().join("arrivals");
    let deadline = Instant::now() + Duration::from_secs(120);
    while fs::read_dir(&arrivals).map_or(0, Iterator::count) < 2 {
        if Instant::now() >= deadline || child.try_wait().unwrap().is_some() {
            kill_group(&mut child);
            panic!(
                "both workers did not start: {}",
                fs::read_to_string(&output).unwrap()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
    kill_group(&mut child);
    fs::write(fixture.root().join("released"), "").unwrap();
    let (success, output) = fixture.measure(2, 1);
    assert!(success, "{output}");
    assert_eq!(
        fixture.target_entries(),
        ["flake-logs", "mutation-tests"],
        "worker builds from the interrupted measurement remain"
    );
}

#[test]
fn a_broken_warm_build_fails_without_starting_workers() {
    let fixture = Fixture::new("compile_error!(\"broken fixture build\");\n");
    let (success, output) = fixture.measure(2, 2);
    assert!(!success, "{output}");
    assert!(output.contains("broken fixture build"), "{output}");
    let logs = fixture.logs();
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert_eq!(logs[0].file_name().unwrap(), "warm.log");
}
