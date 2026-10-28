use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

const GOLDEN: &str = "crates/climon2-core/tests/goldens/metadata-v1.json";

fn git(index: &Path, args: &[&str], input: Option<&[u8]>) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap())
        .env("GIT_INDEX_FILE", index)
        .env("GIT_AUTHOR_NAME", "Fixture test")
        .env("GIT_AUTHOR_EMAIL", "fixtures@example.invalid")
        .env("GIT_COMMITTER_NAME", "Fixture test")
        .env("GIT_COMMITTER_EMAIL", "fixtures@example.invalid")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(input) = input {
        child.stdin.take().unwrap().write_all(input).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn fixture_base(index: &Path, modified: bool) -> String {
    git(index, &["read-tree", "HEAD"], None);
    if modified {
        let blob = git(
            index,
            &["hash-object", "-w", "--stdin"],
            Some(b"old fixture\n"),
        );
        git(
            index,
            &[
                "update-index",
                "--cacheinfo",
                &format!("100644,{blob},{GOLDEN}"),
            ],
            None,
        );
    } else {
        git(index, &["update-index", "--force-remove", GOLDEN], None);
    }
    let tree = git(index, &["write-tree"], None);
    git(
        index,
        &["commit-tree", &tree, "-m", "append-only fixture control"],
        None,
    )
}

/// #26/#39: old goldens cannot change; new fixture additions remain allowed.
#[test]
fn metadata_fixture_gate_distinguishes_changes_from_additions() {
    let temporary = tempfile::tempdir().unwrap();
    for modified in [true, false] {
        let base = fixture_base(&temporary.path().join("index"), modified);
        let output = Command::new(env!("CARGO_BIN_EXE_xtask"))
            .args(["fixtures-check", "--base", &base])
            .output()
            .unwrap();
        assert_eq!(output.status.success(), !modified, "{output:?}");
        if modified {
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("metadata fixtures are append-only")
            );
        }
    }
}
