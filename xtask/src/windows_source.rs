//! Windows-only source (docs/for-humans/windows/quality.md): where it may live, and
//! which changes must run the Windows jobs whatever `CLIMON2_WINDOWS_TESTS`
//! says.
//!
//! Windows-only shipped code lives only in registered Windows files. Anywhere
//! else in shipped source a `cfg` naming Windows may appear only on a
//! `mod name;` declaration, because code it guarded would compile only on
//! Windows and be mutated nowhere. Test code, such as the harness's Windows
//! host, is not mutated on any platform and is not checked.
use crate::quality::{self, SHIPPED, native_source_os};
use proc_macro2::{TokenStream, TokenTree};
use quote::ToTokens;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};
use syn::{
    Attribute, ItemMod, Macro,
    spanned::Spanned,
    visit::{self, Visit},
};

/// Paths outside the registered Windows files whose changes run the Windows
/// jobs regardless of the switch: the harness's Windows host and its tests,
/// the installer's Windows tests, the Windows dist and download script whose
/// zip the Archive job tests, the pinned downloads, the workflows and xtask's
/// quality code, including what the Windows gate builds xtask from and its
/// probes' fixtures.
const WINDOWS_TRIGGERS: &[&str] = &[
    ".github/",
    "crates/climon2-cli/tests/exec_windows.rs",
    "crates/climon2-cli/tests/packaged_windows.rs",
    "crates/climon2-cli/tests/support/",
    "crates/climon2-install/tests/archive_windows.rs",
    "crates/climon2-install/tests/windows.rs",
    "crates/climon2-cli/tests/host_models_windows.rs",
    "crates/climon2-test-term/src/windows",
    "crates/climon2-test-term/tests/windows_host.rs",
    "install.ps1",
    "pins/",
    "tools/",
    "xtask/Cargo.toml",
    "xtask/src/coverage.rs",
    "xtask/src/main.rs",
    "xtask/src/mutation_plan.rs",
    "xtask/src/quality.rs",
    "xtask/src/windows_dist.rs",
    "xtask/src/windows_source.rs",
    "xtask/tests/fixtures/mutation-isolation/",
    "xtask/tests/fixtures/mutation-sentinel/",
];

fn is_registered_windows(path: &Path) -> bool {
    native_source_os(path) == Some("windows")
}

/// Whether a change to `path`, relative to the workspace root, must run the
/// Windows Test and Quality jobs whatever `CLIMON2_WINDOWS_TESTS` says.
pub(crate) fn triggers_windows_jobs(path: &str) -> bool {
    is_registered_windows(Path::new(path))
        || WINDOWS_TRIGGERS
            .iter()
            .any(|trigger| path.starts_with(trigger))
}

/// `windows-changed <base>`: prints `true` when the changes since the merge
/// base of `base` and `HEAD` include one that runs the Windows jobs, else
/// `false`.
pub(crate) fn changed(root: &Path, base: &str) -> io::Result<()> {
    let diff = quality::output(Command::new("git").current_dir(root).args([
        "diff",
        "--name-only",
        &format!("{base}...HEAD"),
    ]))?;
    let names = String::from_utf8(diff.stdout).map_err(io::Error::other)?;
    println!("{}", names.lines().any(triggers_windows_jobs));
    Ok(())
}

/// Fails, naming each `path:line`, when shipped source outside the registered
/// Windows files uses a `cfg` naming Windows anywhere but on a `mod name;`.
pub(crate) fn check(root: &Path) -> io::Result<()> {
    let mut misplaced = Vec::new();
    for path in checked_sources(root)? {
        let source = fs::read_to_string(root.join(&path))?;
        let lines = misplaced_windows_cfgs(&source)
            .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
        misplaced.extend(
            lines
                .into_iter()
                .map(|line| format!("{}:{line}", path.display())),
        );
    }
    if misplaced.is_empty() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "cfg naming Windows outside a registered Windows file, other than on a `mod name;` declaration: {}",
        misplaced.join(", ")
    )))
}

/// Every tracked or new shipped Rust file except the registered Windows ones.
fn checked_sources(root: &Path) -> io::Result<Vec<PathBuf>> {
    let listing = quality::output(
        Command::new("git")
            .current_dir(root)
            .args([
                "ls-files",
                "--cached",
                "--others",
                "--exclude-standard",
                "--",
            ])
            .args(SHIPPED),
    )?;
    let names = String::from_utf8(listing.stdout).map_err(io::Error::other)?;
    Ok(names
        .lines()
        .map(PathBuf::from)
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .filter(|path| !is_registered_windows(path) && root.join(path).is_file())
        .collect())
}

/// The lines of each `cfg`, `cfg_attr` or `cfg!` naming Windows other than
/// on a `mod name;` declaration.
fn misplaced_windows_cfgs(source: &str) -> syn::Result<Vec<usize>> {
    let mut finder = Finder::default();
    finder.visit_file(&syn::parse_file(source)?);
    Ok(finder.lines)
}

#[derive(Default)]
struct Finder {
    lines: Vec<usize>,
}

impl Finder {
    fn record(&mut self, is_cfg: bool, tokens: &TokenStream, line: usize) {
        if is_cfg && names_windows(tokens) {
            self.lines.push(line);
        }
    }
}

impl<'ast> Visit<'ast> for Finder {
    fn visit_item_mod(&mut self, item: &'ast ItemMod) {
        if item.content.is_some() {
            visit::visit_item_mod(self, item);
        }
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        let path = attribute.path();
        self.record(
            path.is_ident("cfg") || path.is_ident("cfg_attr"),
            &attribute.meta.to_token_stream(),
            attribute.span().start().line,
        );
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        self.record(
            mac.path.is_ident("cfg"),
            &mac.tokens,
            mac.path.span().start().line,
        );
    }
}

fn names_windows(tokens: &TokenStream) -> bool {
    tokens.clone().into_iter().any(|token| match token {
        TokenTree::Ident(ident) => ident == "windows",
        TokenTree::Literal(literal) => literal.to_string() == "\"windows\"",
        TokenTree::Group(group) => names_windows(&group.stream()),
        TokenTree::Punct(_) => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(source: &str) -> Vec<usize> {
        misplaced_windows_cfgs(source).unwrap()
    }

    #[test]
    fn windows_cfg_is_allowed_only_on_a_module_declaration() {
        assert!(lines("#[cfg(windows)]\nmod native;\n").is_empty());
        assert!(
            lines("#[cfg(windows)]\n#[path = \"home/windows.rs\"]\npub(crate) mod native;\n")
                .is_empty()
        );
        assert!(lines("#[cfg_attr(windows, path = \"w.rs\")]\nmod native;\n").is_empty());
        assert!(lines("#[cfg(unix)]\nfn f() { if cfg!(unix) {} }\n").is_empty());
        assert!(
            lines("// #[cfg(windows)] fn f() {}\nconst S: &str = \"cfg(windows)\";\n").is_empty()
        );
        assert!(lines("#[cfg(feature = \"x\")]\nfn windows() {}\n").is_empty());
    }

    #[test]
    fn windows_cfg_anywhere_else_is_reported_by_line() {
        assert_eq!(lines("\n#[cfg(windows)]\nfn f() {}\n"), [2]);
        assert_eq!(
            lines("#[cfg(target_os = \"windows\")]\nuse std::io;\n"),
            [1]
        );
        assert_eq!(lines("#[cfg(any(unix, windows))]\nfn f() {}\n"), [1]);
        assert_eq!(lines("#[cfg(not(windows))]\nfn f() {}\n"), [1]);
        assert_eq!(lines("#[cfg(windows)]\nmod inline {}\n"), [1]);
        assert_eq!(
            lines("mod inline {\n    #[cfg(windows)]\n    fn f() {}\n}\n"),
            [2]
        );
        assert_eq!(lines("#![cfg(windows)]\n"), [1]);
        assert_eq!(
            lines("#[cfg_attr(windows, allow(dead_code))]\nfn f() {}\n"),
            [1]
        );
        assert_eq!(lines("fn f() {\n    if cfg!(windows) {}\n}\n"), [2]);
        assert_eq!(
            lines("fn f() {\n    #[cfg(windows)]\n    let x = 1;\n}\n"),
            [2]
        );
    }

    #[test]
    fn the_check_names_each_misplaced_use_outside_registered_windows_files() {
        let scratch = tempfile::tempdir().unwrap();
        let root = scratch.path();
        quality::run(
            Command::new("git")
                .current_dir(root)
                .args(["init", "--quiet"]),
        )
        .unwrap();
        let write = |path: &str, source: &str| {
            fs::create_dir_all(root.join(path).parent().unwrap()).unwrap();
            fs::write(root.join(path), source).unwrap();
        };
        write(
            "crates/climon2-platform/src/home.rs",
            "#[cfg(windows)]\nmod native;\n",
        );
        write(
            "crates/climon2-platform/src/home/windows.rs",
            "#[cfg(windows)]\nfn f() {}\n",
        );
        write(
            "crates/climon2-platform/tests/home.rs",
            "#[cfg(windows)]\nfn f() {}\n",
        );
        write("xtask/src/main.rs", "#[cfg(windows)]\nfn f() {}\n");
        check(root).unwrap();
        write(
            "crates/climon2-core/src/lib.rs",
            "fn f() {}\n\n#[cfg(windows)]\nfn g() {}\n",
        );
        write(
            "crates/climon2-cli/src/x.rs",
            "#[cfg(windows)]\nuse std::io;\n",
        );
        assert_eq!(
            check(root).unwrap_err().to_string(),
            "cfg naming Windows outside a registered Windows file, other than on a `mod name;` \
             declaration: crates/climon2-cli/src/x.rs:1, crates/climon2-core/src/lib.rs:3"
        );
    }

    #[test]
    fn the_workspace_confines_windows_cfg_to_registered_files() {
        check(Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap()).unwrap();
    }

    #[test]
    fn windows_files_workflows_and_quality_code_trigger_the_windows_jobs() {
        for path in [
            "crates/climon2-platform/src/home/windows.rs",
            "crates/climon2-test-term/src/windows.rs",
            "crates/climon2-test-term/src/windows/host.rs",
            "crates/climon2-test-term/tests/windows_host.rs",
            "crates/climon2-cli/tests/exec_windows.rs",
            "crates/climon2-cli/tests/packaged_windows.rs",
            "crates/climon2-cli/tests/support/mod.rs",
            "crates/climon2-install/src/windows.rs",
            "crates/climon2-install/src/user_path/windows.rs",
            "crates/climon2-install/tests/windows.rs",
            "crates/climon2-install/tests/archive_windows.rs",
            "crates/climon2-cli/tests/host_models_windows.rs",
            "install.ps1",
            "xtask/src/windows_dist.rs",
            "pins/conpty.txt",
            ".github/workflows/check.yml",
            ".github/actions/controlled-bash/action.yml",
            "tools/test-mutation.py",
            "xtask/src/quality.rs",
            "xtask/src/coverage.rs",
            "xtask/src/mutation_plan.rs",
            "xtask/src/windows_source.rs",
            "xtask/src/main.rs",
            "xtask/Cargo.toml",
            "xtask/tests/fixtures/mutation-sentinel/src/lib.rs",
            "xtask/tests/fixtures/mutation-isolation/cli/src/main.rs",
        ] {
            assert!(triggers_windows_jobs(path), "{path}");
        }
        for path in [
            "crates/climon2-platform/src/home.rs",
            "crates/climon2-other/src/windows.rs",
            "crates/climon2-test-term/src/terminal.rs",
            "crates/climon2-test-term/tests/received.rs",
            "crates/climon2-platform/src/runtime/linux.rs",
            "crates/climon2-install/src/unix.rs",
            "crates/climon2-install/tests/install.rs",
            "install.sh",
            "xtask/src/dist.rs",
            "README.md",
            "x.github/workflows/check.yml",
        ] {
            assert!(!triggers_windows_jobs(path), "{path}");
        }
    }
}
