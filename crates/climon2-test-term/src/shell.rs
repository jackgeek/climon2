use crate::Isolation;
use portable_pty::CommandBuilder;
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

pub fn bash_profiles() -> Vec<PathBuf> {
    let candidates = if let Some(path) = env::var_os("CLIMON2_TEST_BASH") {
        vec![PathBuf::from(path)]
    } else {
        vec![
            PathBuf::from("/bin/bash"),
            ["/opt/homebrew/bin/bash", "/usr/local/bin/bash"]
                .iter()
                .map(PathBuf::from)
                .find(|path| path.is_file())
                .unwrap_or_else(|| PathBuf::from("/opt/homebrew/bin/bash")),
        ]
    };
    candidates
        .into_iter()
        .filter(|path| {
            if !path.is_file() {
                assert!(
                    env::var_os("CI").is_none(),
                    "required bash missing: {}",
                    path.display()
                );
                eprintln!("SKIP: required bash missing locally: {}", path.display());
                return false;
            }
            let output = Command::new(path)
                .arg("--version")
                .output()
                .expect("query bash version");
            let version = String::from_utf8_lossy(&output.stdout);
            assert!(
                output.status.success()
                    && (version.starts_with("GNU bash, version 3.2.")
                        || version.starts_with("GNU bash, version 5.")),
                "unsupported bash {}: {output:?}",
                path.display()
            );
            true
        })
        .collect()
}

impl Isolation {
    pub fn bash(&self, path: &Path) -> CommandBuilder {
        let mut command = self.command(path);
        command.args(["--noprofile", "--norc", "-i"]);
        command.env("PS1", "CLIMON2-PROMPT> ");
        command
    }
}
