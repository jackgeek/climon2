// Off Unix only the source check, the native-only gate and dist are built
// (docs/for-humans/windows/quality.md), so the release tooling they share goes unused.
#![cfg_attr(not(unix), allow(dead_code))]
use std::{
    env,
    ffi::OsString,
    io,
    path::Path,
    process::{Command, ExitCode},
};

mod coverage;
#[cfg(unix)]
mod dist;
#[cfg(windows)]
#[path = "windows_dist.rs"]
mod dist;
mod fixtures;
#[cfg(unix)]
mod library;
mod measure;
mod mutation_plan;
// Only the Windows dist reads PE files; the reader's tests run everywhere.
#[cfg_attr(unix, allow(dead_code))]
mod pe;
mod qualify;
mod quality;
mod release;
mod windows_source;

const HELP: &str = "\
Usage: cargo xtask <check [--full] [--mutate-all] [--print-mutation-plan]|test|windows-changed <base>|dist|install [--dir <directory>]|fixtures-check|setup-quality|mutation-probe|tooling-probe|release-check|mutate [--native-only] [--mutate-all]|measure --archive <archive> --out <directory> [<flag>...]|qualify run ...|qualify verdict ...|qualify candidate ...>

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
test runs the Windows source-ownership check, then fmt, clippy and workspace tests,
including packaged acceptance, plus ShellCheck.
windows-changed prints whether the changes since the merge base of <base> and HEAD must run
the Windows Test and Quality jobs whatever CLIMON2_WINDOWS_TESTS says.
Before the workspace tests it runs the monitor's build test alone, against its plain binary.
setup-quality installs the pinned Python quality tools and cargo-llvm-cov.
mutation-probe runs the sentinel, whose one checked mutant must be killed and one unchecked
mutant survive, then checks real-binary worker isolation.
tooling-probe proves staged coverage, complexity counting and covered mutation gates.
release-check proves per-package releases exclude hooks and preserve test binaries.
dist builds verified universal macOS release artifacts (arm64 host), x86_64 GNU/Linux artifacts
or the x86_64 Windows zip in target/dist.
install builds verified production artifacts and runs their installer (optional --dir).
fixtures-check refuses any historical or uncommitted edit/deletion of an existing metadata golden.
mutate runs coverage, crapper and mutation checks on shipped source.
measure runs the PERF-01 matrix on an exact candidate archive (climon2-measure; see its README):
a missed target is a failed Measure case in the report, not a failed command.
qualify run writes one platform's evidence records (#122) for every acceptance-matrix case on this
native host, from the packaged tests against the exact candidate archive, every workspace test,
the platform's Measure reports and the manual records; a failed case is a record, not a failure.
qualify verdict judges every platform's records with the owner's recorded dispositions and fails
unless the release may be published; --tag of a release candidate publishes without the Windows
manual checklist.
qualify candidate names a final tag's latest release candidate, whose checklist records count,
and fails unless the tag differs from it only in the version stamp and qualification/.
mutate --native-only excludes shared source; CI qualifies only host-exclusive implementations.
On Windows it tests only what builds there, after the sentinel and a probe that a killed
process keeps its coverage. Windows builds no check, tooling-probe, release-check or install.
Each load round runs 8 workspace suites at once before mutating, even with nothing to mutate;
mutate skips it. Logs remain in target/flake-logs.
Cross-architecture execution of the same final archive is enforced in CI.";

fn main() -> ExitCode {
    let args: Vec<_> = env::args_os().skip(1).collect();
    if args.is_empty() || args == ["--help"] {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let result = if let Some(result) = unix_command(&args) {
        result
    } else if args == ["test"] {
        test()
    } else if args.len() == 2 && args[0] == "windows-changed" {
        windows_source::changed(workspace_root(), &args[1].to_string_lossy())
    } else if args == ["dist"] {
        dist::run(workspace_root())
    } else if args == ["setup-quality"] {
        quality::setup(workspace_root())
    } else if args == ["mutation-probe"] {
        quality::mutation_probe(workspace_root())
    } else if args.first().is_some_and(|command| command == "measure") {
        measure::run(workspace_root(), &args[1..])
    } else if args.first().is_some_and(|command| command == "qualify") {
        qualify::run(workspace_root(), &args[1..])
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

/// The commands that need the Unix installer and release tooling: everything
/// but the source check and the native-only gate, which Windows CI runs.
#[cfg(unix)]
fn unix_command(args: &[OsString]) -> Option<io::Result<()>> {
    Some(if let Some(options) = CheckOptions::parse(args) {
        check(options)
    } else if args == ["tooling-probe"] {
        quality::tooling_probe(workspace_root())
    } else if args == ["release-check"] {
        release::unix::check(workspace_root())
    } else if args == ["install"] {
        dist::install(workspace_root(), &[])
    } else if args.len() == 3 && args[0] == "install" && args[1] == "--dir" && !args[2].is_empty() {
        dist::install(workspace_root(), &args[1..])
    } else {
        return None;
    })
}

#[cfg(not(unix))]
fn unix_command(_: &[OsString]) -> Option<io::Result<()>> {
    None
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask is inside the workspace")
}

/// `check [--full] [--mutate-all] [--print-mutation-plan]`, each flag at most once.
#[cfg(unix)]
#[derive(Debug, Default, Eq, PartialEq)]
struct CheckOptions {
    full: bool,
    all: bool,
    print: bool,
}

#[cfg(unix)]
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

#[cfg(unix)]
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
    windows_source::check(root)?;
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

#[cfg(all(test, unix))]
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
