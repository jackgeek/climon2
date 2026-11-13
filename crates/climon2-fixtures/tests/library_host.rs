use climon2_test_term::{output_within, process_command};
use std::{path::Path, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(20);

/// #75 harness self-test: the Library-layer host reports a library it cannot
/// load as an observation, and refuses to run without a library path.
#[test]
fn library_host_reports_an_unloadable_library_and_requires_a_path() {
    let host = env!("CARGO_BIN_EXE_climon2-library-host");
    let output = output_within(&mut process_command(host), TIMEOUT);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage"));
    let missing = Path::new(env!("CARGO_MANIFEST_DIR")).join("missing-client-library");
    let output = output_within(process_command(host).arg(&missing), TIMEOUT);
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        report["error"]
            .as_str()
            .unwrap()
            .contains("missing-client-library"),
        "{report}"
    );
}
