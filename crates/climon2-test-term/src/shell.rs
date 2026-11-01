use crate::Isolation;
use portable_pty::CommandBuilder;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

const PROBED: [&str; 2] = ["/opt/homebrew/bin/bash", "/usr/local/bin/bash"];

/// The controlled interactive shell: GNU bash 5.x from `CLIMON2_TEST_BASH`,
/// else Homebrew's, else `/usr/local`'s. macOS `/bin/bash` (3.2) is never an
/// interactive profile. A missing shell fails CI and is a reported local skip.
pub fn controlled_bash() -> Option<PathBuf> {
    let path = candidate(
        env::var_os("CLIMON2_TEST_BASH").map(PathBuf::from),
        |path| path.is_file(),
    );
    if !path.is_file() {
        assert!(
            env::var_os("CI").is_none(),
            "required bash missing: {}",
            path.display()
        );
        eprintln!("SKIP: required bash missing locally: {}", path.display());
        return None;
    }
    let output = Command::new(&path)
        .arg("--version")
        .output()
        .expect("query bash version");
    assert!(
        output.status.success() && is_bash_5(&output.stdout),
        "unsupported bash {} (interactive tests need GNU bash 5.x): {output:?}",
        path.display()
    );
    Some(path)
}

fn candidate(requested: Option<PathBuf>, exists: impl Fn(&Path) -> bool) -> PathBuf {
    requested.unwrap_or_else(|| {
        PROBED
            .iter()
            .map(PathBuf::from)
            .find(|path| exists(path))
            .unwrap_or_else(|| PathBuf::from(PROBED[0]))
    })
}

fn is_bash_5(version: &[u8]) -> bool {
    version.starts_with(b"GNU bash, version 5.")
}

impl Isolation {
    pub fn bash(&self, path: &Path) -> CommandBuilder {
        let mut command = self.command(path);
        command.args(["--noprofile", "--norc", "-i"]);
        command.env("PS1", "CLIMON2-PROMPT> ");
        command
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOMEBREW: &str = "/opt/homebrew/bin/bash";
    const USR_LOCAL: &str = "/usr/local/bin/bash";

    fn probe(existing: &[&str]) -> PathBuf {
        candidate(None, |path| {
            existing.iter().any(|known| path == Path::new(known))
        })
    }

    #[test]
    fn requested_bash_wins_over_probed_locations() {
        let requested = PathBuf::from("/requested/bash");
        assert_eq!(candidate(Some(requested.clone()), |_| true), requested);
    }

    #[test]
    fn homebrew_is_probed_before_usr_local_and_never_bin_bash() {
        assert_eq!(
            probe(&["/bin/bash", HOMEBREW, USR_LOCAL]),
            PathBuf::from(HOMEBREW)
        );
        assert_eq!(probe(&["/bin/bash", USR_LOCAL]), PathBuf::from(USR_LOCAL));
    }

    #[test]
    fn missing_bash_5_is_reported_at_homebrew_not_bin_bash() {
        assert_eq!(probe(&["/bin/bash"]), PathBuf::from(HOMEBREW));
        assert_eq!(probe(&[]), PathBuf::from(HOMEBREW));
    }

    #[test]
    fn only_gnu_bash_5_qualifies() {
        assert!(is_bash_5(
            b"GNU bash, version 5.3.3(1)-release (aarch64-apple-darwin24)"
        ));
        assert!(!is_bash_5(
            b"GNU bash, version 3.2.57(1)-release (x86_64-apple-darwin25)"
        ));
        assert!(!is_bash_5(b"GNU bash, version 4.4.23(1)-release"));
        assert!(!is_bash_5(b"zsh 5.9 (x86_64-apple-darwin25.0)"));
    }
}
