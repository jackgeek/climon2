use serde::Deserialize;
use std::{collections::BTreeSet, path::PathBuf, process::Command};

const PACKAGES: &[&str] = &[
    "climon2-launcher",
    "climon2-core",
    "climon2-platform",
    "climon2-cli",
    "climon2-daemon",
    "climon2-client",
    "climon2-install",
    "climon2-test-term",
    "climon2-fixtures",
    "climon2-e2e",
    "xtask",
];

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
        "climon2-test-term" | "climon2-fixtures" | "climon2-e2e" | "xtask"
    )
}

fn allowed_internal_dependencies(name: &str) -> &[&str] {
    match name {
        "climon2-platform" | "climon2-install" => &["climon2-core"],
        "climon2-cli" | "climon2-daemon" | "climon2-client" => {
            &["climon2-core", "climon2-platform"]
        }
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
            is_test_only(&dependency.name)
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

/// #23/#24: one workspace version; none of these packages is published to crates.io.
#[test]
fn packages_share_one_version_and_are_not_publishable() {
    for package in metadata().packages {
        assert_eq!(
            package.version,
            env!("CARGO_PKG_VERSION"),
            "{}",
            package.name
        );
        assert_eq!(package.publish, Some(vec![]), "{}", package.name);
    }
}

/// ADR 0004: Cargo names match the frozen install layout; product packages own libraries.
#[test]
fn shipped_targets_have_the_agreed_names_and_libraries() {
    let metadata = metadata();
    for (name, binary) in [
        ("climon2-launcher", "climon2"),
        ("climon2-cli", "climon2-cli"),
        ("climon2-daemon", "climon2-daemon"),
        ("climon2-client", "climon2-client"),
        ("climon2-install", "climon2-install"),
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
                "core must not select platform bodies: {}",
                path.display()
            );
        }
    }
}

/// ADRs 0003/0004: core stays platform-neutral and can be tested independently.
#[test]
fn core_has_no_target_os_conditions() {
    let metadata = metadata();
    let core = metadata
        .packages
        .iter()
        .find(|package| package.name == "climon2-core")
        .unwrap();
    assert_neutral_sources(&core.manifest_path.parent().unwrap().join("src"));
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
        ("climon2-test-term", "climon2-daemon", None),
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
