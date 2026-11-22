#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

/// A stand-in for Cargo: journals each call by stage, sleeps `MUTATION_RUNNER_SLEEP_<stage>`
/// seconds in a test run (never in a `--no-run` build), fails `MUTATION_RUNNER_FAILURE` (a
/// stage, or `build-<stage>` for its build) and
/// hangs `MUTATION_RUNNER_HANG` after recording its process ID.
const CARGO: &str = r#"#!/bin/sh
if [ "$MUTATION_RUNNER_OVERLAY" = yes ]; then
    test -f target/mutation-tests/cache-proof || exit 99
    printf 'private worker' > target/mutation-tests/cache-proof
fi
case " $* " in
    *" --lib "*) stage=unit ;;
    *" --test "*) stage=integration ;;
    *) stage=full ;;
esac
case " $* " in
    *" --no-run "*)
        printf 'build-%s %s\n' "$stage" "$*" >> "$MUTATION_RUNNER_JOURNAL"
        if [ "build-$stage" = "$MUTATION_RUNNER_FAILURE" ]; then exit 1; fi
        exit 0
        ;;
esac
printf '%s %s\n' "$stage" "$*" >> "$MUTATION_RUNNER_JOURNAL"
if [ "$stage" = "$MUTATION_RUNNER_HANG" ]; then
    printf '%s' $$ > "$MUTATION_RUNNER_JOURNAL.hung"
    exec sleep 600
fi
eval "seconds=\${MUTATION_RUNNER_SLEEP_$stage:-0}"
sleep "$seconds"
if [ "$stage" = "$MUTATION_RUNNER_FAILURE" ]; then exit 1; fi
"#;

/// Where the mutator puts a worker overlay holding one private mutated source.
const OVERLAY: &str = "target/mutation-workers/run-proof/worker-0";

/// A scratch root with the warm build workers copy, a stand-in Cargo and its journal.
struct Fixture {
    temp: tempfile::TempDir,
    /// Extra environment for the runner, such as stage durations.
    environment: Vec<(String, String)>,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let baseline = temp.path().join("target/mutation-tests");
        fs::create_dir_all(&baseline).unwrap();
        fs::write(baseline.join("cache-proof"), "unchanged baseline").unwrap();
        let staged = temp.path().join("cargo.staged");
        fs::write(&staged, CARGO).unwrap();
        // Only an external `cp` opens the executable for writing, so no fork inherits a
        // write descriptor for it (ETXTBSY).
        let copied = Command::new("cp")
            .arg(&staged)
            .arg(temp.path().join("cargo"))
            .status()
            .unwrap();
        assert!(copied.success());
        fs::set_permissions(temp.path().join("cargo"), fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            temp,
            environment: Vec::new(),
        }
    }

    fn with(mut self, name: &str, value: &str) -> Self {
        self.environment.push((name.to_owned(), value.to_owned()));
        self
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn journal(&self) -> PathBuf {
        self.root().join("calls")
    }

    /// Writes a crate with a library and the integration test `terminal` under `workspace`.
    fn add_crate(&self, workspace: &Path, name: &str) {
        let package = workspace.join("crates").join(name);
        fs::create_dir_all(package.join("src")).unwrap();
        fs::create_dir_all(package.join("tests")).unwrap();
        fs::write(package.join("src/lib.rs"), "").unwrap();
        fs::write(package.join("Cargo.toml"), "").unwrap();
        fs::write(package.join("tests/terminal.rs"), "").unwrap();
    }

    /// A workspace holding one crate at `relative`, such as [`OVERLAY`].
    fn workspace(&self, relative: &str) -> PathBuf {
        let workspace = self.root().join(relative);
        self.add_crate(&workspace, "example");
        workspace
    }

    /// Hand-written stage timeouts, as a measurement would write them.
    fn timeouts(self, timeouts: &[(&str, f64)]) -> Self {
        let path = self.root().join("stage-timeouts.json");
        let entries: BTreeMap<_, _> = timeouts.iter().copied().collect();
        fs::write(&path, serde_json::to_string(&entries).unwrap()).unwrap();
        let path = path.to_str().unwrap().to_owned();
        self.with("CLIMON2_MUTATION_STAGE_TIMEOUTS", &path)
    }

    fn command(&self, arguments: &[&str], directory: &Path) -> Command {
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("tools/test-mutation.py");
        let mut command = Command::new(
            std::env::var_os("CLIMON2_QUALITY_PYTHON").unwrap_or_else(|| "python3".into()),
        );
        command
            .arg(script)
            .args(arguments)
            .current_dir(directory)
            .env("CLIMON2_MUTATION_CARGO", self.root().join("cargo"))
            .env("MUTATION_RUNNER_JOURNAL", self.journal())
            // A gate runs these tests as part of a mutation baseline, with its own measured
            // timeouts exported; only the timeouts a test writes may reach the runner.
            .env_remove("CLIMON2_MUTATION_STAGE_TIMEOUTS")
            .envs(self.environment.iter().cloned());
        command
    }

    /// Runs a mutant's stages in `workspace` as the mutator does: in a process group of its
    /// own, which a stage timeout kills.
    fn run(&self, workspace: &Path) -> Run {
        let overlay = if workspace.ends_with(OVERLAY) {
            "yes"
        } else {
            "no"
        };
        let mut command = self.command(&[], workspace);
        command.env("MUTATION_RUNNER_OVERLAY", overlay);
        let run = self.finish(command);
        assert_eq!(
            fs::read_to_string(self.root().join("target/mutation-tests/cache-proof")).unwrap(),
            "unchanged baseline",
            "a worker modified the baseline cache"
        );
        run
    }

    /// Measures the stage timeouts of `sources` in the root with `factor`.
    fn measure(&self, factor: &str, sources: &[&str]) -> (Run, PathBuf) {
        let timeouts = self.root().join("measured.json");
        let cargo = self.root().join("cargo");
        let mut arguments = vec![
            self.root().to_str().unwrap(),
            cargo.to_str().unwrap(),
            factor,
            timeouts.to_str().unwrap(),
        ];
        arguments.extend(sources);
        let run = self.finish(self.command(&arguments, self.root()));
        (run, timeouts)
    }

    /// Runs `command` in a process group of its own, bounded by a deadline.
    fn finish(&self, mut command: Command) -> Run {
        let output = self.root().join("output");
        let file = fs::File::create(&output).unwrap();
        let started = Instant::now();
        let mut child = command
            .process_group(0)
            .stdout(Stdio::from(file.try_clone().unwrap()))
            .stderr(Stdio::from(file))
            .spawn()
            .unwrap();
        let deadline = started + Duration::from_secs(60);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() >= deadline {
                kill_group(&child.id().to_string());
                child.wait().unwrap();
                panic!("runner timed out: {}", fs::read_to_string(&output).unwrap());
            }
            thread::sleep(Duration::from_millis(20));
        };
        let seconds = started.elapsed().as_secs_f64();
        let calls = fs::read_to_string(self.journal())
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect();
        Run {
            status,
            seconds,
            output: fs::read_to_string(output).unwrap(),
            calls,
        }
    }

    /// Waits for the stage that hung to be gone; kills its group and fails at the deadline.
    fn assert_hung_stage_gone(&self) {
        let pid = fs::read_to_string(self.journal().with_extension("hung")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while process_exists(&pid) {
            if Instant::now() >= deadline {
                kill_group(&pid);
                panic!("the hung stage {pid} outlived its run");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

struct Run {
    status: ExitStatus,
    seconds: f64,
    output: String,
    calls: Vec<String>,
}

impl Run {
    /// Each call's stage, in order.
    fn stages(&self) -> Vec<&str> {
        self.calls
            .iter()
            .map(|line| line.split_once(' ').unwrap().0)
            .collect()
    }
}

fn process_exists(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn kill_group(pid: &str) {
    Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stderr(Stdio::null())
        .status()
        .unwrap();
}

#[test]
fn killed_mutants_stop_without_running_unrelated_tests() {
    for (failure, stages) in [
        ("unit", vec!["unit"]),
        ("integration", vec!["unit", "integration"]),
    ] {
        let fixture = Fixture::new().with("MUTATION_RUNNER_FAILURE", failure);
        let run = fixture.run(&fixture.workspace(OVERLAY));
        assert_eq!(run.status.code(), Some(1), "{}", run.output);
        assert_eq!(run.stages(), stages);
    }
}

#[test]
fn surviving_fast_tests_always_reach_the_full_workspace() {
    for failure in ["", "full"] {
        let fixture = Fixture::new().with("MUTATION_RUNNER_FAILURE", failure);
        let run = fixture.run(&fixture.workspace(OVERLAY));
        assert_eq!(run.status.success(), failure.is_empty());
        assert_eq!(run.calls.len(), 3, "{:?}", run.calls);
        assert!(run.calls[0].starts_with("unit "));
        assert!(run.calls[1].starts_with("integration "));
        assert!(run.calls[1].ends_with("--test terminal"));
        assert_eq!(
            run.calls[2],
            "full test --workspace --locked --target-dir target/mutation-tests"
        );
    }
}

/// The mutator's baseline runs in the unmutated root: only the full workspace, with no stage
/// timeout, so the mutator's timeout for the whole command stays ten times that run.
#[test]
fn the_unmutated_baseline_still_runs_the_full_workspace() {
    let fixture = Fixture::new()
        .with("MUTATION_RUNNER_FAILURE", "full")
        .timeouts(&[("--lib", 2.0), ("--test terminal", 2.0)]);
    let run = fixture.run(&fixture.workspace("workspace"));
    assert_eq!(run.status.code(), Some(1));
    assert_eq!(
        run.calls,
        ["full test --workspace --locked --target-dir target/mutation-tests"]
    );
}

/// Only a worker of a mutation run is an overlay; any other workspace, even a worker-named
/// one elsewhere under the mutation workers, is unmutated and runs only the full workspace.
#[test]
fn only_a_mutation_runs_worker_is_a_mutant_overlay() {
    for relative in [
        "target/mutation-workers/other/worker-0",
        "target/elsewhere/run-proof/worker-0",
        "target/mutation-workers/run-proof/helper",
    ] {
        let fixture = Fixture::new();
        let run = fixture.run(&fixture.workspace(relative));
        assert!(run.status.success(), "{relative}: {}", run.output);
        assert_eq!(run.stages(), ["full"], "{relative}");
    }
}

/// A worker whose copy of the warm build fails stops before running any test, rather than
/// judge the mutant on a partial build.
#[test]
fn a_failed_warm_build_copy_stops_the_worker() {
    let fixture = Fixture::new();
    let unreadable = fixture.root().join("target/mutation-tests/unreadable");
    fs::write(&unreadable, "").unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();
    let run = fixture.run(&fixture.workspace(OVERLAY));
    assert!(!run.status.success(), "{}", run.output);
    assert_eq!(run.calls, Vec::<String>::new());
}

/// Gate speed-up (#73, 2026-11-16): a test that hangs in an earlier stage kills the mutant at
/// that stage's own timeout, with its whole process group, rather than holding the worker
/// until the mutator's timeout for the whole command; later stages never run.
#[test]
fn a_hung_stage_is_killed_with_its_process_group_at_its_own_timeout() {
    for (hung, limit, stages) in [
        ("unit", 2.0, vec!["build-unit", "unit"]),
        (
            "integration",
            2.5,
            vec!["build-unit", "unit", "build-integration", "integration"],
        ),
    ] {
        let fixture = Fixture::new()
            .with("MUTATION_RUNNER_HANG", hung)
            .timeouts(&[("--lib", 2.0), ("--test terminal", 2.5)]);
        let run = fixture.run(&fixture.workspace(OVERLAY));
        assert!(!run.status.success(), "{hung}: {}", run.output);
        assert_eq!(run.stages(), stages, "{hung}");
        assert!(
            run.seconds >= limit,
            "{hung}: cut early at {}s",
            run.seconds
        );
        assert!(
            run.seconds < limit + 10.0,
            "{hung}: cut at {}s",
            run.seconds
        );
        assert!(
            run.output.contains("stage timeout"),
            "{hung}: {}",
            run.output
        );
        fixture.assert_hung_stage_gone();
    }
}

/// Stages that finish within their own timeouts pass on, and the full workspace, bounded only
/// by the mutator's timeout for the whole command, may run longer than any stage timeout.
#[test]
fn slow_passing_stages_reach_the_full_workspace_which_keeps_the_mutators_timeout() {
    let fixture = Fixture::new()
        .with("MUTATION_RUNNER_SLEEP_unit", "1")
        .with("MUTATION_RUNNER_SLEEP_integration", "1")
        .with("MUTATION_RUNNER_SLEEP_full", "3")
        .timeouts(&[("--lib", 2.0), ("--test terminal", 2.0)]);
    let run = fixture.run(&fixture.workspace(OVERLAY));
    assert!(run.status.success(), "{}", run.output);
    assert_eq!(
        run.stages(),
        [
            "build-unit",
            "unit",
            "build-integration",
            "integration",
            "full"
        ]
    );
    assert_eq!(
        run.calls[4],
        "full test --workspace --locked --target-dir target/mutation-tests"
    );
}

/// Each earlier stage's timeout is the timeout factor times one unloaded run of that stage in
/// the warm build, never under two seconds. Stages that sources share are timed once, and the
/// full workspace is never timed here.
#[test]
fn measurement_times_each_earlier_stage_once_in_the_warm_build() {
    let fixture = Fixture::new().with("MUTATION_RUNNER_SLEEP_unit", "0.3");
    fixture.add_crate(fixture.root(), "example");
    fixture.add_crate(fixture.root(), "other");
    fs::write(fixture.root().join("crates/example/src/more.rs"), "").unwrap();
    let (run, timeouts) = fixture.measure(
        "10",
        &[
            "crates/example/src/lib.rs",
            "crates/example/src/more.rs",
            "crates/other/src/lib.rs",
        ],
    );
    assert!(run.status.success(), "{}", run.output);
    assert_eq!(
        run.calls,
        [
            "build-full test --workspace --locked --target-dir target/mutation-tests --no-run",
            "unit test --workspace --locked --target-dir target/mutation-tests --lib",
            "integration test --workspace --locked --target-dir target/mutation-tests --test terminal",
        ]
    );
    let timeouts: BTreeMap<String, f64> =
        serde_json::from_str(&fs::read_to_string(timeouts).unwrap()).unwrap();
    assert_eq!(
        timeouts.keys().collect::<Vec<_>>(),
        ["--lib", "--test terminal"]
    );
    let library = timeouts["--lib"];
    assert!((3.0..6.0).contains(&library), "{library}");
    assert_eq!(timeouts["--test terminal"], 2.0);
}

/// A warm build or stage that fails unmutated has no baseline, so the measurement fails.
#[test]
fn measurement_fails_when_the_unmutated_build_or_a_stage_fails() {
    for failure in ["build-full", "integration"] {
        let fixture = Fixture::new().with("MUTATION_RUNNER_FAILURE", failure);
        fixture.add_crate(fixture.root(), "example");
        let (run, timeouts) = fixture.measure("10", &["crates/example/src/lib.rs"]);
        assert!(!run.status.success(), "{failure}: {}", run.output);
        assert!(!timeouts.exists(), "{failure}");
    }
}

/// A mutant that does not compile is killed by its stage's build, before any test runs.
#[test]
fn a_failed_stage_build_kills_the_mutant_without_running_tests() {
    let fixture = Fixture::new()
        .with("MUTATION_RUNNER_FAILURE", "build-unit")
        .timeouts(&[("--lib", 2.0)]);
    let run = fixture.run(&fixture.workspace(OVERLAY));
    assert_eq!(run.status.code(), Some(1), "{}", run.output);
    assert_eq!(run.stages(), ["build-unit"]);
}
