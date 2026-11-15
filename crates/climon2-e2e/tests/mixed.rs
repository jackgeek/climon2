//! Library layer (#97): mixed releases never cross-bind. The
//! [second-release fixture](../../../GLOSSARY.md#testing), release 0.0.97,
//! is packaged as a release archive lays one out and installed by the real
//! installer in a sibling version folder, beside the release under test.
//! Its client library is this build's library source compiled as the
//! test-only `climon2-second-release` package, with that package's version
//! as its release identity and without the `test-hooks` faults. Its session
//! daemon is the `climon2-daemon` library behind that package's thin main,
//! which has the release compiled in and gives its session's command
//! `CLIMON2_SECOND_RELEASE_DAEMON`, a private behaviour a session's title
//! makes visible over revision 1. Each release's CLI starts headless
//! sessions served by its own folder's daemon. One `climon2-library-host`
//! process loads both releases' libraries through the shared loader and
//! drives instances of each at the same time. Expected values come from the
//! native header, the native contract and the fixture's documented
//! behaviour, never from another run of the library.
//!
//! Releases ship no private dependencies, so these tests cannot exercise
//! release-relative private-dependency resolution (LIB-01); they cover
//! exact-version lookup, one shared load per release, and that no symbol,
//! table or daemon crosses releases.

#[allow(dead_code)]
mod support;

use climon2_fixtures::second_release::{self, VERSION as SECOND};
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

/// What every test-hooks build contains, and the second-release fixture's
/// library must not.
const HOOK_MARKER: &[u8] = b"CLIMON2_TEST_HOOKS_ENABLED";

/// Packages a release as a release archive lays it out, the launcher and a
/// version folder of the installed release's executables with `daemon` and
/// `library` instead of its own, and installs it as `version` with the real
/// installer, which keeps the installed release and points `current` at the
/// new one.
fn install_release(package: &Package, version: &str, daemon: &Path, library: &Path) {
    let installed = package.executable_home();
    let staged = package.isolation.home().join(format!("release {version}"));
    let folder = staged.join(version);
    fs::create_dir_all(&folder).unwrap();
    copy_executable(&installed.join("climon2"), &staged.join("climon2"));
    for name in climon2_install::version_files() {
        let source = match name.as_str() {
            LIBRARY => library.to_owned(),
            "climon2-daemon" => daemon.to_owned(),
            _ => installed.join(VERSION).join(&name),
        };
        copy_executable(&source, &folder.join(&name));
    }
    let payload = Payload::from_directory(&staged, version).unwrap();
    install(&payload, &installed).unwrap();
}

/// The second-release fixture's daemon and library, as Cargo built them.
fn second_release_artifacts() -> (PathBuf, PathBuf) {
    let daemon = bin_path(second_release::DAEMON);
    (
        daemon.clone(),
        second_release::library(daemon.parent().unwrap()),
    )
}

/// Installs the second-release fixture as release 0.0.97, after checking
/// that its library carries no test-hook marker: its missing fault points
/// are the private behaviour the Fault case relies on.
fn install_second_release(package: &Package) {
    let (daemon, library) = second_release_artifacts();
    let bytes = fs::read(&library).unwrap();
    assert!(
        !bytes
            .windows(HOOK_MARKER.len())
            .any(|window| window == HOOK_MARKER),
        "{} carries the test-hook marker",
        library.display()
    );
    install_release(package, SECOND, &daemon, &library);
}

/// The title a session's command sets once `go` exists: its daemon's
/// `CLIMON2_SECOND_RELEASE_DAEMON`, or `none`.
fn daemon_title(release: &str) -> String {
    let daemon = if release == SECOND { SECOND } else { "none" };
    format!("daemon:{daemon}")
}

/// A headless session of `release`, created by that release's CLI and
/// served by that release's daemon. Its command waits for `go`, then sets
/// its title from `CLIMON2_SECOND_RELEASE_DAEMON` (see [`daemon_title`]),
/// and waits until it is killed (two minutes at most).
struct Session {
    release: &'static str,
    id: String,
    metadata: PathBuf,
    socket: PathBuf,
    go: PathBuf,
}

impl Session {
    fn start(package: &Package, release: &'static str, name: &str) -> Self {
        let go = package.isolation.home().join(format!("go {name}"));
        let command = r#"until [ -e "$1" ] || [ "$SECONDS" -ge 120 ]; do sleep 0.05; done
printf '\033]2;daemon:%s\007' "${CLIMON2_SECOND_RELEASE_DAEMON:-none}"
until [ "$SECONDS" -ge 120 ]; do sleep 0.05; done"#;
        let cli = format!("{release}/climon2-cli");
        let go_text = go.to_str().unwrap();
        let args = [
            "exec",
            "--headless",
            "/bin/bash",
            "-c",
            command,
            "session",
            go_text,
        ];
        let output = scripted(package, &cli, &args);
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
            go,
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
    (primary, releases): (&str, &[&str]),
    directory: &Path,
    environment: &[(&str, &str)],
    steps: &Value,
) -> Value {
    let root = root(package);
    let input = json!({ "root": root.to_str().unwrap(), "releases": releases, "steps": steps });
    let mut host = support::library::host(&home(package), primary, directory);
    host.arg("script").arg(input.to_string());
    host.envs(environment.iter().copied());
    support::library::observe(host)
}

/// One instance: its name, release, session and startup generation.
type Instance<'a> = (&'a str, &'a str, &'a Session, u64);

/// The steps that run `instances` at the same time: all are created, each
/// submits a passive Open of its session (`title-v1` optional) and runs,
/// each drains to its Initial metadata; then every session's command sets
/// its title, each instance drains to that Title changed record, and only
/// then are all shut down and finished.
fn simultaneous(package: &Package, instances: &[Instance]) -> Value {
    let mut steps = Vec::new();
    for (name, release, session, generation) in instances {
        steps.push(json!({
            "do": "create", "instance": name, "release": release,
            "startup": session.startup(package, *generation),
        }));
    }
    for (name, ..) in instances {
        let open = open(1, &REQUIRED, &["title-v1"]);
        steps.push(json!({ "do": "submit", "instance": name, "request": open }));
        steps.push(json!({ "do": "run", "instance": name }));
    }
    for (name, ..) in instances {
        let until = json!({ "tag": INITIAL_METADATA });
        steps.push(json!({ "do": "drain", "instance": name, "until": until }));
    }
    for (_, _, session, _) in instances {
        let go = json!({ "do": "touch", "path": session.go.to_str().unwrap() });
        if !steps.contains(&go) {
            steps.push(go);
        }
    }
    for (name, ..) in instances {
        let until = json!({ "tag": TITLE });
        steps.push(json!({ "do": "drain", "instance": name, "until": until, "titled": true }));
    }
    for (name, ..) in instances {
        steps.push(json!({ "do": "shutdown", "instance": name }));
    }
    for (name, ..) in instances {
        steps.push(json!({ "do": "finish", "instance": name }));
    }
    Value::Array(steps)
}

/// The step of `report` that `do`es `action` for `instance` (with `extra`
/// fields).
fn step_where<'a>(
    report: &'a Value,
    steps: &Value,
    action: &str,
    instance: &str,
    extra: &Value,
) -> &'a Value {
    let index = steps
        .as_array()
        .unwrap()
        .iter()
        .position(|step| {
            step["do"] == action
                && step["instance"] == instance
                && extra
                    .as_object()
                    .unwrap()
                    .iter()
                    .all(|(key, value)| step[key] == *value)
        })
        .unwrap();
    step(report, index)
}

fn step_of<'a>(report: &'a Value, steps: &Value, action: &str, instance: &str) -> &'a Value {
    step_where(report, steps, action, instance, &json!({}))
}

/// An instance that opened a passive connection to its session: create
/// succeeded, the Open was answered OK `opened` with `title-v1` selected
/// and Initial metadata followed.
fn assert_opened(report: &Value, steps: &Value, name: &str, generation: u64) {
    assert_success(&step_of(report, steps, "create", name)["call"]);
    assert_success(step_of(report, steps, "submit", name));
    let records = step_of(report, steps, "drain", name)["records"]
        .as_array()
        .unwrap();
    let response = assert_response(&records[0], generation, 1, OK);
    assert_eq!(response["result_tag"], OPENED, "{name}: {response:#}");
    let selected = &response["result"]["selected"];
    assert_eq!(
        *selected,
        json!(["daemon-wire-v1", "monitor-v1", "title-v1"]),
        "{name}"
    );
    assert_header(&records[1], INITIAL_METADATA, generation);
}

/// An instance registered with a session of `release`: opened, it relays
/// the title only that release's daemon gives its command, and shutdown
/// departs with a confirmed Leave.
fn assert_registered(
    report: &Value,
    steps: &Value,
    name: &str,
    (release, generation): (&str, u64),
) {
    assert_opened(report, steps, name, generation);
    let titled = step_where(report, steps, "drain", name, &json!({ "titled": true }));
    let records = titled["records"].as_array().unwrap();
    assert_eq!(records.len(), 1, "{name}: {records:#?}");
    assert_header(&records[0], TITLE, generation);
    let expected = title(VALUE, Some(&daemon_title(release)));
    assert_eq!(records[0]["title"], expected, "{name}");
    assert_departed(report, steps, name, generation);
}

fn assert_departed(report: &Value, steps: &Value, name: &str, generation: u64) {
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
/// same-release instances share one load; private-dependency resolution is
/// not exercised, since releases ship none): with both releases installed,
/// one host process loads one release, then the other, through the shared
/// loader, from a working directory and with `LD_LIBRARY_PATH` and
/// `DYLD_FALLBACK_LIBRARY_PATH` holding the *other* release's library under
/// the library's file name, and `current` naming the other release. Two
/// instances of each release run at the same time, each against a headless
/// session of its own release: every one opens its passive connection,
/// relays the title only its own release's daemon produces, and departs
/// with a confirmed Leave. Each release's bootstrap and description are in
/// its own version folder's file, a second load of each shares the first,
/// and the process has loaded exactly the two files, once each. Both load
/// orders run, each against fresh sessions.
#[test]
fn mixed_releases_load_by_exact_version_and_run_side_by_side_whatever_current_cwd_and_search_paths()
{
    let package = Package::private();
    install_second_release(&package);
    let home = home(&package);
    let (first, second) = (library(&home, VERSION), library(&home, SECOND));
    for (primary, other, primary_library, other_library) in [
        (VERSION, SECOND, &first, &second),
        (SECOND, VERSION, &second, &first),
    ] {
        let a = Session::start(&package, VERSION, &format!("a loading {primary}"));
        let b = Session::start(&package, SECOND, &format!("b loading {primary}"));
        let instances = [
            ("a1", VERSION, &a, 1),
            ("b1", SECOND, &b, 2),
            ("a2", VERSION, &a, 3),
            ("b2", SECOND, &b, 4),
        ];
        let steps = simultaneous(&package, &instances);
        let decoys = package.isolation.home().join(format!("decoys of {other}"));
        let (directory, search) = (decoys.join("working directory"), decoys.join("search path"));
        for folder in [&directory, &search] {
            fs::create_dir_all(folder).unwrap();
            copy_executable(other_library, &folder.join(LIBRARY));
        }
        fs::write(home.path().join("current"), format!("{other}\n")).unwrap();
        let search = search.to_str().unwrap();
        let environment = [
            ("LD_LIBRARY_PATH", search),
            ("DYLD_FALLBACK_LIBRARY_PATH", search),
        ];
        let report = mixed_script(
            &package,
            (primary, &[other]),
            &directory,
            &environment,
            &steps,
        );
        for (name, release, _, generation) in instances {
            assert_registered(&report, &steps, name, (release, generation));
        }
        assert_loaded(&report, VERSION, &first);
        assert_loaded(&report, SECOND, &second);
        assert_eq!(
            report["images"],
            json!([primary_library, other_library]),
            "{report:#}"
        );
        a.kill(&package);
        b.kill(&package);
    }
    package.assert_clean();
}

/// LIB-01 Library case (exact-release pairing; the second-release fixture's
/// daemon has its release compiled in): a version folder 0.0.98 holding the
/// installed release's CLI with the second-release fixture's daemon cannot
/// start a session, because that daemon serves only 0.0.97, whatever its
/// folder is named; no session is left behind.
#[test]
fn second_release_daemon_serves_only_its_compiled_in_release() {
    let package = Package::private();
    let (daemon, library) = second_release_artifacts();
    install_release(&package, "0.0.98", &daemon, &library);
    let wait = r#"until [ "$SECONDS" -ge 120 ]; do sleep 0.05; done"#;
    let output = scripted(
        &package,
        "0.0.98/climon2-cli",
        &["exec", "--headless", "/bin/bash", "-c", wait],
    );
    assert_ne!(output.status.code(), Some(0), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let sessions = package.isolation.climon2_home().join("sessions");
    let left = fs::read_dir(&sessions).map_or(0, |entries| {
        entries
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == "pty")
            })
            .count()
    });
    assert_eq!(left, 0, "no session metadata");
    package.assert_clean();
}

/// LIB-03 Library case, identity separation (simultaneous instances of both
/// releases keep their own release's identity checks): in one process, with
/// instances of both releases registered with their own sessions, each
/// library still answers only for its own release. A create through one
/// release's table that expects the other release fails with
/// `CM1_RELEASE_MISMATCH` and no handles; an instance of one release whose
/// startup names the other release's session, served by the other release's
/// daemon, refuses its Open as `release_mismatch`, permanently, before any
/// role takes effect. A call that crossed releases, or a library that bound
/// to the other release's daemon, would accept what its own release refuses.
#[test]
fn mixed_release_instances_keep_their_own_release_identity_side_by_side() {
    let package = Package::private();
    install_second_release(&package);
    let (a, b) = (
        Session::start(&package, VERSION, "a"),
        Session::start(&package, SECOND, "b"),
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
    steps.as_array_mut().unwrap().splice(at..at, extra);
    let report = mixed_script(
        &package,
        (VERSION, &[SECOND]),
        package.isolation.home(),
        &[],
        &steps,
    );
    assert_registered(&report, &steps, "a", (VERSION, 1));
    assert_registered(&report, &steps, "b", (SECOND, 2));
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
        let error = Some((ORIGIN_LIBRARY, "release_mismatch"));
        assert_terminal(terminal, 5, refused, error);
    }
    let home = home(&package);
    assert_loaded(&report, VERSION, &library(&home, VERSION));
    assert_loaded(&report, SECOND, &library(&home, SECOND));
    a.kill(&package);
    b.kill(&package);
    package.assert_clean();
}

/// LIB-03 Fault case, fault containment (a fault in one release reaches
/// only that release's instances; the deliberately different private
/// behaviour shows no call crossed releases):
/// `CLIMON2_LIBRARY_FAULT=panic:receive` reaches the whole process, but
/// only the hooked release under test has the fault point, which panics
/// once per library image, at the first frame any of its connections
/// receives. The second-release fixture's instance opens and receives its
/// Initial metadata first, before the first release's instance is even
/// submitted: had its calls crossed into the hooked library, that first
/// panic would have been its own. Only then does the first release's
/// instance open; its connection panics, which the library contains and
/// reports as an `internal_error` failure, while the second-release
/// fixture's instance still departs with a confirmed Leave. Skipped
/// against an exact release archive, whose library has no hooks.
#[test]
fn a_fault_in_one_release_never_reaches_the_other_releases_instances() {
    if archive() {
        return;
    }
    let package = Package::private();
    install_second_release(&package);
    let (a, b) = (
        Session::start(&package, VERSION, "a"),
        Session::start(&package, SECOND, "b"),
    );
    let steps = json!([
        { "do": "create", "instance": "a", "release": VERSION, "startup": a.startup(&package, 1) },
        { "do": "create", "instance": "b", "release": SECOND, "startup": b.startup(&package, 2) },
        { "do": "submit", "instance": "b", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "b" },
        { "do": "drain", "instance": "b", "until": { "tag": INITIAL_METADATA } },
        { "do": "submit", "instance": "a", "request": open(1, &REQUIRED, &["title-v1"]) },
        { "do": "run", "instance": "a" },
        { "do": "drain", "instance": "a", "until": "terminal" },
        { "do": "shutdown", "instance": "b" },
        { "do": "finish", "instance": "a" },
        { "do": "finish", "instance": "b" },
    ]);
    let report = mixed_script(
        &package,
        (VERSION, &[SECOND]),
        package.isolation.home(),
        &[("CLIMON2_LIBRARY_FAULT", "panic:receive")],
        &steps,
    );
    assert_opened(&report, &steps, "b", 2);
    assert_departed(&report, &steps, "b", 2);
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
    let home = home(&package);
    assert_loaded(&report, VERSION, &library(&home, VERSION));
    assert_loaded(&report, SECOND, &library(&home, SECOND));
    a.kill(&package);
    b.kill(&package);
    package.assert_clean();
}
