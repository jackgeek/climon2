// Off Unix only the source check, the native-only gate, dist and install are built
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
mod mutation_slice;
// Only the Windows dist reads PE files; the reader's tests run everywhere.
#[cfg_attr(unix, allow(dead_code))]
mod pe;
mod qualify;
mod quality;
mod release;
mod windows_source;

use mutation_slice::{Selection, Slice};

const HELP: &str = "\
Usage: cargo xtask <check [--full] [--mutate-all] [--print-mutation-plan]|test|windows-changed <base>|dist|install [--dir <directory>]|fixtures-check|setup-quality|mutation-probe|tooling-probe|release-check|mutate [--native-only] [--mutate-all|--mutate-slice <0-9>]|measure --archive <archive> --out <directory> [<flag>...]|qualify run ...|qualify verdict ...|qualify candidate ...>

check runs test, append-only fixtures, tooling proofs, coverage, crapper, mutation-load measurement and mutation.
check is the ticket gate: 1 load round, then mutation of only the functions, consts and statics
added or changed since the merge base of CLIMON2_MUTATION_BASE (default HEAD) and HEAD, and of
the functions in the same file that name a changed struct, enum, const, static or type;
a changed import selects the functions naming it; new files are mutated whole, as are files
with a changed glob import, item macro, trait signature, associated type or union, or an
unparsable base. One mutator run takes every file with a selected site, so the
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
install builds verified production artifacts and runs their installer (optional --dir) on
every dist host; it leaves PATH and shell start-up files alone and prints the PATH entry to add.
fixtures-check refuses any historical or uncommitted edit/deletion of an existing metadata golden.
mutate runs coverage, crapper and mutation checks on shipped source, mutating what check would
and also the functions a changed test names, or the whole file when it names none.
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
mutate --mutate-slice <k> mutates the whole shipped files in slice k of 10, chosen by a hash of
each file's path; main mutates the next slice on each squash-merge.
On Windows it tests only what builds there, after the sentinel and a probe that a killed
process keeps its coverage. Windows builds no check, tooling-probe or release-check.
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
    } else if args == ["install"] {
        dist::install(workspace_root(), &[])
    } else if args.len() == 3 && args[0] == "install" && args[1] == "--dir" && !args[2].is_empty() {
        dist::install(workspace_root(), &args[1..])
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
    } else if let Some((selection, scope)) = mutate_options(&args) {
        quality::check(workspace_root(), selection, scope)
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

/// The commands that need the Unix release tooling: the gate, its tooling
/// proofs and the per-package release check.
#[cfg(unix)]
fn unix_command(args: &[OsString]) -> Option<io::Result<()>> {
    Some(if let Some(options) = CheckOptions::parse(args) {
        check(options)
    } else if args == ["tooling-probe"] {
        quality::tooling_probe(workspace_root())
    } else if args == ["release-check"] {
        release::unix::check(workspace_root())
    } else {
        return None;
    })
}

#[cfg(not(unix))]
fn unix_command(_: &[OsString]) -> Option<io::Result<()>> {
    None
}

/// `mutate [--native-only] [--mutate-all|--mutate-slice <k>]`, in that order.
fn mutate_options(args: &[OsString]) -> Option<(Selection, quality::Scope)> {
    let args: Vec<_> = args.iter().map(|arg| arg.to_str()).collect::<Option<_>>()?;
    let (command, mut rest) = args.split_first()?;
    if *command != "mutate" {
        return None;
    }
    let scope = if rest.first() == Some(&"--native-only") {
        rest = &rest[1..];
        quality::Scope::Native
    } else {
        quality::Scope::Host
    };
    let selection = match rest {
        [] => Selection::Changed,
        ["--mutate-all"] => Selection::All,
        ["--mutate-slice", slice] => Selection::Slice(Slice::parse(slice)?),
        _ => return None,
    };
    Some((selection, scope))
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

    fn selection(&self) -> Selection {
        if self.all {
            Selection::All
        } else {
            Selection::Changed
        }
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
        return quality::plan(workspace_root(), options.selection(), options.scope());
    }
    fixtures::check(workspace_root())?;
    test()?;
    quality::probe(workspace_root())?;
    quality::tooling_probe(workspace_root())?;
    quality::check(workspace_root(), options.selection(), options.scope())
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

#[cfg(test)]
mod mutate_tests {
    use super::*;
    use mutation_slice::SLICES;
    use quality::Scope::{Host, Native};

    fn parse(args: &[&str]) -> Option<(Selection, quality::Scope)> {
        mutate_options(&args.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn mutate_takes_a_scope_then_one_selection() {
        assert_eq!(parse(&["mutate"]), Some((Selection::Changed, Host)));
        assert_eq!(
            parse(&["mutate", "--mutate-all"]),
            Some((Selection::All, Host))
        );
        assert_eq!(
            parse(&["mutate", "--native-only"]),
            Some((Selection::Changed, Native))
        );
        assert_eq!(
            parse(&["mutate", "--native-only", "--mutate-all"]),
            Some((Selection::All, Native))
        );
        for index in 0..SLICES {
            let text = index.to_string();
            let slice = Selection::Slice(Slice::parse(&text).unwrap());
            assert_eq!(
                parse(&["mutate", "--mutate-slice", &text]),
                Some((slice, Host))
            );
            assert_eq!(
                parse(&["mutate", "--native-only", "--mutate-slice", &text]),
                Some((slice, Native))
            );
        }
        for args in [
            &["mutate", "--mutate-slice"][..],
            &["mutate", "--mutate-slice", "10"],
            &["mutate", "--mutate-all", "--mutate-slice", "1"],
            &["mutate", "--mutate-slice", "1", "--native-only"],
            &["mutate", "--mutate-all", "--native-only"],
            &["mutate", "--native-only", "--native-only"],
            &["mutate", "--full"],
            &["check", "--mutate-all"],
        ] {
            assert_eq!(parse(args), None, "{args:?}");
        }
    }
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
