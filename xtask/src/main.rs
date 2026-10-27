use std::{
    env, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

const HELP: &str = "\
Usage: cargo xtask check

check runs the bootstrap gates: fmt, clippy and workspace tests.
PTY/packaged tests, coverage, crapper and mutator are not implemented yet (#31-#39).
This command is not yet a full release gate.";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    if args != ["check"] {
        eprintln!("xtask: error: unknown arguments\n\n{HELP}");
        return ExitCode::from(2);
    }
    match check() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn check() -> io::Result<()> {
    eprintln!(
        "Bootstrap checks only; PTY/packaged tests and coverage/CRAP/mutation gates \
         remain in #31-#39."
    );
    check_bash_profiles()?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace");
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
