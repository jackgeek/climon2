//! Library layer (#75, #84): the installed client library
//! (`libclimon2_client.dylib` on macOS, `libclimon2_client.so` on Linux),
//! loaded by the `climon2-library-host` fixture through the shared validated
//! loader (`climon2_platform::library`), which opens it with
//! `dlopen(RTLD_NOW | RTLD_LOCAL)`, and observed through the frozen
//! revision-1 C ABI, never through Rust calls, static linking or a mock
//! table. In archive mode this is the exact release archive's library.

#[allow(dead_code)]
mod support;

use climon2_test_term::{bin_path, copy_executable, output_within, process_command};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::{LIBRARY, Package, TIMEOUT};

const SUCCESS: u64 = 0;
const UNSUPPORTED_REVISION: u64 = 1;
const INVALID_ARGUMENT: u64 = 2;
const UNSUPPORTED_OPERATION: u64 = 3;
const ORIGIN_LIBRARY: u64 = 1;
const MIB: u64 = 1024 * 1024;

/// The installed library of this package's version, by absolute path.
fn installed_library(package: &Package) -> PathBuf {
    package
        .executable_home()
        .join(env!("CARGO_PKG_VERSION"))
        .join(LIBRARY)
        .canonicalize()
        .unwrap()
}

/// The version folder name, which is the library's release identity.
fn version_folder(library: &Path) -> &str {
    library
        .parent()
        .unwrap()
        .file_name()
        .unwrap()
        .to_str()
        .unwrap()
}

/// The host loading `library`'s version from its executable home, with a
/// cleared environment; instrumented builds keep their profile.
fn host(library: &Path, directory: &Path) -> Command {
    let mut command = process_command(bin_path("climon2-library-host"));
    command
        .arg(library.parent().unwrap().parent().unwrap())
        .arg(version_folder(library))
        .current_dir(directory)
        .env_clear();
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

fn observe(mut host: Command) -> Value {
    let output = output_within(&mut host, TIMEOUT);
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "library or loader wrote: {output:?}"
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// LIB-01 Library case (absolute exact-release load; same-release instances
/// share one load): the loader accepts the installed library, loaded by
/// absolute path with eager local resolution, which returns revision 1's
/// immutable description. Its release text is exactly the version folder name,
/// it advertises no capability this build does not implement, its
/// requirements are present and bounded, and a second bootstrap returns the
/// same description. A second load of the version shares the first, and the
/// process has loaded that one client library file.
#[test]
fn installed_library_describes_revision_one_with_its_version_identity() {
    let package = Package::new();
    let library = installed_library(&package);
    let report = observe(host(&library, package.isolation.home()));
    assert_eq!(report["status"], SUCCESS, "{report:#}");
    assert_eq!(report["output"], "set");
    assert_eq!(report["image"], library.to_str().unwrap());
    assert_eq!(report["description_image"], library.to_str().unwrap());
    assert_eq!(report["stable"], true);
    assert_eq!(report["shared"], true);
    assert_eq!(report["images"], json!([library]));
    let description = &report["description"];
    assert_eq!(description["revision"], 1);
    assert_eq!(description["reserved"], 0);
    assert_eq!(description["release"], version_folder(&library));
    assert_eq!(
        description["capabilities"],
        json!({ "null": true, "count": 0, "tokens": [] })
    );
    assert_bounded(&description["requirements"]);
    package.assert_clean();
}

/// Present and bounded: every requirement fits the default `monitor.limits`
/// reservations (docs/monitor-operational-limits.md), so a default-configured
/// monitor admits the release without raising a reservation. Resources every
/// instance needs (owner stack, local allocation, notification descriptor)
/// are nonzero; workers need stacks.
fn assert_bounded(requirements: &Value) {
    let field = |name: &str| requirements[name].as_u64().unwrap();
    assert!(field("release_bytes") <= 64 * MIB, "{requirements}");
    assert!(
        (1..=64 * MIB).contains(&field("instance_bytes")),
        "{requirements}"
    );
    assert!(
        (1..=2 * MIB).contains(&field("owner_stack_bytes")),
        "{requirements}"
    );
    assert!(field("workers") <= 2, "{requirements}");
    let worker_stack = if field("workers") == 0 { 0 } else { 1 };
    assert!(
        (worker_stack..=2 * MIB).contains(&field("worker_stack_bytes")),
        "{requirements}"
    );
    assert!(
        (1..=8).contains(&field("instance_descriptors")),
        "{requirements}"
    );
    assert!(field("load_descriptors") <= 4, "{requirements}");
    assert_eq!(field("reserved"), 0, "{requirements}");
}

/// LIB-02 Library case (unsupported revision): any revision other than 1 is
/// rejected with `CM1_UNSUPPORTED_REVISION` and a null description, and null
/// output storage is `CM1_INVALID_ARGUMENT`, before any instance exists.
#[test]
fn installed_library_rejects_other_revisions_without_a_description() {
    let package = Package::new();
    let library = installed_library(&package);
    let report = observe(host(&library, package.isolation.home()));
    for revision in [0u64, 2, u32::MAX.into()] {
        assert!(
            report["other_revisions"]
                .as_array()
                .unwrap()
                .contains(&json!({
                    "revision": revision,
                    "status": UNSUPPORTED_REVISION,
                    "output": "null",
                })),
            "{report:#}"
        );
    }
    assert_eq!(report["null_output_status"], INVALID_ARGUMENT);
    package.assert_clean();
}

/// DOC-01 named case (implemented table versus the frozen call rules): all
/// eleven function-table entries exist. Until #85 and #93 implement them, each
/// fails with `CM1_UNSUPPORTED_OPERATION`, library origin, zero reserved
/// fields and a nonempty stable error code and message; failed calls clear
/// their output structs, notification writes -1, and null outputs are not
/// written.
#[test]
fn installed_library_table_reports_every_instance_call_unsupported() {
    let package = Package::new();
    let library = installed_library(&package);
    let report = observe(host(&library, package.isolation.home()));
    let functions = &report["description"]["functions"];
    let entries = functions["entries"].as_object().unwrap();
    let names = [
        "create",
        "run",
        "destroy",
        "submit",
        "drain",
        "release_delivery",
        "shutdown",
        "release_communication",
        "release_control",
        "notification",
        "snapshot",
    ];
    assert_eq!(entries.len(), names.len(), "{functions:#}");
    for name in names {
        let entry = &entries[name];
        assert_eq!(entry["code"], UNSUPPORTED_OPERATION, "{name}: {entry}");
        assert_eq!(entry["reserved"], 0, "{name}: {entry}");
        assert_eq!(entry["origin"], ORIGIN_LIBRARY, "{name}: {entry}");
        assert_eq!(entry["error_reserved"], 0, "{name}: {entry}");
        assert_eq!(entry["error_code"], "unsupported_operation", "{name}");
        assert!(!entry["message"].as_str().unwrap().is_empty(), "{name}");
    }
    for name in ["create", "run", "drain", "snapshot"] {
        assert_eq!(entries[name]["output"], "zeroed", "{name}");
    }
    assert_eq!(entries["notification"]["descriptor"], -1);
    assert_eq!(
        functions["null_outputs"],
        json!({
            "create": UNSUPPORTED_OPERATION,
            "run": UNSUPPORTED_OPERATION,
            "drain": UNSUPPORTED_OPERATION,
            "notification": UNSUPPORTED_OPERATION,
            "snapshot": UNSUPPORTED_OPERATION,
        })
    );
    package.assert_clean();
}

/// LIB-01 Library case: `current` names another version folder, and valid
/// copies of the library sit there, in the working directory and on
/// `LD_LIBRARY_PATH` (Linux) and `DYLD_FALLBACK_LIBRARY_PATH` (macOS).
/// Loading by absolute version path still maps the requested file, and only
/// that one: the system loader's lookup, which the library relies on but
/// cannot change, so only a loader or host change fails it; the Linux release
/// checks reject the library properties through which a lookup could bind
/// another release. The library's own part is its identity: it answers from
/// its own image with its own folder's release, not the one `current` names.
#[test]
fn installed_library_loads_by_absolute_version_path_whatever_current_cwd_and_search_paths() {
    let package = Package::private();
    let library = installed_library(&package);
    let home = package.executable_home();
    let misplaced = home.join("0.0.1").join(LIBRARY);
    let private = home.parent().unwrap();
    let directory = private.join("working directory");
    let search = private.join("search path");
    for decoy in [&misplaced, &directory.join(LIBRARY), &search.join(LIBRARY)] {
        fs::create_dir_all(decoy.parent().unwrap()).unwrap();
        copy_executable(&library, decoy);
    }
    fs::write(home.join("current"), "0.0.1\n").unwrap();
    let mut ambient = host(&library, &directory);
    ambient
        .env("DYLD_FALLBACK_LIBRARY_PATH", &search)
        .env("LD_LIBRARY_PATH", &search);
    let report = observe(ambient);
    assert_eq!(report["status"], SUCCESS, "{report:#}");
    assert_eq!(report["image"], library.to_str().unwrap());
    assert_eq!(report["description_image"], library.to_str().unwrap());
    assert_eq!(report["description"]["release"], version_folder(&library));
    assert_eq!(report["images"], json!([library]));
    package.assert_clean();
}

/// LIB-02 Library case (wrong release; missing library; no substituted
/// release): `current` names 0.0.1, whose folder holds a copy of the
/// installed library, which describes its own compiled-in release, not
/// 0.0.1. Loading 0.0.1 is refused as the wrong release, naming the release
/// the copy describes, and that refused copy stays loaded; the installed
/// library of the release it describes is not loaded instead. A version
/// folder without a library is refused as a missing library, and nothing is
/// loaded.
#[test]
fn installed_library_of_another_release_or_none_is_refused_without_substitution() {
    let package = Package::private();
    let library = installed_library(&package);
    let home = library.parent().unwrap().parent().unwrap();
    let misplaced = home.join("0.0.1").join(LIBRARY);
    fs::create_dir(misplaced.parent().unwrap()).unwrap();
    copy_executable(&library, &misplaced);
    fs::create_dir(home.join("0.0.2")).unwrap();
    fs::write(home.join("current"), "0.0.1\n").unwrap();
    let report = observe(host(&misplaced, package.isolation.home()));
    assert_eq!(report["error"]["failure"], "WrongRelease", "{report:#}");
    let message = report["error"]["message"].as_str().unwrap();
    let described = format!("release \"{}\", not 0.0.1", version_folder(&library));
    assert!(message.contains(&described), "{message}");
    assert_eq!(report["images"], json!([misplaced]));
    let missing = home.join("0.0.2").join(LIBRARY);
    let report = observe(host(&missing, package.isolation.home()));
    assert_eq!(report["error"]["failure"], "MissingLibrary", "{report:#}");
    let message = report["error"]["message"].as_str().unwrap();
    assert!(message.contains(missing.to_str().unwrap()), "{message}");
    assert_eq!(report["images"], json!([]));
    package.assert_clean();
}

/// A private directory beside the build's other test scratch.
fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("library-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// LIB-01 Library case on Linux (the user's loader environment, library
/// embedding): `LD_PRELOAD` and `LD_AUDIT` name an audit library that accepts
/// every interface version, `LD_LIBRARY_PATH` names a directory and
/// `GLIBC_TUNABLES` sets a malloc tunable. The installed library still loads
/// from its absolute version path, and the four variables are exactly as they
/// were: the loader neither clears them nor refuses to load because they are
/// set.
#[cfg(target_os = "linux")]
#[test]
fn installed_library_loads_under_the_users_loader_environment() {
    let package = Package::new();
    let library = installed_library(&package);
    let scratch = scratch();
    let directory = scratch.path().canonicalize().unwrap();
    let source = directory.join("audit.c");
    fs::write(
        &source,
        "unsigned la_version(unsigned version) { return version; }\n",
    )
    .unwrap();
    let audit = directory.join("libclimon2_audit.so");
    let output = output_within(
        Command::new("cc")
            .args(["-shared", "-fPIC", "-o"])
            .arg(&audit)
            .arg(&source),
        TIMEOUT,
    );
    assert!(output.status.success(), "{output:?}");
    let audit = audit.to_str().unwrap();
    let variables = [
        ("LD_PRELOAD", audit),
        ("LD_AUDIT", audit),
        ("LD_LIBRARY_PATH", directory.to_str().unwrap()),
        ("GLIBC_TUNABLES", "glibc.malloc.tcache_count=7"),
    ];
    let mut host = host(&library, &directory);
    host.envs(variables);
    let report = observe(host);
    assert_eq!(report["status"], SUCCESS, "{report:#}");
    assert_eq!(report["images"], json!([library]));
    for (name, value) in variables {
        assert_eq!(report["environment"][name], value, "{name}");
    }
    package.assert_clean();
}

/// LIB-01 Library case on macOS (exact lookup prevails over
/// `DYLD_LIBRARY_PATH`; owner decision on #84): dyld looks for the library's
/// file name in every `DYLD_LIBRARY_PATH` directory before it opens an
/// absolute path, so a copy of the library there is what it loads. The loader
/// finds with `dladdr` that the loaded image is not the requested file and
/// refuses the substitute, naming it; the substitute it opened stays loaded.
#[cfg(target_os = "macos")]
#[test]
fn installed_library_substituted_through_dyld_library_path_is_refused() {
    let package = Package::new();
    let library = installed_library(&package);
    let scratch = scratch();
    let search = scratch.path().canonicalize().unwrap();
    let decoy = search.join(LIBRARY);
    copy_executable(&library, &decoy);
    let mut substituted = host(&library, &search);
    substituted.env("DYLD_LIBRARY_PATH", &search);
    let report = observe(substituted);
    assert_eq!(report["error"]["failure"], "SubstitutedImage", "{report:#}");
    let message = report["error"]["message"].as_str().unwrap();
    assert!(message.contains(library.to_str().unwrap()), "{message}");
    assert!(message.contains(decoy.to_str().unwrap()), "{message}");
    assert_eq!(report["images"], json!([decoy]));
    package.assert_clean();
}
