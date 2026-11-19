use climon2_test_term::{output_within, process_command};
use std::{path::Path, time::Duration};

const TIMEOUT: Duration = Duration::from_secs(20);

/// #75/#84 harness self-test: the Library-layer host reports the shared
/// loader's refusal of a missing library as an observation, naming the
/// library it was asked for, and refuses to run without an absolute
/// executable home and a version.
#[test]
fn library_host_reports_the_loaders_refusal_and_requires_a_home_and_version() {
    let host = env!("CARGO_BIN_EXE_climon2-library-host");
    for arguments in [&[][..], &["0.1.0"], &["relative", "0.1.0"], &["/", "1"]] {
        let output = output_within(process_command(host).args(arguments), TIMEOUT);
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("usage"),
            "{arguments:?}"
        );
    }
    let home = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = output_within(process_command(host).arg(home).arg("0.0.0"), TIMEOUT);
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"]["failure"], "MissingLibrary", "{report}");
    let library = home
        .join("0.0.0")
        .join(climon2_core::layout::client_library_name());
    assert!(
        report["error"]["message"]
            .as_str()
            .unwrap()
            .contains(library.to_str().unwrap()),
        "{report}"
    );
    assert_eq!(report["images"], serde_json::json!([]));
}
