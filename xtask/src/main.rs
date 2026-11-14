use std::{
    env,
    ffi::OsString,
    io,
    path::Path,
    process::{Command, ExitCode},
};

mod coverage;
mod dist;
mod fixtures;
mod library;
mod mutation_plan;
mod quality;
mod release;

const HELP: &str = "\
Usage: cargo xtask <check [--full] [--mutate-all] [--print-mutation-plan]|test|dist|install [--dir <directory>]|fixtures-check|setup-quality|mutation-probe|tooling-probe|release-check|mutate [--native-only] [--mutate-all]>

check runs test, append-only fixtures, tooling proofs, coverage, crapper, mutation-load measurement and mutation.
check is the ticket gate: 1 load round, then mutation of only the functions, consts and statics
added or changed since the merge base of CLIMON2_MUTATION_BASE (default HEAD) and HEAD;
new files are mutated whole. One mutator run takes every file with a selected site, so the
workspace baseline runs once; its --lines is the union of the lines holding each file's
selected sites (from mutator --scan).
check --full is the full gate, run before a feature branch merges to main with
CLIMON2_MUTATION_BASE=origin/main: 5 load rounds, then mutation of whole files changed
since that merge base.
--mutate-all mutates every shipped file; --print-mutation-plan prints the selection and exits.
test runs fmt, clippy and workspace tests, including packaged acceptance, plus ShellCheck.
Before the workspace tests it runs the monitor's build test alone, against its plain binary.
setup-quality installs the pinned Python quality tools and cargo-llvm-cov.
mutation-probe checks real-binary worker isolation.
tooling-probe proves staged coverage, complexity counting and covered mutation gates.
release-check proves per-package releases exclude hooks and preserve test binaries.
dist builds verified universal macOS release artifacts (arm64 host) or x86_64 GNU/Linux artifacts in target/dist.
install builds verified production artifacts and runs their installer (optional --dir).
fixtures-check refuses any historical or uncommitted edit/deletion of an existing metadata golden.
mutate runs coverage, crapper and mutation checks on shipped source.
mutate --native-only excludes shared source; CI qualifies only host-exclusive implementations.
Each load round runs 8 workspace suites at once before mutating, even with nothing to mutate;
mutate skips it. Logs remain in target/flake-logs.
Cross-architecture execution of the same final archive is enforced in CI.";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let result = if let Some(options) = CheckOptions::parse(&args) {
        check(options)
    } else if args == ["test"] {
        test()
    } else if args == ["setup-quality"] {
        quality::setup(workspace_root())
    } else if args == ["mutation-probe"] {
        quality::probe(workspace_root())
    } else if args == ["tooling-probe"] {
        quality::tooling_probe(workspace_root())
    } else if args == ["release-check"] {
        release::check(workspace_root())
    } else if args == ["dist"] {
        dist::run(workspace_root())
    } else if args == ["install"] {
        dist::install(workspace_root(), &[])
    } else if args.len() == 3 && args[0] == "install" && args[1] == "--dir" && !args[2].is_empty() {
        dist::install(workspace_root(), &args[1..])
    } else if args == ["fixtures-check"] {
        fixtures::check(workspace_root())
    } else if args == ["mutate"] {
        quality::check(workspace_root(), false, quality::Scope::Host)
    } else if args == ["mutate", "--mutate-all"] {
        quality::check(workspace_root(), true, quality::Scope::Host)
    } else if args == ["mutate", "--native-only"] {
        quality::check(workspace_root(), false, quality::Scope::Native)
    } else if args == ["mutate", "--native-only", "--mutate-all"] {
        quality::check(workspace_root(), true, quality::Scope::Native)
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

/// `check [--full] [--mutate-all] [--print-mutation-plan]`, each flag at most once.
#[derive(Debug, Default, Eq, PartialEq)]
struct CheckOptions {
    full: bool,
    all: bool,
    print: bool,
}

impl CheckOptions {
    fn parse(args: &[OsString]) -> Option<Self> {
        let (command, flags) = args.split_first()?;
        if command != "check" {
            return None;
        }
        let mut options = Self::default();
        for flag in flags {
            let slot = match flag.to_str()? {
                "--full" => &mut options.full,
                "--mutate-all" => &mut options.all,
                "--print-mutation-plan" => &mut options.print,
                _ => return None,
            };
            if *slot {
                return None;
            }
            *slot = true;
        }
        Some(options)
    }

    fn scope(&self) -> quality::Scope {
        if self.full {
            quality::Scope::Full
        } else {
            quality::Scope::Ticket
        }
    }
}

fn check(options: CheckOptions) -> io::Result<()> {
    if options.print {
        return quality::plan(workspace_root(), options.all, options.scope());
    }
    fixtures::check(workspace_root())?;
    test()?;
    quality::probe(workspace_root())?;
    quality::tooling_probe(workspace_root())?;
    quality::check(workspace_root(), options.all, options.scope())
}

fn test() -> io::Result<()> {
    let root = workspace_root();
    quality::run(Command::new("shellcheck").arg(root.join("install.sh")))?;
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
        // Alone, the monitor package builds its binary without test hooks; the
        // workspace tests that follow rebuild the hooked sibling in its place.
        &[
            "test",
            "-p",
            "climon2-monitor",
            "--locked",
            "--test",
            "build",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Option<CheckOptions> {
        CheckOptions::parse(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn check_takes_each_tier_flag_once_in_any_order() {
        assert_eq!(parse(&["check"]), Some(CheckOptions::default()));
        assert_eq!(
            parse(&["check", "--mutate-all", "--full"]),
            Some(CheckOptions {
                full: true,
                all: true,
                print: false,
            })
        );
        assert_eq!(
            parse(&["check", "--print-mutation-plan"]),
            Some(CheckOptions {
                full: false,
                all: false,
                print: true,
            })
        );
        assert_eq!(parse(&["check", "--full", "--full"]), None);
        assert_eq!(parse(&["check", "--fast"]), None);
        assert_eq!(parse(&["mutate", "--full"]), None);
    }
}
