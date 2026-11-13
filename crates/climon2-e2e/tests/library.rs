//! Library layer (#75): the installed client library (`libclimon2_client.dylib`
//! on macOS, `libclimon2_client.so` on Linux), loaded by the
//! `climon2-library-host` fixture through `dlopen(RTLD_NOW | RTLD_LOCAL)`,
//! `dlsym` and the frozen revision-1 C ABI, never through Rust calls, static
//! linking or a mock table. In archive mode this is the exact release
//! archive's library.

#[allow(dead_code)]
mod support;

use climon2_test_term::copy_executable;
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::{
    LIBRARY, Package,
    library::{host, installed_library, observe},
};

const SUCCESS: u64 = 0;
const UNSUPPORTED_REVISION: u64 = 1;
const INVALID_ARGUMENT: u64 = 2;
const UNSUPPORTED_OPERATION: u64 = 3;
const ORIGIN_LIBRARY: u64 = 1;
const MIB: u64 = 1024 * 1024;

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

/// LIB-01 Library case (absolute exact-release load): the installed library,
/// loaded by absolute path with eager local resolution, returns revision 1's
/// immutable description. Its release text is exactly the version folder name,
/// it advertises no capability this build does not implement, its
/// requirements are present and bounded, and a second bootstrap returns the
/// same description.
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
/// Loading by absolute version path still maps the requested file. That is
/// the system loader's lookup, which the library relies on but cannot change,
/// so only a loader or host change fails it; the Linux release checks reject
/// the library properties through which a lookup could bind another release.
/// The library's own part is its identity: it answers from its own image with
/// its own folder's release, not the one `current` names, and the copy in the
/// other version folder reports its compiled-in release, not one derived from
/// its location.
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
    let misplaced = misplaced.canonicalize().unwrap();
    let report = observe(host(&misplaced, &directory));
    assert_eq!(report["image"], misplaced.to_str().unwrap());
    assert_eq!(version_folder(&misplaced), "0.0.1");
    assert_eq!(report["description"]["release"], version_folder(&library));
    package.assert_clean();
}
