#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Client embedding's one shared loader (#489), over the
//! [client library stand-in](../../../GLOSSARY.md#testing) in a private
//! climon2 executable home: a loaded release hands its consumer the
//! release's calls and qualified requirements, and a refusal the loader's
//! failure and message.
use climon2_core::{VersionName, layout::ExecutableHome};
use climon2_embed::load::{Refusal, loader};
use climon2_fixtures::stand_in;
use climon2_platform::library::{Failure, Functions, Loader};
use std::{fs, ptr};

fn version(text: &str) -> VersionName {
    text.parse().unwrap()
}

/// LIB-02 (over the stand-in): a release's validated library loads with
/// the calls and the requirements the platform loader reads from it, the
/// same on every load; a version without a library is refused as a missing
/// library, with the loader's message.
#[test]
fn a_release_loads_with_its_calls_and_requirements_or_the_loaders_refusal() {
    let directory = tempfile::Builder::new()
        .prefix("embed-loader-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let home = ExecutableHome::new(&directory.path().canonicalize().unwrap()).unwrap();
    let library = home.client_library(&version("0.4.89"));
    fs::create_dir_all(library.parent().unwrap()).unwrap();
    stand_in::compile(&library, "0.4.89", &["-DWORKERS=2"]).unwrap();

    let load = loader(home.clone());
    let loaded = load(&version("0.4.89")).unwrap();
    let requirements = loaded.requirements;
    assert_eq!(requirements.workers, 2);
    assert_eq!(
        [requirements.release_bytes, requirements.instance_bytes],
        [5_000_000, 1_000_000]
    );
    let direct = Loader::new(home.clone()).load(&version("0.4.89")).unwrap();
    let calls = |functions: &Functions| [functions.create as usize, functions.drain as usize];
    assert_eq!(calls(loaded.functions), calls(direct.functions()));
    let again = load(&version("0.4.89")).unwrap();
    assert!(ptr::eq(loaded.functions, again.functions));

    let Err(refusal) = load(&version("0.4.90")) else {
        panic!("a version without a library loaded");
    };
    let Refusal::Library(failure, message) = &refusal else {
        panic!("{refusal:?}");
    };
    assert_eq!(*failure, Failure::MissingLibrary);
    let direct = Loader::new(home).load(&version("0.4.90"));
    assert_eq!(*message, direct.err().unwrap().to_string());
    assert!(!message.is_empty());
}
