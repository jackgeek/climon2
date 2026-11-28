use proc_macro2::{Delimiter, TokenStream, TokenTree};
use serde::Deserialize;
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

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
    "climon2-monitor-tui",
    "climon2-embed",
    "climon2-relay",
    "climon2-vt",
    "climon2-link",
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
    /// The platform a `[target.'...']` table limits the dependency to.
    target: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

fn metadata() -> Metadata {
    metadata_in(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/..")))
}

/// What `cargo metadata` says of the workspace in `directory`.
fn metadata_in(directory: &Path) -> Metadata {
    let output = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--offline",
            "--no-deps",
            "--format-version",
            "1",
        ])
        .current_dir(directory)
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
        "climon2-client" | "climon2-discovery" | "climon2-embed" => {
            &["climon2-core", "climon2-platform"]
        }
        // ADR 0011: the relay side of monitor links is built on the link
        // code (#501), and the relay never depends on the monitor library.
        "climon2-relay" => &["climon2-core", "climon2-platform", "climon2-link"],
        "climon2-cli" => &["climon2-core", "climon2-platform", "climon2-discovery"],
        "climon2-daemon" => &["climon2-core", "climon2-platform", "climon2-vt"],
        "climon2-monitor" => &[
            "climon2-core",
            "climon2-platform",
            "climon2-vt",
            "climon2-discovery",
            "climon2-embed",
            // ADR 0011: the hub monitor's side of monitor links (#501).
            "climon2-link",
        ],
        "climon2-monitor-tui" => &["climon2-core", "climon2-platform", "climon2-monitor"],
        _ => &[],
    }
}

/// The workspace packages `name`'s tests alone may also depend on: the
/// terminal interface builds the terminal models its tests show (ADR 0011,
/// rule 4).
fn allowed_dev_dependencies(name: &str) -> &[&str] {
    match name {
        "climon2-monitor-tui" => &["climon2-vt"],
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
                || allowed_dev_dependencies(&package.name).contains(&dependency.name.as_str())
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
        ("climon2-monitor-tui", "climon2-monitor"),
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

/// ADR 0011 (#490): the monitor library is a library alone; its terminal
/// interface builds the monitor's binary.
#[test]
fn the_monitor_library_has_no_binary() {
    let metadata = metadata();
    let library = metadata
        .packages
        .iter()
        .find(|package| package.name == "climon2-monitor")
        .unwrap();
    let kinds: Vec<_> = library
        .targets
        .iter()
        .filter(|target| target.name == "climon2_monitor" || target.name == "climon2-monitor")
        .flat_map(|target| target.kind.iter().map(String::as_str))
        .collect();
    assert_eq!(kinds, ["lib"]);
}

/// ADR 0011, rule 4 (#490): of the shipped packages, only the session
/// daemon and the monitor library depend on `climon2-vt`; the terminal
/// interface sees its views through the library, and builds models only in
/// its tests.
#[test]
fn only_the_daemon_and_the_monitor_library_depend_on_the_virtual_terminal() {
    let metadata = metadata();
    let dependents: Vec<_> = metadata
        .packages
        .iter()
        .filter(|package| !is_test_only(&package.name))
        .filter(|package| {
            let normal = |dependency: &Dependency| dependency.kind.as_deref() != Some("dev");
            let vt = |dependency: &&Dependency| dependency.name == "climon2-vt";
            package.dependencies.iter().filter(vt).any(normal)
        })
        .map(|package| package.name.as_str())
        .collect();
    let dependents: BTreeSet<_> = dependents.into_iter().collect();
    assert_eq!(
        dependents,
        BTreeSet::from(["climon2-daemon", "climon2-monitor"])
    );
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
        ("climon2-monitor", "climon2-monitor-tui", None, Some(".")),
        ("climon2-monitor-tui", "climon2-vt", None, Some(".")),
        ("climon2-monitor-tui", "climon2-embed", None, Some(".")),
        ("climon2-monitor-tui", "climon2-discovery", None, Some(".")),
        ("climon2-monitor-tui", "climon2-client", None, Some(".")),
        ("climon2-monitor-tui", "climon2-daemon", None, Some(".")),
        ("climon2-monitor-tui", "climon2-cli", None, Some(".")),
        ("climon2-cli", "climon2-monitor-tui", None, Some(".")),
        // ADR 0011, rule 2: the relay never depends on the monitor library,
        // and, rule 4, only the daemon and the monitor library on climon2-vt.
        ("climon2-relay", "climon2-monitor", None, Some(".")),
        ("climon2-relay", "climon2-monitor", Some("dev"), Some(".")),
        ("climon2-relay", "climon2-monitor-tui", None, Some(".")),
        ("climon2-relay", "climon2-vt", None, Some(".")),
        ("climon2-relay", "climon2-cli", None, Some(".")),
        ("climon2-relay", "climon2-client", None, Some(".")),
        ("climon2-relay", "climon2-daemon", None, Some(".")),
        ("climon2-monitor", "climon2-relay", None, Some(".")),
        ("climon2-cli", "climon2-relay", None, Some(".")),
        // ADR 0011: the link code reaches the hub monitor's terminal
        // interface only through the monitor library.
        ("climon2-monitor-tui", "climon2-link", None, Some(".")),
        ("climon2-cli", "climon2-link", None, Some(".")),
    ] {
        package.name = from.to_owned();
        package.dependencies = vec![Dependency {
            name: to.to_owned(),
            kind: kind.map(str::to_owned),
            path: path.map(PathBuf::from),
            target: None,
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
        ("climon2-test-term", "climon2-monitor-tui", None),
        ("climon2-monitor-tui", "climon2-monitor", None),
        ("climon2-monitor-tui", "climon2-core", None),
        ("climon2-monitor-tui", "climon2-platform", None),
        ("climon2-monitor-tui", "climon2-vt", Some("dev")),
        ("climon2-cli", "climon2-cli", Some("dev")),
        ("climon2-monitor", "climon2-embed", None),
        ("climon2-embed", "climon2-core", None),
        ("climon2-embed", "climon2-platform", None),
        ("climon2-embed", "climon2-test-term", Some("dev")),
        ("climon2-relay", "climon2-core", None),
        ("climon2-relay", "climon2-platform", None),
        ("climon2-relay", "climon2-test-term", Some("dev")),
        ("climon2-relay", "climon2-link", None),
        ("climon2-monitor", "climon2-link", None),
    ] {
        package.name = from.to_owned();
        package.dependencies = vec![Dependency {
            name: to.to_owned(),
            kind: kind.map(str::to_owned),
            path: Some(PathBuf::from(".")),
            target: None,
        }];
        assert!(dependency_violations(&package).is_empty(), "{from} -> {to}");
    }
}

/// ADR 0011, rule 1: the packages that never depend on a terminal, HTTP or
/// WebSocket crate and never read or write the process's terminal, each
/// with the only third-party crates it may link. An allowlist, so a crate
/// nobody thought to forbid is rejected too.
const HEADLESS: &[(&str, &[&str])] = &[
    ("climon2-embed", &["tracing"]),
    (
        "climon2-monitor",
        &["fuzzy-matcher", "time", "tracing", "unicode-width"],
    ),
];

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

/// The package of the link code (ADR 0011, ONE-01).
const LINK: &str = "climon2-link";
/// ONE-01: every crate the link code may depend on, whatever the kind of
/// dependency. It never holds `climon2-platform`, an operating-system or
/// socket crate, an async runtime or a source of randomness. A ticket that
/// needs a crate adds it here, where a reviewer sees it.
const LINK_ALLOWLIST: &[&str] = &[
    // Pairing (#495): CPace on ristretto255 with SHA-512, behind `pakery`.
    "pakery-cpace",
    "pakery-crypto",
    // Pairing (#495): the traits `pakery` draws the caller's randomness
    // through. The crate holds no generator of its own.
    "rand_core",
    // Pairing (#495): the key schedule, the confirmation and the box.
    "chacha20poly1305",
    "hkdf",
    "hmac",
    "sha2",
    // The pinned-key handshake (#494): the Noise framework of the handshake
    // and the units. The link crate builds it without default features, so
    // it brings no source of randomness: the caller's randomness reaches it.
    "snow",
];

/// The modules of the standard library the link code never names (ONE-01).
const PLATFORM_MODULES: &[&str] = &["net", "os", "fs", "process", "env"];
/// The standard clocks (ONE-01, ONE-02). The link code never writes these
/// names, so no import, glob or rename of `std::time` reaches a clock.
const CLOCKS: &[&str] = &["Instant", "SystemTime", "UNIX_EPOCH"];
/// The standard library's own randomness: the keys of its hash maps come
/// from the operating system (ONE-02).
const RANDOMNESS: &[&str] = &["RandomState", "HashMap", "HashSet"];
/// What introduces a compile-time condition.
const CONDITIONS: &[&str] = &["cfg", "cfg_attr", "cfg_select"];

/// ONE-01, ONE-02: each rule of the link code that `package` breaks.
fn link_violations(package: &Package, allowlist: &[&str]) -> Vec<String> {
    let mut violations = Vec::new();
    for dependency in &package.dependencies {
        if !allowlist.contains(&dependency.name.as_str()) {
            violations.push(format!(
                "dependency outside the allowlist: {}",
                dependency.name
            ));
        }
        if let Some(target) = &dependency.target {
            violations.push(format!(
                "dependency for one platform only: {}, {target}",
                dependency.name
            ));
        }
    }
    // A build script can read the target and set conditions no scan sees.
    let is_build_script = |target: &Target| target.kind.iter().any(|kind| kind == "custom-build");
    if package.targets.iter().any(is_build_script) {
        violations.push("build script".to_owned());
    }
    let root = package.manifest_path.parent().unwrap();
    for path in rust_sources(&root.join("src")) {
        let source = std::fs::read_to_string(&path).expect("read link source");
        let name = path.strip_prefix(root).unwrap().display().to_string();
        for violation in source_violations(&source) {
            violations.push(format!("{}: {violation}", name.replace('\\', "/")));
        }
    }
    violations
}

/// ADR 0011, rule 1 (#489, #490): client embedding and the monitor library
/// link no terminal, HTTP or WebSocket crate, and their sources never touch
/// the process's terminal.
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
        target: None,
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
    assert_eq!(
        interface_violations(&package),
        ["climon2-monitor -> ratatui"]
    );
    package.dependencies = vec![
        dependency("fuzzy-matcher", None, None),
        dependency("time", None, None),
        dependency("tracing", None, None),
        dependency("unicode-width", None, None),
    ];
    assert!(interface_violations(&package).is_empty());
    package.name = "climon2-monitor-tui".to_owned();
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

/// Every Rust source file under `directory`, in a stable order.
fn rust_sources(directory: &Path) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    for entry in std::fs::read_dir(directory).expect("read link source directory") {
        let path = entry.expect("link source entry").path();
        if path.is_dir() {
            sources.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            sources.push(path);
        }
    }
    sources.sort();
    sources
}

/// ONE-01, ONE-02: each rule that one source file of the link code breaks.
/// It reads tokens, not syntax, so it sees inside macros and ignores
/// comments and strings.
fn source_violations(source: &str) -> BTreeSet<String> {
    let tokens: TokenStream = source.parse().expect("link source is Rust");
    let mut violations = BTreeSet::new();
    scan(tokens, false, &mut violations);
    violations
}

/// Scans one level of tokens and the groups in it. `conditional` says the
/// tokens are the body of a compile-time condition.
fn scan(tokens: TokenStream, conditional: bool, violations: &mut BTreeSet<String>) {
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    let mut condition_follows = false;
    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Group(group) => {
                scan(group.stream(), conditional || condition_follows, violations);
                condition_follows = false;
            }
            TokenTree::Ident(ident) => {
                let name = ident.to_string();
                condition_follows = CONDITIONS.contains(&name.as_str());
                let platform = name.starts_with("target_") || name == "unix" || name == "windows";
                if conditional && platform {
                    violations.insert(format!("cfg names a platform: {name}"));
                }
                if CLOCKS.contains(&name.as_str()) {
                    violations.insert(format!("standard clock: {name}"));
                }
                if RANDOMNESS.contains(&name.as_str()) {
                    violations.insert(format!("randomness the caller did not supply: {name}"));
                }
                if name == "std" {
                    violations.extend(after_std(&trees[index + 1..]));
                }
            }
            // The `!` of `cfg!(...)` sits between the name and its body.
            TokenTree::Punct(punct) if punct.as_char() == '!' => {}
            _ => condition_follows = false,
        }
    }
}

/// What the tokens that follow the name `std` break.
fn after_std(rest: &[TokenTree]) -> Vec<String> {
    match rest {
        [TokenTree::Ident(word), ..] if word == "as" => vec!["std renamed".to_owned()],
        [TokenTree::Punct(first), TokenTree::Punct(second), rest @ ..]
            if first.as_char() == ':' && second.as_char() == ':' =>
        {
            imported_from_std(rest)
        }
        _ => Vec::new(),
    }
}

/// What a path breaks, given the tokens that follow `std::` in it.
fn imported_from_std(rest: &[TokenTree]) -> Vec<String> {
    match rest {
        [TokenTree::Ident(module), ..]
            if PLATFORM_MODULES.contains(&module.to_string().as_str()) =>
        {
            vec![format!("std::{module}")]
        }
        // `std::{self as other}` renames the standard library.
        [TokenTree::Ident(this), rest @ ..] if this == "self" => after_std(rest),
        // A glob would bring every module into scope under its bare name.
        [TokenTree::Punct(glob), ..] if glob.as_char() == '*' => vec!["std::*".to_owned()],
        [TokenTree::Group(group), ..] if group.delimiter() == Delimiter::Brace => {
            let trees: Vec<TokenTree> = group.stream().into_iter().collect();
            let is_comma = |tree: &TokenTree| matches!(tree, TokenTree::Punct(punct) if punct.as_char() == ',');
            trees.split(is_comma).flat_map(imported_from_std).collect()
        }
        _ => Vec::new(),
    }
}

/// ONE-01, ONE-02: the link code depends on nothing outside its allowlist and
/// reaches no platform facility, clock or randomness of its own.
#[test]
fn link_code_keeps_its_rules() {
    let metadata = metadata();
    let link = metadata
        .packages
        .iter()
        .find(|package| package.name == LINK)
        .expect("the link code's package");
    assert_eq!(link_violations(link, LINK_ALLOWLIST), [""; 0]);
}

/// A sample crate in a directory of its own: `manifest` follows its
/// `[package]` table, and `files` are its sources.
fn sample_crate(manifest: &str, files: &[(&str, &str)]) -> (tempfile::TempDir, Package) {
    let directory = tempfile::tempdir().expect("sample crate directory");
    let write = |name: &str, text: &str| {
        let path = directory.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).expect("sample crate folder");
        std::fs::write(path, text).expect("sample crate file");
    };
    write(
        "Cargo.toml",
        &format!(
            "[package]\nname = \"sample\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\
             [workspace]\n{manifest}"
        ),
    );
    for (name, text) in files {
        write(name, text);
    }
    let mut packages = metadata_in(directory.path()).packages;
    packages.retain(|package| package.name == "sample");
    (directory, packages.remove(0))
}

const CLEAN: &[(&str, &str)] = &[("src/lib.rs", "pub fn link() {}\n")];

/// A stand-in for `climon2-platform`, for a sample crate to depend on.
const PLATFORM: &[(&str, &str)] = &[
    (
        "platform/Cargo.toml",
        "[package]\nname = \"climon2-platform\"\nversion = \"0.0.0\"\nedition = \"2024\"\n",
    ),
    ("platform/src/lib.rs", ""),
];

/// ONE-01: a sample crate that breaks one rule is rejected for that rule and
/// for no other.
#[test]
fn link_guard_rejects_each_sample_crate_for_its_own_rule() {
    let lib = |source: &'static str| [("src/lib.rs", source)];
    for (manifest, files, allowlist, violation) in [
        (
            "[dependencies]\ntokio = \"1\"\n",
            CLEAN.to_vec(),
            &[][..],
            "dependency outside the allowlist: tokio",
        ),
        (
            "[dependencies]\nclimon2-platform = { path = \"platform\" }\n",
            [CLEAN, PLATFORM].concat(),
            &["climon2-core"][..],
            "dependency outside the allowlist: climon2-platform",
        ),
        (
            "[dev-dependencies]\ngetrandom = \"0.3\"\n",
            CLEAN.to_vec(),
            &["serde"][..],
            "dependency outside the allowlist: getrandom",
        ),
        (
            "[target.'cfg(unix)'.dependencies]\nserde = \"1\"\n",
            CLEAN.to_vec(),
            &["serde"][..],
            "dependency for one platform only: serde, cfg(unix)",
        ),
        (
            "",
            [CLEAN, &[("build.rs", "fn main() {}\n")]].concat(),
            &[][..],
            "build script",
        ),
        (
            "",
            lib("#[cfg(target_os = \"linux\")]\npub fn link() {}\n").to_vec(),
            &[][..],
            "src/lib.rs: cfg names a platform: target_os",
        ),
        (
            "",
            lib("#[cfg(unix)]\npub fn link() {}\n").to_vec(),
            &[][..],
            "src/lib.rs: cfg names a platform: unix",
        ),
        (
            "",
            lib("#[cfg(target_arch = \"x86_64\")]\npub fn link() {}\n").to_vec(),
            &[][..],
            "src/lib.rs: cfg names a platform: target_arch",
        ),
        (
            "",
            lib("pub fn link() { let _ = std::net::Ipv4Addr::LOCALHOST; }\n").to_vec(),
            &[][..],
            "src/lib.rs: std::net",
        ),
        (
            "",
            lib("pub type Link = std::os::raw::c_int;\n").to_vec(),
            &[][..],
            "src/lib.rs: std::os",
        ),
        (
            "",
            lib("pub fn link() { let _ = std::fs::read(\"key\"); }\n").to_vec(),
            &[][..],
            "src/lib.rs: std::fs",
        ),
        (
            "",
            lib("pub fn link() { std::process::abort() }\n").to_vec(),
            &[][..],
            "src/lib.rs: std::process",
        ),
        (
            "",
            lib("pub fn link() { let _ = std::env::var(\"HOME\"); }\n").to_vec(),
            &[][..],
            "src/lib.rs: std::env",
        ),
        (
            "",
            lib("pub fn link() { let _ = std::time::Instant::now(); }\n").to_vec(),
            &[][..],
            "src/lib.rs: standard clock: Instant",
        ),
        (
            "",
            lib("pub fn link() { let _ = std::time::SystemTime::now(); }\n").to_vec(),
            &[][..],
            "src/lib.rs: standard clock: SystemTime",
        ),
        (
            "",
            lib("pub type Link = std::collections::HashMap<u8, u8>;\n").to_vec(),
            &[][..],
            "src/lib.rs: randomness the caller did not supply: HashMap",
        ),
        (
            "",
            vec![
                ("src/lib.rs", "pub mod pairing;\n"),
                ("src/pairing/mod.rs", "pub mod store;\n"),
                ("src/pairing/store.rs", "pub use std::fs::File;\n"),
            ],
            &[][..],
            "src/pairing/store.rs: std::fs",
        ),
    ] {
        let (_directory, package) = sample_crate(manifest, &files);
        assert_eq!(
            link_violations(&package, allowlist),
            [violation],
            "{manifest}{files:?}"
        );
    }
}

/// ONE-01: the guard accepts what the rules leave open, so that it fails
/// only for a broken rule.
#[test]
fn link_guard_accepts_a_sample_crate_that_keeps_every_rule() {
    let source = r#"
        //! Mentions std::fs, Instant and cfg(unix) only in a comment.
        use std::{collections::BTreeMap, time::Duration};
        pub const VERSION: &str = env!("CARGO_PKG_VERSION");
        pub fn process(units: &[u8], fs: Duration) -> BTreeMap<u8, Duration> {
            let net = "std::net and SystemTime in a string";
            let _ = (net, units.windows(2).count(), cfg!(debug_assertions));
            BTreeMap::from([(0, fs)])
        }
        #[cfg(test)]
        mod tests {
            #[cfg_attr(not(feature = "slow"), ignore)]
            #[test]
            fn unix() {}
        }
    "#;
    let (_directory, package) = sample_crate(
        "[features]\nslow = []\n[dependencies]\nserde = \"1\"\n[dev-dependencies]\nserde_json = \"1\"\n",
        &[("src/lib.rs", source), ("notes.rs", "use std::fs;\n")],
    );
    assert_eq!(link_violations(&package, &["serde", "serde_json"]), [""; 0]);
}

/// ONE-01, ONE-02: no spelling of a forbidden name gets past the source scan.
#[test]
fn link_source_scan_sees_through_every_spelling() {
    for (source, violation) in [
        ("use std::{io, fs};", "std::fs"),
        ("use std::{io::{self, Read}, net::TcpStream};", "std::net"),
        ("use ::std::env;", "std::env"),
        ("use std::*;", "std::*"),
        ("use std as platform;", "std renamed"),
        ("extern crate std as platform;", "std renamed"),
        ("use std::{self as platform};", "std renamed"),
        (
            "fn f() { let _ = vec![std::process::id()]; }",
            "std::process",
        ),
        (
            "use std::time::{Duration, Instant};",
            "standard clock: Instant",
        ),
        (
            "use std::time::*; fn f() { Instant::now(); }",
            "standard clock: Instant",
        ),
        (
            "use std::time as t; const E: t::SystemTime = t::UNIX_EPOCH;",
            "standard clock: SystemTime",
        ),
        (
            "use std::time as t; const E: t::SystemTime = t::UNIX_EPOCH;",
            "standard clock: UNIX_EPOCH",
        ),
        (
            "use std::collections::HashSet;",
            "randomness the caller did not supply: HashSet",
        ),
        (
            "use std::hash::RandomState;",
            "randomness the caller did not supply: RandomState",
        ),
        (
            "fn f() -> bool { cfg!(windows) }",
            "cfg names a platform: windows",
        ),
        (
            "fn f() -> bool { cfg!(any(test, target_family = \"unix\")) }",
            "cfg names a platform: target_family",
        ),
        (
            "#[cfg_attr(target_pointer_width = \"64\", inline)] fn f() {}",
            "cfg names a platform: target_pointer_width",
        ),
        (
            "#![cfg(not(target_env = \"musl\"))]",
            "cfg names a platform: target_env",
        ),
        (
            "cfg_select! { unix => { fn f() {} } _ => {} }",
            "cfg names a platform: unix",
        ),
    ] {
        assert!(
            source_violations(source).contains(violation),
            "{source}: {:?}",
            source_violations(source)
        );
    }
}
