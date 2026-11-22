//! #316: how `list` shows a creation time and how wide its table may be, on
//! Windows.
#![cfg(windows)]
use climon2_platform::presentation;
use std::process::Command;

/// A winter and a summer instant, so a daylight-saving zone shows both
/// offsets.
const INSTANTS: [&str; 2] = ["2026-01-02T20:30:00.000Z", "2026-07-02T05:04:03.000Z"];

/// The local times Windows itself gives for the instants, as .NET reads them.
fn local_by_dotnet() -> Vec<String> {
    let output = Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg(format!(
            "'{}', '{}' | ForEach-Object {{ [DateTimeOffset]::Parse($_).ToLocalTime().ToString('yyyy-MM-dd HH:mm:ss') }}",
            INSTANTS[0], INSTANTS[1]
        ))
        .env_remove("PSModulePath")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn a_creation_time_is_shown_in_the_systems_local_time() {
    let shown: Vec<_> = INSTANTS
        .iter()
        .map(|instant| presentation::local_datetime(&instant.parse().unwrap()).unwrap())
        .collect();
    assert_eq!(shown, local_by_dotnet());
}

/// Under `cargo test` stdout is a pipe, not a console.
#[test]
fn redirected_output_is_80_columns_wide() {
    assert_eq!(presentation::columns(), 80);
}
