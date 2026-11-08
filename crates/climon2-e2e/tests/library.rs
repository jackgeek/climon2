//! Library layer (#75): the installed `libclimon2_client.dylib`, loaded by the
//! `climon2-library-host` fixture through `dlopen`, `dlsym` and the frozen
//! revision-1 C ABI, never through Rust calls, static linking or a mock table.
//! In archive mode this is the exact release archive's universal library.
#![cfg(target_os = "macos")]

#[allow(dead_code)]
mod support;

use climon2_test_term::bin_path;
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use support::Package;

const LIBRARY: &str = "libclimon2_client.dylib";
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

/// The host with a cleared environment; instrumented builds keep their profile.
fn host(library: &Path, directory: &Path) -> Command {
    let mut command = Command::new(bin_path("climon2-library-host"));
    command.arg(library).current_dir(directory).env_clear();
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    command
}

fn observe(mut host: Command) -> Value {
    let output = host.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(
        output.stderr.is_empty(),
        "library or loader wrote: {output:?}"
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// LIB-01 Library case (absolute exact-release load, one shared load): the
/// installed dylib, loaded by absolute path with eager local resolution,
/// returns revision 1's immutable description. Its release text is exactly the
/// version folder name, it advertises no capability this build does not
/// implement, its requirements are present and bounded, and a second load
/// or bootstrap shares the same handle and description.
#[test]
fn installed_library_describes_revision_one_with_its_version_identity() {
    let package = Package::new();
    let library = installed_library(&package);
    let report = observe(host(&library, package.isolation.home()));
    assert_eq!(report["status"], SUCCESS, "{report:#}");
    assert_eq!(report["output"], "set");
    assert_eq!(report["image"], library.to_str().unwrap());
    assert_eq!(report["description_image"], library.to_str().unwrap());
    assert_eq!(report["handle_shared"], true);
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

/// LIB-01 Library case: the library loads from its absolute version path
/// whatever `current`, the working directory and ambient fallback search
/// paths name. Valid decoy copies sit in another version folder named by
/// `current`, in the working directory and on `DYLD_FALLBACK_LIBRARY_PATH`
/// (and `LD_LIBRARY_PATH`); the loaded image is still the requested file.
/// LIB-02 Library case (wrong release): the identity is compiled in, not
/// derived from the location, so the copy in the other version folder still
/// reports its own release and an exact-version check can reject it.
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
        fs::copy(&library, decoy).unwrap();
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

/// Defined external symbols of every architecture slice, from `nm`.
fn exported_symbols(library: &Path) -> Vec<Vec<String>> {
    let output = Command::new("nm")
        .args(["-gUj", "-arch", "all"])
        .arg(library)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let mut slices: Vec<Vec<String>> = Vec::new();
    for line in String::from_utf8(output.stdout).unwrap().lines() {
        if line.ends_with("):") && line.contains(" (for architecture ") {
            slices.push(Vec::new());
        } else if !line.is_empty() {
            if slices.is_empty() {
                slices.push(Vec::new());
            }
            slices.last_mut().unwrap().push(line.to_owned());
        }
    }
    slices
}

/// DOC-01 named case (exported interface): every architecture slice of the
/// installed library exports exactly the frozen bootstrap symbol, so Rust
/// symbols, unwinding and allocators never cross the interface by name.
#[test]
fn installed_library_exports_only_the_frozen_bootstrap() {
    let package = Package::new();
    let slices = exported_symbols(&installed_library(&package));
    assert!(!slices.is_empty());
    for symbols in slices {
        assert_eq!(symbols, ["_climon2_client_get_interface"]);
    }
    package.assert_clean();
}
