#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};

fn exercise(failure: &str, overlay: bool) -> (std::process::Output, Vec<String>) {
    let temp = tempfile::tempdir().unwrap();
    let workspace = if overlay {
        temp.path()
            .join("target/mutation-workers/run-proof/worker-0")
    } else {
        temp.path().join("workspace")
    };
    fs::create_dir_all(workspace.join("crates/example/src")).unwrap();
    fs::create_dir_all(workspace.join("crates/example/tests")).unwrap();
    fs::write(workspace.join("crates/example/src/lib.rs"), "").unwrap();
    fs::write(workspace.join("crates/example/Cargo.toml"), "").unwrap();
    fs::write(workspace.join("crates/example/tests/terminal.rs"), "").unwrap();
    let baseline = temp.path().join("target/mutation-tests");
    fs::create_dir_all(&baseline).unwrap();
    fs::write(baseline.join("cache-proof"), "unchanged baseline").unwrap();
    let journal = temp.path().join("calls");
    let cargo = temp.path().join("cargo");
    fs::write(
        &cargo,
        r#"#!/bin/sh
if [ "$MUTATION_RUNNER_OVERLAY" = yes ]; then
    test -f target/mutation-tests/cache-proof || exit 99
    printf 'private worker' > target/mutation-tests/cache-proof
fi
case " $* " in
    *" --lib "*) stage=unit ;;
    *" --test "*) stage=integration ;;
    *) stage=full ;;
esac
printf '%s %s\n' "$stage" "$*" >> "$MUTATION_RUNNER_JOURNAL"
if [ "$stage" = "$MUTATION_RUNNER_FAILURE" ]; then exit 1; fi
"#,
    )
    .unwrap();
    fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).unwrap();
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tools/test-mutation.py");
    let output = Command::new(
        std::env::var_os("CLIMON2_QUALITY_PYTHON").unwrap_or_else(|| "python3".into()),
    )
    .arg(script)
    .current_dir(workspace)
    .env("CLIMON2_MUTATION_CARGO", cargo)
    .env("MUTATION_RUNNER_JOURNAL", &journal)
    .env("MUTATION_RUNNER_FAILURE", failure)
    .env(
        "MUTATION_RUNNER_OVERLAY",
        if overlay { "yes" } else { "no" },
    )
    .output()
    .unwrap();
    assert_eq!(
        fs::read_to_string(baseline.join("cache-proof")).unwrap(),
        "unchanged baseline",
        "a worker modified the baseline cache"
    );
    assert!(
        journal.exists(),
        "runner did not invoke Cargo: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(journal)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    (output, calls)
}

#[test]
fn killed_mutants_stop_without_running_unrelated_tests() {
    for (failure, stages) in [
        ("unit", vec!["unit"]),
        ("integration", vec!["unit", "integration"]),
    ] {
        let (output, calls) = exercise(failure, true);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            calls
                .iter()
                .map(|line| line.split_once(' ').unwrap().0)
                .collect::<Vec<_>>(),
            stages
        );
    }
}

#[test]
fn surviving_fast_tests_always_reach_the_full_workspace() {
    for failure in ["", "full"] {
        let (output, calls) = exercise(failure, true);
        assert_eq!(output.status.success(), failure.is_empty());
        assert_eq!(calls.len(), 3, "{calls:?}");
        assert!(calls[0].starts_with("unit "));
        assert!(calls[1].starts_with("integration "));
        assert!(calls[1].ends_with("--test terminal"));
        assert_eq!(
            calls[2],
            "full test --workspace --locked --target-dir target/mutation-tests"
        );
    }
}

#[test]
fn the_unmutated_baseline_still_runs_the_full_workspace() {
    let (output, calls) = exercise("full", false);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        calls,
        ["full test --workspace --locked --target-dir target/mutation-tests"]
    );
}
