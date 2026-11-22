//! The `pwsh` shell profile (docs/windows-qualification.md, "Shell
//! profiles"): PowerShell 7 started by absolute path with `-NoLogo
//! -NoProfile` and a fixed `prompt` function. PSReadLine stays on, because
//! users run with it.

use portable_pty::CommandBuilder;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

/// The prompt every controlled shell shows, as `controlled_bash` does on Unix.
pub const PWSH_PROMPT: &str = "CLIMON2-PROMPT> ";

/// The controlled `pwsh`: PowerShell 7 from `CLIMON2_TEST_PWSH`, else the MSI
/// build the CI runner has, `%ProgramFiles%\PowerShell\7\pwsh.exe`. The path
/// must be absolute. A missing shell fails CI and is a reported local skip;
/// any major version but 7 fails. The exact version is printed with the
/// results.
pub fn controlled_pwsh() -> Option<PathBuf> {
    let path = candidate(
        env::var_os("CLIMON2_TEST_PWSH").map(PathBuf::from),
        env::var_os("ProgramFiles").map(PathBuf::from),
    );
    assert!(
        path.is_absolute(),
        "pwsh must be named by absolute path, not {}",
        path.display()
    );
    if !path.is_file() {
        assert!(
            env::var_os("CI").is_none(),
            "required pwsh missing: {}",
            path.display()
        );
        eprintln!("SKIP: required pwsh missing locally: {}", path.display());
        return None;
    }
    let output = Command::new(&path)
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"])
        .arg("$PSVersionTable.PSVersion.ToString()")
        .output()
        .expect("query pwsh version");
    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    assert!(
        output.status.success() && is_pwsh_7(&version),
        "unsupported pwsh {} (the profile needs PowerShell 7.x): {output:?}",
        path.display()
    );
    eprintln!("pwsh {version} at {}", path.display());
    Some(path)
}

fn candidate(requested: Option<PathBuf>, program_files: Option<PathBuf>) -> PathBuf {
    requested.unwrap_or_else(|| {
        program_files
            .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
            .join(r"PowerShell\7\pwsh.exe")
    })
}

fn is_pwsh_7(version: &str) -> bool {
    version
        .strip_prefix("7.")
        .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

/// `pwsh` at `path` with the profile's start-up and prompt, interactive.
pub fn pwsh(path: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(path);
    command.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command"]);
    command.arg(format!("function global:prompt {{ '{PWSH_PROMPT}' }}"));
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_requested_pwsh_wins_over_the_msi_build() {
        let requested = PathBuf::from(r"D:\tools\pwsh.exe");
        assert_eq!(
            candidate(Some(requested.clone()), Some(r"C:\Program Files".into())),
            requested
        );
        assert_eq!(
            candidate(None, Some(r"E:\Programs".into())),
            PathBuf::from(r"E:\Programs\PowerShell\7\pwsh.exe")
        );
    }

    #[test]
    fn only_powershell_7_qualifies() {
        assert!(is_pwsh_7("7.6.0"));
        assert!(is_pwsh_7("7.10.1"));
        assert!(!is_pwsh_7("5.1.26100.1"));
        assert!(!is_pwsh_7("17.0.0"));
        assert!(!is_pwsh_7("7"));
        assert!(!is_pwsh_7(""));
    }
}
