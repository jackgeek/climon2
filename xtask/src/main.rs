use std::{
    env, io,
    path::Path,
    process::{Command, ExitCode},
};

mod quality;
mod release;

const HELP: &str = "\
Usage: cargo xtask <check|setup-quality|mutation-probe|release-check|mutate [--mutate-all]>

check runs the bootstrap gates: fmt, clippy and workspace tests.
setup-quality installs the pinned Python quality tools and cargo-llvm-cov.
mutation-probe checks real-binary worker isolation.
release-check proves per-package releases exclude hooks and preserve test binaries.
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
    } else if args == ["release-check"] {
        release::check(workspace_root())
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
