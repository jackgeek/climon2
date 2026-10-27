use std::{
    env, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

mod quality;

const HELP: &str = "\
Usage: cargo xtask <check|setup-quality|mutation-probe|mutate [--mutate-all]>

check runs the bootstrap gates: fmt, clippy and workspace tests.
setup-quality installs the pinned Python quality tools and cargo-llvm-cov.
mutation-probe checks real-binary worker isolation.
mutate runs coverage, crapper and mutation checks on shipped source.
PTY/packaged acceptance and installer-built coverage remain in #31-#39.
This command is not yet a full release gate.";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let result = if args == ["check"] {
        check()
    } else if args == ["setup-quality"] {
        quality::setup(workspace_root())
    } else if args == ["mutation-probe"] {
        quality::probe(workspace_root())
    } else if args == ["mutate"] {
        quality::check(workspace_root(), false)
    } else if args == ["mutate", "--mutate-all"] {
        quality::check(workspace_root(), true)
    } else {
        eprintln!("xtask: error: unknown arguments\n\n{HELP}");
        return ExitCode::from(2);
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace")
}

fn check() -> io::Result<()> {
    eprintln!(
        "Bootstrap checks only; PTY/packaged acceptance remains in #31-#39. \
         Run cargo xtask mutate for coverage/CRAP/mutation checks."
    );
    check_bash_profiles()?;
    let root = workspace_root();
    for args in [
        &["fmt", "--all", "--", "--check"][..],
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        &["test", "--workspace", "--locked"],
    ] {
        eprintln!("> cargo {}", args.join(" "));
        let status = Command::new(env!("CARGO"))
            .args(args)
            .current_dir(root)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "cargo {} failed with {status}",
                args.join(" ")
            )));
        }
    }
    Ok(())
}

fn check_bash_profiles() -> io::Result<()> {
    if let Some(path) = env::var_os("CLIMON2_TEST_BASH") {
        return check_bash(Path::new(&path));
    }
    check_bash(Path::new("/bin/bash"))?;
    let homebrew = [
        PathBuf::from("/opt/homebrew/bin/bash"),
        PathBuf::from("/usr/local/bin/bash"),
    ];
    if let Some(path) = homebrew.iter().find(|path| path.is_file()) {
        check_bash(path)
    } else if env::var_os("CI").is_some() {
        Err(io::Error::other(
            "Homebrew bash is missing; CI must provide CLIMON2_TEST_BASH",
        ))
    } else {
        eprintln!("SKIP: Homebrew bash is missing; bash 5 profile is unavailable locally");
        Ok(())
    }
}

fn check_bash(path: &Path) -> io::Result<()> {
    let output = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| io::Error::new(error.kind(), format!("{}: {error}", path.display())))?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "{} --version failed with {}",
            path.display(),
            output.status
        )));
    }
    let version = String::from_utf8_lossy(&output.stdout);
    if !version.starts_with("GNU bash, version 3.2.")
        && !version.starts_with("GNU bash, version 5.")
    {
        return Err(io::Error::other(format!(
            "{} is not supported bash 3.2 or 5.x",
            path.display()
        )));
    }
    eprintln!(
        "Bash available (PTY profile tests follow in #31): {}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_preflight_rejects_a_successful_non_bash_executable() {
        assert!(check_bash(Path::new(env!("CARGO"))).is_err());
    }

    #[test]
    fn shell_preflight_reports_a_missing_executable() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml/missing-bash");
        let error = check_bash(&path).unwrap_err();
        assert!(error.to_string().contains(&path.display().to_string()));
    }
}
