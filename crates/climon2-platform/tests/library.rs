#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Fault layer for the shared client-library loader (#84). Each test owns a
//! private climon2 executable home whose version folders hold the
//! [client library stand-in](../../../GLOSSARY.md#testing), compiled by
//! `climon2_fixtures::stand_in` exact or with one defect, and loads it through
//! [`Loader`] the way the monitor and the client executable will. The
//! stand-in's calls abort, so a test process that survives a refusal proves
//! that nothing called them.

use climon2_core::{VersionName, layout::ExecutableHome};
use climon2_fixtures::stand_in::{self, DEPENDENCY};
use climon2_platform::library::{Failure, Library, LoadError, Loader};
use std::{
    ffi::{CStr, CString, OsStr, c_void},
    fs,
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
};

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("loader-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

fn version(text: &str) -> VersionName {
    text.parse().unwrap()
}

/// Whether the process still has `library` loaded: `RTLD_NOLOAD` only looks
/// it up and never loads it.
fn loaded(library: &Path) -> bool {
    let path = CString::new(library.as_os_str().as_bytes()).unwrap();
    // SAFETY: a NUL-terminated path; a handle found is deliberately kept.
    let handle = unsafe {
        libc::dlopen(
            path.as_ptr(),
            libc::RTLD_NOW | libc::RTLD_LOCAL | libc::RTLD_NOLOAD,
        )
    };
    !handle.is_null()
}

/// The file of the loaded image that contains `address`.
fn image(address: *const c_void) -> PathBuf {
    let mut info = MaybeUninit::<libc::Dl_info>::zeroed();
    // SAFETY: dladdr only writes the supplied Dl_info.
    assert_ne!(unsafe { libc::dladdr(address, info.as_mut_ptr()) }, 0);
    // SAFETY: dladdr succeeded, so dli_fname is a NUL-terminated path.
    let name = unsafe { CStr::from_ptr(info.assume_init().dli_fname) };
    PathBuf::from(OsStr::from_bytes(name.to_bytes()))
}

/// A private climon2 executable home.
struct Home {
    directory: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            directory: scratch(),
        }
    }

    /// The canonical home, as the monitor resolves it: dyld names a loaded
    /// image by the path it opened.
    fn home(&self) -> ExecutableHome {
        ExecutableHome::new(&self.directory.path().canonicalize().unwrap()).unwrap()
    }

    fn library(&self, version: &str) -> PathBuf {
        self.home().client_library(&self::version(version))
    }

    /// `version`'s library path, with its version folder created.
    fn folder(&self, version: &str) -> PathBuf {
        let library = self.library(version);
        fs::create_dir_all(library.parent().unwrap()).unwrap();
        library
    }

    /// Installs the stand-in describing `release` as `version`'s library.
    fn stand_in(&self, version: &str, release: &str, options: &[&str]) -> PathBuf {
        let library = self.folder(version);
        stand_in::compile(&library, release, options).unwrap();
        library
    }

    /// Installs a stand-in that needs a library found nowhere.
    fn stand_in_needing_a_missing_library(&self, version: &str) -> PathBuf {
        let library = self.folder(version);
        let scratch = scratch();
        stand_in::compile_needing_a_missing_library(&library, version, scratch.path()).unwrap();
        library
    }

    /// Installs a stand-in built for another architecture than this
    /// process's: an x86-64 one whose ELF header names AArch64 on Linux, or
    /// a thin library of the other Mac architecture.
    fn stand_in_for_another_architecture(&self, version: &str) -> PathBuf {
        if cfg!(target_os = "linux") {
            let library = self.stand_in(version, version, &[]);
            stand_in::name_aarch64(&library).unwrap();
            return library;
        }
        let other = if cfg!(target_arch = "aarch64") {
            "x86_64"
        } else {
            "arm64"
        };
        self.stand_in(version, version, &["-arch", other])
    }

    fn load(&self, version: &str) -> Result<&'static Library, LoadError> {
        Loader::new(self.home()).load(&self::version(version))
    }

    /// The loader's refusal of `version`, which names that version's library.
    fn refusal(&self, version: &str) -> LoadError {
        let Err(error) = self.load(version) else {
            panic!("{version} loaded");
        };
        assert_eq!(error.library(), self.library(version), "{error}");
        error
    }
}

/// LIB-02 Fault (missing library; no substituted release): a version folder
/// without its library, and a version without a folder, are refused as
/// missing libraries naming the requested file. The valid library of the
/// version `current` names is never used instead: it is not even loaded.
#[test]
fn missing_library_is_refused_without_substituting_another_release() {
    let home = Home::new();
    let current = home.stand_in("0.1.0", "0.1.0", &[]);
    fs::write(home.home().path().join("current"), "0.1.0\n").unwrap();
    home.folder("0.2.0");
    for missing in ["0.2.0", "0.3.0"] {
        let error = home.refusal(missing);
        assert_eq!(error.failure(), Failure::MissingLibrary, "{error}");
        let message = error.to_string();
        assert!(
            message.contains(&home.library(missing).display().to_string()),
            "{message}"
        );
    }
    assert!(!loaded(&current));
}

/// LIB-01 Fault (same-release instances share one load), with the exact
/// stand-in: two loads of one version through one loader return the same
/// library, and another version's library is a separate load. Each library
/// reports the requirements its description declares and the complete
/// function table of its own image, and stays loaded.
#[test]
fn two_loads_of_one_version_share_one_library() {
    let home = Home::new();
    let first_path = home.stand_in("0.1.0", "0.1.0", &[]);
    let other_path = home.stand_in("0.2.0", "0.2.0", &[]);
    let loader = Loader::new(home.home());
    let first = loader.load(&version("0.1.0")).unwrap();
    let again = loader.load(&version("0.1.0")).unwrap();
    let other = loader.load(&version("0.2.0")).unwrap();
    assert!(ptr::eq(first, again));
    assert!(!ptr::eq(first, other));
    let requirements = first.requirements();
    assert_eq!(
        [
            requirements.release_bytes,
            requirements.instance_bytes,
            requirements.owner_stack_bytes,
            requirements.worker_stack_bytes,
        ],
        [5_000_000, 1_000_000, 2_097_152, 1_048_576]
    );
    assert_eq!(
        [
            requirements.workers,
            requirements.instance_descriptors,
            requirements.load_descriptors,
            requirements.reserved,
        ],
        [2, 3, 4, 0]
    );
    for (library, path) in [(first, &first_path), (other, &other_path)] {
        let functions = library.functions();
        assert_eq!(image(functions.create as *const c_void), *path);
        assert_eq!(image(functions.snapshot as *const c_void), *path);
        assert!(loaded(path));
    }
}

/// LIB-02 Fault (missing symbol): a library that loads but does not export
/// `climon2_client_get_interface` is refused as a missing symbol before
/// anything calls it, and its opened handle stays loaded.
#[test]
fn library_without_the_bootstrap_is_refused_and_stays_loaded() {
    let home = Home::new();
    let library = home.stand_in("0.1.0", "0.1.0", &["-DNO_BOOTSTRAP"]);
    let error = home.refusal("0.1.0");
    assert_eq!(error.failure(), Failure::MissingSymbol, "{error}");
    assert!(
        error.to_string().contains("climon2_client_get_interface"),
        "{error}"
    );
    assert!(loaded(&library));
}

/// LIB-02 Fault (unsupported revision; malformed description): a library
/// whose bootstrap refuses revision 1 is refused as unsupported. One that
/// answers revision 1 with success but no description, or with a status the
/// bootstrap contract does not allow when output storage is given
/// (`CM1_INVALID_ARGUMENT`), breaks that contract and is refused as a
/// malformed description. Each opened handle stays loaded.
#[test]
fn bootstrap_without_a_revision_one_description_is_refused() {
    let home = Home::new();
    for (version, option, failure, diagnosis) in [
        (
            "0.1.0",
            "-DUNSUPPORTED",
            Failure::UnsupportedRevision,
            "revision 1",
        ),
        (
            "0.2.0",
            "-DBOOTSTRAP_STATUS=0",
            Failure::MalformedDescription,
            "status 0 and no description",
        ),
        (
            "0.3.0",
            "-DBOOTSTRAP_STATUS=2",
            Failure::MalformedDescription,
            "status 2 and no description",
        ),
    ] {
        let library = home.stand_in(version, version, &[option]);
        let error = home.refusal(version);
        assert_eq!(error.failure(), failure, "{option}: {error}");
        assert!(error.to_string().contains(diagnosis), "{option}: {error}");
        assert!(loaded(&library), "{option}");
    }
}

/// LIB-02 Fault (contained bootstrap failure): a bootstrap that answers
/// revision 1 with `CM1_INTERNAL_ERROR` and clears the output it was given, a
/// contained failure the bootstrap contract allows, is refused as a bootstrap
/// failure, not a malformed description. It is recoverable: the loader does
/// not remember the refusal, so when the failure has passed, the next load of
/// the version succeeds.
#[test]
fn contained_bootstrap_failure_is_refused_and_recoverable() {
    let home = Home::new();
    home.stand_in("0.1.0", "0.1.0", &["-DFAIL_FIRST_BOOTSTRAP"]);
    let loader = Loader::new(home.home());
    let Err(error) = loader.load(&version("0.1.0")) else {
        panic!("the first bootstrap succeeded");
    };
    assert_eq!(error.failure(), Failure::BootstrapFailure, "{error}");
    assert!(error.to_string().contains("CM1_INTERNAL_ERROR"), "{error}");
    let library = loader.load(&version("0.1.0")).unwrap();
    assert_eq!(library.requirements().workers, 2);
}

/// LIB-02 Fault (contained bootstrap failure; malformed description; owner
/// decision on #84): a bootstrap that answers revision 1 with
/// `CM1_INTERNAL_ERROR` but leaves the output it was given as it found it
/// breaks the bootstrap contract, so it is refused as a malformed
/// description, not as a recoverable bootstrap failure. The opened handle
/// stays loaded.
#[test]
fn contained_bootstrap_failure_that_leaves_its_output_is_a_malformed_description() {
    let home = Home::new();
    let library = home.stand_in(
        "0.1.0",
        "0.1.0",
        &["-DFAIL_FIRST_BOOTSTRAP", "-DUNCLEARED_FAILURE"],
    );
    let error = home.refusal("0.1.0");
    assert_eq!(error.failure(), Failure::MalformedDescription, "{error}");
    assert!(
        error
            .to_string()
            .contains("CM1_INTERNAL_ERROR) without clearing its output"),
        "{error}"
    );
    assert!(loaded(&library));
}

/// LIB-02 Fault (wrong release; malformed description), at the release
/// text's bounds: a library describing another release than its version
/// folder names is refused as the wrong release, naming both, even with a
/// 255-byte release text, as long as a version folder name can be. A 256-byte
/// release text, or none, cannot name any version folder and is refused as a
/// malformed description. Each opened handle stays loaded.
#[test]
fn library_describing_another_release_is_refused() {
    let home = Home::new();
    let longest = "9".repeat(255);
    let longer = "9".repeat(256);
    let quoted = format!("release \"{longest}\", not 0.2.0");
    for (version, release, option, failure, diagnosis) in [
        (
            "0.1.0",
            "9.9.9",
            None,
            Failure::WrongRelease,
            "release \"9.9.9\", not 0.1.0",
        ),
        ("0.2.0", &longest, None, Failure::WrongRelease, &quoted),
        (
            "0.3.0",
            &longer,
            None,
            Failure::MalformedDescription,
            "release text of 256 bytes",
        ),
        (
            "0.4.0",
            "0.4.0",
            Some("-DNULL_RELEASE"),
            Failure::MalformedDescription,
            "release text of 0 bytes",
        ),
    ] {
        let library = home.stand_in(version, release, option.as_slice());
        let error = home.refusal(version);
        assert_eq!(error.failure(), failure, "{version}: {error}");
        assert!(error.to_string().contains(diagnosis), "{version}: {error}");
        assert!(loaded(&library), "{version}");
    }
}

/// LIB-02 Fault (malformed description): a revision-1 description that breaks
/// the native contract is refused as malformed before any instance exists:
/// another revision or a nonzero reserved field; a missing requirement (no
/// instance allocation, owner stack or instance descriptor, or a worker,
/// even one, without a stack) or a nonzero reserved requirement; an
/// incomplete function table. Each opened handle stays loaded.
#[test]
fn malformed_description_is_refused_before_any_instance() {
    let home = Home::new();
    for (version, options, diagnosis) in [
        ("0.1.0", &["-DREVISION=2"][..], "revision 2"),
        ("0.2.0", &["-DRESERVED=1"], "reserved field 1"),
        ("0.3.0", &["-DINSTANCE_BYTES=0"], "requirements"),
        ("0.4.0", &["-DOWNER_STACK_BYTES=0"], "requirements"),
        ("0.5.0", &["-DINSTANCE_DESCRIPTORS=0"], "requirements"),
        ("0.6.0", &["-DWORKER_STACK_BYTES=0"], "requirements"),
        (
            "0.7.0",
            &["-DWORKERS=1", "-DWORKER_STACK_BYTES=0"],
            "requirements",
        ),
        ("0.8.0", &["-DREQUIREMENTS_RESERVED=1"], "requirements"),
        ("0.9.0", &["-DINCOMPLETE_TABLE"], "function table"),
    ] {
        let library = home.stand_in(version, version, options);
        let error = home.refusal(version);
        assert_eq!(
            error.failure(),
            Failure::MalformedDescription,
            "{options:?}: {error}"
        );
        assert!(
            error.to_string().contains(diagnosis),
            "{options:?}: {error}"
        );
        assert!(loaded(&library), "{options:?}");
    }
}

/// LIB-02 Fault (present requirements, at their limits): the loader requires
/// only that each requirement is present, never an upper bound, which
/// admission owns. Every nonzero minimum is accepted at 1: one byte of
/// instance allocation and of owner stack, one instance descriptor, and one
/// worker with a one-byte stack. A release that needs no workers needs no
/// worker stack. The library reports exactly the requirements declared.
#[test]
fn requirements_are_accepted_at_their_minimums() {
    let home = Home::new();
    home.stand_in(
        "0.1.0",
        "0.1.0",
        &[
            "-DINSTANCE_BYTES=1",
            "-DOWNER_STACK_BYTES=1",
            "-DINSTANCE_DESCRIPTORS=1",
            "-DWORKERS=1",
            "-DWORKER_STACK_BYTES=1",
        ],
    );
    let requirements = *home.load("0.1.0").unwrap().requirements();
    assert_eq!(
        [
            requirements.instance_bytes,
            requirements.owner_stack_bytes,
            requirements.worker_stack_bytes,
        ],
        [1, 1, 1]
    );
    assert_eq!(
        [requirements.workers, requirements.instance_descriptors],
        [1, 1]
    );
    home.stand_in("0.2.0", "0.2.0", &["-DWORKERS=0", "-DWORKER_STACK_BYTES=0"]);
    let requirements = *home.load("0.2.0").unwrap().requirements();
    assert_eq!(
        [requirements.workers, requirements.worker_stack_bytes as u32],
        [0, 0]
    );
}

/// LIB-02 Fault (missing dependency; incompatible architecture), classified
/// by the loader's own checks once the dynamic loader fails, never from that
/// loader's message: a library built for this process's architecture that
/// needs a library found nowhere is refused as a missing dependency, quoting
/// the dynamic loader's message only as diagnosis. A library for another
/// architecture, and a file that is no library at all, are refused as an
/// incompatible architecture. A failed load leaves nothing loaded.
#[test]
fn library_the_dynamic_loader_cannot_load_is_classified_by_the_loaders_checks() {
    let home = Home::new();
    let dependent = home.stand_in_needing_a_missing_library("0.1.0");
    let error = home.refusal("0.1.0");
    assert_eq!(error.failure(), Failure::MissingDependency, "{error}");
    assert!(error.to_string().contains(DEPENDENCY), "{error}");
    assert!(!loaded(&dependent));
    let foreign = home.stand_in_for_another_architecture("0.2.0");
    let text = home.folder("0.3.0");
    fs::write(&text, "not a client library\n").unwrap();
    for (version, library) in [("0.2.0", &foreign), ("0.3.0", &text)] {
        let error = home.refusal(version);
        assert_eq!(
            error.failure(),
            Failure::IncompatibleArchitecture,
            "{version}: {error}"
        );
        assert!(!loaded(library), "{version}");
    }
}

/// Names the requested executable home in the child of
/// `library_substituted_through_dyld_library_path_is_refused_even_without_a_bootstrap`.
#[cfg(target_os = "macos")]
const DECOY_CHILD: &str = "CLIMON2_LOADER_DECOY_HOME";

/// LIB-01 Fault, macOS (exact lookup prevails over `DYLD_LIBRARY_PATH`; owner
/// decisions on #84): in a process whose `DYLD_LIBRARY_PATH` names a
/// directory holding a stand-in of the same file name that lacks
/// `climon2_client_get_interface`, dyld loads that decoy for the requested
/// path. The loader checks substitution before blaming a missing symbol on the
/// requested file: it refuses a substituted image, naming the decoy.
#[cfg(target_os = "macos")]
#[test]
fn library_substituted_through_dyld_library_path_is_refused_even_without_a_bootstrap() {
    if let Some(home) = std::env::var_os(DECOY_CHILD) {
        let home = ExecutableHome::new(Path::new(&home)).unwrap();
        let Err(error) = Loader::new(home).load(&version("0.1.0")) else {
            panic!("the decoy loaded");
        };
        assert_eq!(error.failure(), Failure::SubstitutedImage, "{error}");
        let decoys = PathBuf::from(std::env::var_os("DYLD_LIBRARY_PATH").unwrap());
        let decoy = decoys.join(climon2_core::layout::client_library_name());
        assert!(
            error.to_string().contains(decoy.to_str().unwrap()),
            "{error}"
        );
        return;
    }
    let home = Home::new();
    home.stand_in("0.1.0", "0.1.0", &[]);
    let decoys = scratch();
    let decoys = decoys.path().canonicalize().unwrap();
    let decoy = decoys.join(climon2_core::layout::client_library_name());
    stand_in::compile(&decoy, "0.1.0", &["-DNO_BOOTSTRAP"]).unwrap();
    climon2_fixtures::run(
        std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "library_substituted_through_dyld_library_path_is_refused_even_without_a_bootstrap",
                "--nocapture",
            ])
            .env(DECOY_CHILD, home.home().path())
            .env("DYLD_LIBRARY_PATH", &decoys),
        &decoys.join("child.log"),
    )
    .unwrap();
}
