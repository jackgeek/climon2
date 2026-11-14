//! Library layer (#97): mixed releases never cross-bind. A second,
//! independently packaged release, the test-only `climon2-second-release`
//! fixture, is installed by the real installer in a sibling version folder,
//! beside the release under test. Its client library is this release's
//! library source built as another package, so it has its own release
//! identity and is built without the `test-hooks` faults: its private
//! behaviour deliberately differs, and a call that crossed releases would
//! show it. Its session daemon is the installed daemon run from that
//! folder, which takes its release identity from the folder as every
//! release's CLI does. One `climon2-library-host` process loads both
//! releases through the shared loader and drives instances of each at the
//! same time, against real headless sessions of each release. Expected
//! values come from the native header, the native contract and the
//! fixture's hand-written version, never from another run of the library.

#[allow(dead_code)]
mod support;

use climon2_install::{Payload, install};
use climon2_test_term::{bin_path, copy_executable};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
};
use support::{
    LIBRARY, Package,
    library::{VERSION, archive, home, library},
    passive::*,
    scripted,
};

/// The second release's version: the `climon2-second-release` package's
/// version, written here by hand.
const SECOND: &str = "0.0.97";
/// `CM1_RELEASE_MISMATCH`.
const RELEASE_MISMATCH: u64 = 14;

/// Where Cargo leaves the second release's library: a dependency's cdylib
/// stays in `deps/`.
fn second_release_library() -> PathBuf {
    let name = format!(
        "{}climon2_second_release{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    bin_path("climon2")
        .parent()
        .unwrap()
        .join("deps")
        .join(name)
}

/// Packages the second release as a release archive lays it out (the
/// launcher, and the version folder of this package's executables with the
/// second release's library) and installs it with the real installer, which
/// keeps the installed release and points `current` at the new one.
fn install_second_release(package: &Package) {
    let installed = package.executable_home();
    let staged = package.isolation.home().join("second release");
    let folder = staged.join(SECOND);
    fs::create_dir_all(&folder).unwrap();
    copy_executable(&installed.join("climon2"), &staged.join("climon2"));
    for name in climon2_install::version_files() {
        let source = if name == LIBRARY {
            second_release_library()
        } else {
            installed.join(VERSION).join(&name)
        };
        copy_executable(&source, &folder.join(&name));
    }
    let payload = Payload::from_directory(&staged, SECOND).unwrap();
    install(&payload, &installed).unwrap();
}

/// A headless session of `release`, created by that release's CLI, whose
/// command waits until it is killed (two minutes at most).
struct Session {
    release: &'static str,
    id: String,
    metadata: PathBuf,
    socket: PathBuf,
}

impl Session {
    fn start(package: &Package, release: &'static str) -> Self {
        let cli = format!("{release}/climon2-cli");
        let wait = r#"until [ "$SECONDS" -ge 120 ]; do sleep 0.05; done"#;
        let output = scripted(
            package,
            &cli,
            &["exec", "--headless", "/bin/bash", "-c", wait],
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let id = String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .to_owned();
        let metadata = package
            .isolation
            .climon2_home()
            .join("sessions")
            .join(format!("{id}.pty"));
        let recorded: Value = serde_json::from_slice(&fs::read(&metadata).unwrap()).unwrap();
        assert_eq!(recorded["climon2_version"], release, "{recorded:#}");
        let socket = PathBuf::from(recorded["socket_path"].as_str().unwrap());
        Self {
            release,
            id,
            metadata,
            socket,
        }
    }

    /// Startup overrides naming this session.
    fn startup(&self, package: &Package, generation: u64) -> Value {
        let path = |path: &Path| json!({ "bytes": path.to_str().unwrap() });
        json!({
            "session_id": self.id.replace('-', ""),
            "generation": generation,
            "metadata_path": path(&self.metadata),
            "home_path": path(&package.isolation.climon2_home()),
            "runtime_path": path(self.socket.parent().unwrap()),
        })
    }

    /// `climon2-cli kill` of its own release: status 0 once it has ended.
    fn kill(&self, package: &Package) {
        let cli = format!("{}/climon2-cli", self.release);
        let output = scripted(package, &cli, &["kill", &self.id]);
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }
}

/// One `script` run of the host, which loads `primary` and then `releases`.
fn mixed_script(
    package: &Package,
    primary: &str,
    releases: &[&str],
    directory: &Path,
    environment: &[(&str, &Path)],
    steps: Value,
) -> Value {
    let root = root(package);
    let input = json!({ "root": root.to_str().unwrap(), "releases": releases, "steps": steps });
    let mut host = support::library::host(&home(package), primary, directory);
    host.arg("script").arg(input.to_string());
    for (name, value) in environment {
        host.env(name, value);
    }
    support::library::observe(host)
}

/// The steps that run instances `names` (each `(name, release, session,
/// generation)`) at the same time: all are created, each submits a passive
/// Open of its session and runs, each drains to its Initial metadata, and
/// only then are all shut down and finished.
fn simultaneous(package: &Package, instances: &[(&str, &str, &Session, u64)]) -> Value {
    let mut steps = Vec::new();
    for (name, release, session, generation) in instances {
        steps.push(json!({
            "do": "create", "instance": name, "release": release,
            "startup": session.startup(package, *generation),
        }));
    }
    for (name, ..) in instances {
        steps.push(json!({ "do": "submit", "instance": name, "request": open(1, &REQUIRED, &[]) }));
        steps.push(json!({ "do": "run", "instance": name }));
    }
    for (name, ..) in instances {
        steps
            .push(json!({ "do": "drain", "instance": name, "until": { "tag": INITIAL_METADATA } }));
    }
    for (name, ..) in instances {
        steps.push(json!({ "do": "shutdown", "instance": name }));
    }
    for (name, ..) in instances {
        steps.push(json!({ "do": "finish", "instance": name }));
    }
    Value::Array(steps)
}

/// The step of `report` that `do`es `action` for `instance`.
fn step_of<'a>(report: &'a Value, steps: &Value, action: &str, instance: &str) -> &'a Value {
    let index = steps
        .as_array()
        .unwrap()
        .iter()
        .position(|step| step["do"] == action && step["instance"] == instance)
        .unwrap();
    step(report, index)
}

/// An instance that opened a passive connection to its session: create
/// succeeded, the Open was answered OK `opened` and Initial metadata
/// followed, and shutdown departed with a confirmed Leave.
fn assert_registered(report: &Value, steps: &Value, name: &str, generation: u64) {
    assert_success(&step_of(report, steps, "create", name)["call"]);
    assert_success(step_of(report, steps, "submit", name));
    let records = step_of(report, steps, "drain", name)["records"]
        .as_array()
        .unwrap();
    let response = assert_response(&records[0], generation, 1, OK);
    assert_eq!(response["result_tag"], OPENED, "{name}: {response:#}");
    assert_header(&records[1], INITIAL_METADATA, generation);
    assert_success(step_of(report, steps, "shutdown", name));
    let (rest, terminal) = finished(step_of(report, steps, "finish", name));
    assert!(rest.is_empty(), "{name}: {rest:#?}");
    assert_left(terminal, generation);
}

/// What the host reports of each release it loaded: the image its
/// bootstrap and description are in, its release text, a second load that
/// shares the first, and every object released through its own library.
fn assert_loaded(report: &Value, release: &str, library: &Path) {
    let loaded = &report["releases"][release];
    assert_eq!(loaded["image"], library.to_str().unwrap(), "{loaded:#}");
    assert_eq!(loaded["description_image"], library.to_str().unwrap());
    assert_eq!(loaded["release"], release, "{loaded:#}");
    assert_eq!(loaded["shared"], true, "{loaded:#}");
    assert_all_released(&loaded["ledger"]);
}

/// LIB-01 Library case (several version folders; exact-release lookup
/// prevails over `current`, the working directory and search paths;
/// same-release instances share one load): with both releases installed,
/// one host process loads one release, then the other, through the shared
/// loader, from a working directory and with `LD_LIBRARY_PATH` and
/// `DYLD_FALLBACK_LIBRARY_PATH` holding the *other* release's library under
/// the library's file name, and `current` naming the other release. Two
/// instances of each release run at the same time, each against a headless
/// session of its own release: every one opens its passive connection and
/// departs with a confirmed Leave, which each library allows only for a
/// session of its own release. Each release's bootstrap and description are
/// in its own version folder's file, a second load of each shares the first,
/// and the process has loaded exactly the two files, once each. Both load
/// orders run.
#[test]
fn mixed_releases_load_by_exact_version_and_run_side_by_side_whatever_current_cwd_and_search_paths()
{
    let package = Package::private();
    install_second_release(&package);
    let home = home(&package);
    let (first, second) = (library(&home, VERSION), library(&home, SECOND));
    let (a, b) = (
        Session::start(&package, VERSION),
        Session::start(&package, SECOND),
    );
    let instances = [
        ("a1", VERSION, &a, 1),
        ("b1", SECOND, &b, 2),
        ("a2", VERSION, &a, 3),
        ("b2", SECOND, &b, 4),
    ];
    let steps = simultaneous(&package, &instances);
    for (primary, other, primary_library, other_library) in [
        (VERSION, SECOND, &first, &second),
        (SECOND, VERSION, &second, &first),
    ] {
        let decoys = package.isolation.home().join(format!("decoys of {other}"));
        let (directory, search) = (decoys.join("working directory"), decoys.join("search path"));
        for folder in [&directory, &search] {
            fs::create_dir_all(folder).unwrap();
            copy_executable(other_library, &folder.join(LIBRARY));
        }
        fs::write(home.path().join("current"), format!("{other}\n")).unwrap();
        let environment = [
            ("LD_LIBRARY_PATH", search.as_path()),
            ("DYLD_FALLBACK_LIBRARY_PATH", search.as_path()),
        ];
        let report = mixed_script(
            &package,
            primary,
            &[other],
            &directory,
            &environment,
            steps.clone(),
        );
        for (name, _, _, generation) in instances {
            assert_registered(&report, &steps, name, generation);
        }
        assert_loaded(&report, VERSION, &first);
        assert_loaded(&report, SECOND, &second);
        assert_eq!(
            report["images"],
            json!([primary_library, other_library]),
            "{report:#}"
        );
    }
    a.kill(&package);
    b.kill(&package);
    package.assert_clean();
}

/// LIB-03 Library case (simultaneous instances of both releases stay
/// independent; each release's private behaviour stays its own): in one
/// process, with instances of both releases running against their own
/// sessions, each library still answers only for its own release. A create
/// through one release's table that expects the other release fails with
/// `CM1_RELEASE_MISMATCH` and no handles; an instance of one release whose
/// startup names the other release's session refuses its Open as
/// `release_mismatch`, permanently, before any role takes effect. A call
/// that crossed releases would accept what its own release refuses.
#[test]
fn mixed_release_instances_keep_their_own_release_identity_side_by_side() {
    let package = Package::private();
    install_second_release(&package);
    let (a, b) = (
        Session::start(&package, VERSION),
        Session::start(&package, SECOND),
    );
    let mut steps = simultaneous(&package, &[("a", VERSION, &a, 1), ("b", SECOND, &b, 2)]);
    let crossed = [
        ("a-expects-second", VERSION, SECOND),
        ("b-expects-first", SECOND, VERSION),
    ];
    let foreign = [("a-of-second", VERSION, &b), ("b-of-first", SECOND, &a)];
    let mut extra = Vec::new();
    for (name, release, expected) in crossed {
        extra.push(json!({
            "do": "create", "instance": name, "release": release,
            "startup": { "expected_release": expected },
        }));
    }
    for (name, release, session) in foreign {
        extra.push(json!({
            "do": "create", "instance": name, "release": release,
            "startup": session.startup(&package, 5),
        }));
        extra.push(json!({ "do": "submit", "instance": name, "request": open(1, &REQUIRED, &[]) }));
        extra.push(json!({ "do": "run", "instance": name }));
        extra.push(json!({ "do": "finish", "instance": name }));
    }
    // While `a` and `b` are registered: after their drains, before shutdown.
    let at = steps
        .as_array()
        .unwrap()
        .iter()
        .position(|step| step["do"] == "shutdown")
        .unwrap();
    let all = steps.as_array_mut().unwrap();
    all.splice(at..at, extra);
    let report = mixed_script(
        &package,
        VERSION,
        &[SECOND],
        package.isolation.home(),
        &[],
        steps.clone(),
    );
    assert_registered(&report, &steps, "a", 1);
    assert_registered(&report, &steps, "b", 2);
    for (name, ..) in crossed {
        let create = step_of(&report, &steps, "create", name);
        assert_failure(&create["call"], RELEASE_MISMATCH, "release_mismatch");
        assert_eq!(create["output"], "zeroed", "{name}: {create:#}");
    }
    for (name, ..) in foreign {
        assert_success(&step_of(&report, &steps, "create", name)["call"]);
        let (records, terminal) = finished(step_of(&report, &steps, "finish", name));
        assert_eq!(records.len(), 2, "{name}: {records:#?}");
        assert_rejected(&records[0], 5, 1, ORIGIN_LIBRARY, "release_mismatch");
        assert_diagnostic(&records[1], 5, ORIGIN_LIBRARY, "release_mismatch");
        let refused = (CLIENT_FAILED, RETRY_PERMANENT, LEAVE_NOT_REQUIRED);
        assert_terminal(
            terminal,
            5,
            refused,
            Some((ORIGIN_LIBRARY, "release_mismatch")),
        );
    }
    let home = home(&package);
    assert_loaded(&report, VERSION, &library(&home, VERSION));
    assert_loaded(&report, SECOND, &library(&home, SECOND));
    a.kill(&package);
    b.kill(&package);
    package.assert_clean();
}

/// LIB-03 Fault case (a fault in one release reaches only that release's
/// instances; the deliberately different private behaviour shows no call
/// crossed releases): `CLIMON2_LIBRARY_FAULT=panic:receive` reaches the
/// whole process, but only the hooked release under test has the fault
/// point; the second release is built without it. With an instance of each
/// release registered at the same time, the first release's connection
/// panics on the first frame it receives, which the library contains and
/// reports as an `internal_error` failure. The second release's instance
/// opens, receives its Initial metadata and departs with a confirmed Leave,
/// unaffected. Skipped against an exact release archive, whose library has
/// no hooks.
#[test]
fn a_fault_in_one_release_never_reaches_the_other_releases_instances() {
    if archive() {
        return;
    }
    let package = Package::private();
    install_second_release(&package);
    let (a, b) = (
        Session::start(&package, VERSION),
        Session::start(&package, SECOND),
    );
    let steps = json!([
        { "do": "create", "instance": "a", "release": VERSION, "startup": a.startup(&package, 1) },
        { "do": "create", "instance": "b", "release": SECOND, "startup": b.startup(&package, 2) },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &[]) },
        { "do": "submit", "instance": "b", "request": open(1, &REQUIRED, &[]) },
        { "do": "run", "instance": "a" },
        { "do": "run", "instance": "b" },
        { "do": "drain", "instance": "b", "until": { "tag": INITIAL_METADATA } },
        { "do": "drain", "instance": "a", "until": "terminal" },
        { "do": "shutdown", "instance": "b" },
        { "do": "finish", "instance": "a" },
        { "do": "finish", "instance": "b" },
    ]);
    let root = root(&package);
    let input = json!({ "root": root.to_str().unwrap(), "releases": [SECOND], "steps": steps });
    let mut host = support::library::host(&home(&package), VERSION, package.isolation.home());
    host.arg("script")
        .arg(input.to_string())
        .env("CLIMON2_LIBRARY_FAULT", "panic:receive");
    let report = support::library::observe(host);
    let failed = step_of(&report, &steps, "drain", "a");
    let records = failed["records"].as_array().unwrap();
    let diagnostic = records
        .iter()
        .find(|record| record["tag"] == DIAGNOSTIC)
        .unwrap_or_else(|| panic!("{records:#?}"));
    assert_diagnostic(diagnostic, 1, ORIGIN_LIBRARY, "internal_error");
    let terminal = &failed["drains"].as_array().unwrap().last().unwrap()["drained"]["terminal"];
    assert_eq!(terminal["client_outcome"], CLIENT_FAILED, "{terminal:#}");
    assert_error(&terminal["error"], ORIGIN_LIBRARY, "internal_error");
    assert_registered(&report, &steps, "b", 2);
    let home = home(&package);
    assert_loaded(&report, VERSION, &library(&home, VERSION));
    assert_loaded(&report, SECOND, &library(&home, SECOND));
    a.kill(&package);
    b.kill(&package);
    package.assert_clean();
}
