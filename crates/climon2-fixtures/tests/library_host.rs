use std::{path::Path, process::Command};

/// #75 harness self-test: the Library-layer host reports a library it cannot
/// load as an observation, and refuses to run without a library path.
#[test]
fn library_host_reports_an_unloadable_library_and_requires_a_path() {
    let host = env!("CARGO_BIN_EXE_climon2-library-host");
    let output = Command::new(host).output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage"));
    let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("missing-library.dylib");
    let output = Command::new(host).arg(&missing).output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["error"]
            .as_str()
            .unwrap()
            .contains("missing-library.dylib"),
        "{report}"
    );
}
