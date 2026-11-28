use serde::Deserialize;
use std::{collections::BTreeSet, path::PathBuf, process::Command};

const PACKAGES: &[&str] = &[
    "climon2-launcher",
    "climon2-core",
    "climon2-platform",
    "climon2-cli",
    "climon2-discovery",
    "climon2-daemon",
    "climon2-client",
    "climon2-install",
    "climon2-monitor",
    "climon2-embed",
    "climon2-relay",
    "climon2-vt",
    "climon2-test-term",
    "climon2-fixtures",
    "climon2-e2e",
    "climon2-second-release",
    "climon2-measure",
    "xtask",
];
/// The second-release fixture is built as its own version, the release
/// identity it describes (owner decision on #97, 2026-11-15; ADR 0004).
const SECOND_RELEASE: (&str, &str) = ("climon2-second-release", "0.0.97");

#[derive(Clone, Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Clone, Deserialize)]
struct Package {
    name: String,
    version: String,
    publish: Option<Vec<String>>,
    dependencies: Vec<Dependency>,
    targets: Vec<Target>,
    manifest_path: PathBuf,
}

#[derive(Clone, Deserialize)]
struct Dependency {
    name: String,
    kind: Option<String>,
    path: Option<PathBuf>,
}

#[derive(Clone, Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

fn metadata() -> Metadata {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/.."))
        .output()
        .expect("run cargo metadata (no builds)");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("Cargo metadata JSON")
}

fn is_test_only(name: &str) -> bool {
    matches!(
        name,
        "climon2-test-term"
            | "climon2-fixtures"
            | "climon2-e2e"
            | "climon2-second-release"
            | "climon2-measure"
            | "xtask"
    )
}

fn allowed_internal_dependencies(name: &str) -> &[&str] {
    match name {
        "climon2-platform" | "climon2-install" => &["climon2-core"],
        // ADR 0011: the relay never depends on the monitor library.
        "climon2-client" | "climon2-discovery" | "climon2-embed" | "climon2-relay" => {
            &["climon2-core", "climon2-platform"]
        }
        "climon2-cli" => &["climon2-core", "climon2-platform", "climon2-discovery"],
        "climon2-daemon" => &["climon2-core", "climon2-platform", "climon2-vt"],
        "climon2-monitor" => &[
            "climon2-core",
            "climon2-platform",
            "climon2-vt",
            "climon2-discovery",
            "climon2-embed",
        ],
        _ => &[],
    }
}

fn dependency_violations(package: &Package) -> Vec<String> {
    let mut violations = Vec::new();
    for dependency in &package.dependencies {
        let allowed = if package.name == "climon2-launcher" {
            false
        } else if is_test_only(&package.name) || dependency.path.is_none() {
            true
        } else if dependency.kind.as_deref() == Some("dev") {
            // A package's tests may build its own test hooks (#322).
            is_test_only(&dependency.name)
                || dependency.name == package.name
                || allowed_internal_dependencies(&package.name).contains(&dependency.name.as_str())
        } else {
            allowed_internal_dependencies(&package.name).contains(&dependency.name.as_str())
        };
        if !allowed {
            violations.push(format!("{} -> {}", package.name, dependency.name));
        }
    }
    violations
}

/// ADR 0004 / #23: the workspace has precisely the agreed package boundaries.
#[test]
fn workspace_has_the_agreed_packages() {
    let actual: BTreeSet<_> = metadata().packages.into_iter().map(|p| p.name).collect();
    let expected: BTreeSet<_> = PACKAGES.iter().map(|name| (*name).to_owned()).collect();
    assert_eq!(actual, expected);
}

/// ADRs 0002/0004: launcher is dependency-free; shipped dependency edges are restricted.
#[test]
fn shipped_packages_follow_dependency_direction() {
    let violations: Vec<_> = metadata()
        .packages
        .iter()
        .flat_map(dependency_violations)
        .collect();
    assert!(violations.is_empty(), "{violations:#?}");
}

/// ADR 0004 / #23: every binary package has an integration target so Cargo builds it.
#[test]
fn every_binary_package_has_an_integration_test() {
    for package in metadata().packages {
        let has_kind = |kind: &str| {
            package
                .targets
                .iter()
                .any(|target| target.kind.iter().any(|value| value == kind))
        };
        assert!(
            !has_kind("bin") || has_kind("test"),
            "{} needs a binary integration test",
            package.name
        );
    }
}

/// #23/#24: one workspace version, but for the second-release fixture's own (#97);
/// none of these packages is published to crates.io.
#[test]
fn packages_share_one_version_and_are_not_publishable() {
    for package in metadata().packages {
        let version = if package.name == SECOND_RELEASE.0 {
            SECOND_RELEASE.1
        } else {
            env!("CARGO_PKG_VERSION")
        };
        assert_eq!(package.version, version, "{}", package.name);
        assert_eq!(package.publish, Some(vec![]), "{}", package.name);
    }
}

/// ADR 0004: Cargo names match the frozen install layout; product packages own libraries.
#[test]
fn shipped_targets_have_the_agreed_names_and_libraries() {
    let metadata = metadata();
    for (name, binary) in [
        ("climon2-launcher", "c2"),
        ("climon2-cli", "climon2-cli"),
        ("climon2-daemon", "climon2-daemon"),
        ("climon2-client", "climon2-client"),
        ("climon2-install", "climon2-install"),
        ("climon2-monitor", "climon2-monitor"),
        ("climon2-relay", "climon2-relay"),
    ] {
        let package = metadata.packages.iter().find(|p| p.name == name).unwrap();
        let binaries: Vec<_> = package
            .targets
            .iter()
            .filter(|target| target.kind.iter().any(|kind| kind == "bin"))
            .map(|target| target.name.as_str())
            .collect();
        assert_eq!(binaries, [binary], "{name}");
        if name != "climon2-launcher" {
            assert!(
                package
                    .targets
                    .iter()
                    .any(|target| target.kind.iter().any(|kind| kind == "lib")),
                "{name} needs a library and thin main"
            );
        }
    }
}

fn assert_neutral_sources(path: &std::path::Path) {
    for entry in std::fs::read_dir(path).expect("read core source directory") {
        let path = entry.expect("core source entry").path();
        if path.is_dir() {
            assert_neutral_sources(&path);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).expect("read core source");
            assert!(
                !source.contains("target_os"),
                "neutral libraries must not select platform bodies: {}",
                path.display()
            );
        }
    }
}

/// ADRs 0003/0004/0007: core and the virtual terminal stay platform-neutral.
#[test]
fn neutral_libraries_have_no_target_os_conditions() {
    let metadata = metadata();
    for name in ["climon2-core", "climon2-vt"] {
        let package = metadata
            .packages
            .iter()
            .find(|package| package.name == name)
            .unwrap();
        assert_neutral_sources(&package.manifest_path.parent().unwrap().join("src"));
    }
}

#[test]
fn dependency_guard_rejects_forbidden_edges() {
    let mut package = metadata().packages.remove(0);
    for (from, to, kind, path) in [
        ("climon2-launcher", "serde", None, None),
        (
            "climon2-launcher",
            "climon2-test-term",
            Some("dev"),
            Some("."),
        ),
        ("climon2-core", "climon2-platform", None, Some(".")),
        ("climon2-cli", "climon2-client", None, Some(".")),
        ("climon2-daemon", "climon2-fixtures", None, Some(".")),
        ("climon2-install", "climon2-platform", None, Some(".")),
        ("climon2-vt", "climon2-core", None, Some(".")),
        ("climon2-client", "climon2-vt", None, Some(".")),
        ("climon2-cli", "climon2-vt", None, Some(".")),
        ("climon2-discovery", "climon2-cli", None, Some(".")),
        ("climon2-discovery", "climon2-client", None, Some(".")),
        ("climon2-discovery", "climon2-daemon", None, Some(".")),
        ("climon2-discovery", "climon2-vt", None, Some(".")),
        ("climon2-discovery", "climon2-monitor", None, Some(".")),
        ("climon2-client", "climon2-discovery", None, Some(".")),
        ("climon2-daemon", "climon2-discovery", None, Some(".")),
        ("climon2-monitor", "climon2-client", None, Some(".")),
        ("climon2-monitor", "climon2-daemon", None, Some(".")),
        ("climon2-monitor", "climon2-cli", None, Some(".")),
        ("climon2-cli", "climon2-monitor", None, Some(".")),
        ("climon2-cli", "climon2-cli", None, Some(".")),
        ("climon2-embed", "climon2-monitor", None, Some(".")),
        ("climon2-embed", "climon2-vt", None, Some(".")),
        ("climon2-embed", "climon2-discovery", None, Some(".")),
        ("climon2-embed", "climon2-client", None, Some(".")),
        ("climon2-cli", "climon2-embed", None, Some(".")),
        ("climon2-daemon", "climon2-embed", None, Some(".")),
        // ADR 0011, rule 2: the relay never depends on the monitor library,
        // and, rule 4, only the daemon and the monitor library on climon2-vt.
        ("climon2-relay", "climon2-monitor", None, Some(".")),
        ("climon2-relay", "climon2-monitor", Some("dev"), Some(".")),
        ("climon2-relay", "climon2-vt", None, Some(".")),
        ("climon2-relay", "climon2-cli", None, Some(".")),
        ("climon2-relay", "climon2-client", None, Some(".")),
        ("climon2-relay", "climon2-daemon", None, Some(".")),
        ("climon2-monitor", "climon2-relay", None, Some(".")),
        ("climon2-cli", "climon2-relay", None, Some(".")),
    ] {
        package.name = from.to_owned();
        package.dependencies = vec![Dependency {
            name: to.to_owned(),
            kind: kind.map(str::to_owned),
            path: path.map(PathBuf::from),
        }];
        assert_eq!(dependency_violations(&package).len(), 1, "{from} -> {to}");
    }
}

#[test]
fn dependency_guard_accepts_shared_and_test_only_edges() {
    let mut package = metadata().packages.remove(0);
    for (from, to, kind) in [
        ("climon2-cli", "climon2-core", None),
        ("climon2-client", "climon2-platform", None),
        ("climon2-daemon", "climon2-test-term", Some("dev")),
        ("climon2-daemon", "climon2-vt", None),
        ("climon2-test-term", "climon2-daemon", None),
        ("climon2-cli", "climon2-discovery", None),
        ("climon2-discovery", "climon2-core", None),
        ("climon2-discovery", "climon2-platform", None),
        ("climon2-discovery", "climon2-test-term", Some("dev")),
        ("climon2-monitor", "climon2-platform", None),
        ("climon2-monitor", "climon2-vt", None),
        ("climon2-monitor", "climon2-discovery", None),
        ("climon2-test-term", "climon2-monitor", None),
        ("climon2-cli", "climon2-cli", Some("dev")),
        ("climon2-monitor", "climon2-embed", None),
        ("climon2-embed", "climon2-core", None),
        ("climon2-embed", "climon2-platform", None),
        ("climon2-embed", "climon2-test-term", Some("dev")),
        ("climon2-relay", "climon2-core", None),
        ("climon2-relay", "climon2-platform", None),
        ("climon2-relay", "climon2-test-term", Some("dev")),
    ] {
        package.name = from.to_owned();
        package.dependencies = vec![Dependency {
            name: to.to_owned(),
            kind: kind.map(str::to_owned),
            path: Some(PathBuf::from(".")),
        }];
        assert!(dependency_violations(&package).is_empty(), "{from} -> {to}");
    }
}

/// ADR 0011, rule 1: the packages that never depend on a terminal, HTTP or
/// WebSocket crate and never read or write the process's terminal, each
/// with the only third-party crates it may link. An allowlist, so a crate
/// nobody thought to forbid is rejected too. The monitor library joins
/// when it loses its terminal interface (#490).
const HEADLESS: &[(&str, &[&str])] = &[("climon2-embed", &["tracing"])];

/// The third-party crates `package` links that ADR 0011 does not allow it:
/// none for a package that is not headless. Dev-dependencies link only
/// into its tests.
fn interface_violations(package: &Package) -> Vec<String> {
    let Some((_, allowed)) = HEADLESS.iter().find(|(name, _)| *name == package.name) else {
        return Vec::new();
    };
    package
        .dependencies
        .iter()
        .filter(|dependency| dependency.path.is_none() && dependency.kind.is_none())
        .filter(|dependency| !allowed.contains(&dependency.name.as_str()))
        .map(|dependency| format!("{} -> {}", package.name, dependency.name))
        .collect()
}

/// What in `source` reads or writes the process's terminal: the standard
/// streams, the printing macros, the controlling terminal's device and the
/// platform crate's terminal seam.
fn terminal_uses(source: &str) -> Vec<&'static str> {
    // `print!(` and `println!(` also match `eprint!(` and `eprintln!(`.
    const USES: [&str; 6] = [
        "stdin(",
        "stdout(",
        "stderr(",
        "print!(",
        "println!(",
        "/dev/tty",
    ];
    let seam = uses_terminal_seam(source).then_some("climon2_platform::terminal");
    let used = USES.into_iter().filter(|used| source.contains(used));
    used.chain(seam).collect()
}

/// Whether `source` names the platform crate's `terminal` module: by its
/// path, or in a `use` of the platform crate, however it is grouped.
fn uses_terminal_seam(source: &str) -> bool {
    let imported = |statement: &str| {
        statement
            .find("use climon2_platform")
            .is_some_and(|at| identifiers(&statement[at..]).any(|name| name == "terminal"))
    };
    source.contains("climon2_platform::terminal") || source.split(';').any(imported)
}

fn identifiers(code: &str) -> impl Iterator<Item = &str> {
    code.split(|character: char| !(character.is_alphanumeric() || character == '_'))
}

fn terminal_violations(path: &std::path::Path) -> Vec<String> {
    let mut violations = Vec::new();
    for entry in std::fs::read_dir(path).expect("read a headless source directory") {
        let path = entry.expect("a headless source entry").path();
        if path.is_dir() {
            violations.extend(terminal_violations(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = std::fs::read_to_string(&path).expect("read a headless source");
            let uses = terminal_uses(&source);
            violations.extend(
                uses.iter()
                    .map(|used| format!("{}: {used}", path.display())),
            );
        }
    }
    violations
}

/// ADR 0011, rule 1 (#489): client embedding links no terminal, HTTP or
/// WebSocket crate, and its sources never touch the process's terminal.
#[test]
fn headless_packages_have_no_interface_dependency() {
    let metadata = metadata();
    for (name, _) in HEADLESS {
        let package = metadata
            .packages
            .iter()
            .find(|package| package.name == *name)
            .unwrap_or_else(|| panic!("{name} is a workspace package"));
        assert_eq!(interface_violations(package), [] as [String; 0], "{name}");
        let sources = package.manifest_path.parent().unwrap().join("src");
        assert_eq!(terminal_violations(&sources), [] as [String; 0], "{name}");
    }
}

#[test]
fn interface_guard_rejects_terminal_http_and_websocket_dependencies() {
    let mut package = metadata().packages.remove(0);
    package.name = "climon2-embed".to_owned();
    let dependency = |name: &str, kind: Option<&str>, path: Option<&str>| Dependency {
        name: name.to_owned(),
        kind: kind.map(str::to_owned),
        path: path.map(PathBuf::from),
    };
    for forbidden in [
        "ratatui",
        "ratatui-core",
        "crossterm",
        "termion",
        "http",
        "hyper",
        "axum",
        "tiny_http",
        "tungstenite",
        "tokio-tungstenite",
    ] {
        package.dependencies = vec![dependency(forbidden, None, None)];
        assert_eq!(
            interface_violations(&package),
            [format!("climon2-embed -> {forbidden}")],
        );
    }
    package.dependencies = vec![
        dependency("tracing", None, None),
        dependency("climon2-core", None, Some(".")),
        dependency("tempfile", Some("dev"), None),
        dependency("ratatui", Some("dev"), None),
    ];
    assert!(interface_violations(&package).is_empty());
    package.name = "climon2-monitor".to_owned();
    package.dependencies = vec![dependency("ratatui", None, None)];
    assert!(interface_violations(&package).is_empty());
}

#[test]
fn interface_guard_rejects_reading_or_writing_the_process_terminal() {
    for (source, used) in [
        ("let input = std::io::stdin();", "stdin("),
        ("std::io::stdout().flush()", "stdout("),
        ("io::stderr().write_all(b\"x\")", "stderr("),
        ("print!(\"x\")", "print!("),
        ("println!(\"x\")", "println!("),
        ("eprint!(\"x\")", "print!("),
        ("eprintln!(\"x\")", "println!("),
        ("File::open(\"/dev/tty\")", "/dev/tty"),
        (
            "let size = climon2_platform::terminal::size();",
            "climon2_platform::terminal",
        ),
        (
            "use climon2_platform::{\n    library::Functions,\n    terminal,\n};",
            "climon2_platform::terminal",
        ),
    ] {
        assert_eq!(terminal_uses(source), [used], "{source}");
    }
    for allowed in [
        "tracing::warn!(%error, \"client stopped\");",
        "use climon2_platform::{library::Functions, native_path};\nlet terminal = 1;",
        "use climon2_platform::library::Functions;\nuse crate::terminal;",
        "let terminal_evidence = drained.terminal;",
    ] {
        assert!(terminal_uses(allowed).is_empty(), "{allowed}");
    }
}
