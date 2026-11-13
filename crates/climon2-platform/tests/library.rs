#![cfg(any(target_os = "macos", target_os = "linux"))]
//! Fault layer for the shared client-library loader (#84). Each test owns a
//! private climon2 executable home whose version folders hold the test-only
//! client library stand-in (`crates/climon2-fixtures/stand-in/client-library.c`),
//! compiled here exact or with one defect, and loads it through [`Loader`] the
//! way the monitor and the client executable will. The stand-in's calls abort,
//! so a test process that survives a refusal proves that nothing called them.

use climon2_core::{VersionName, layout::client_library_name};
use climon2_platform::library::{Failure, Library, LoadError, Loader};
use std::{
    env::consts::{DLL_PREFIX, DLL_SUFFIX},
    ffi::{CStr, CString, OsStr, c_void},
    fs::{self, File},
    mem::MaybeUninit,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    ptr, thread,
    time::{Duration, Instant},
};

/// How long one compiler run may take before it is killed and reaped.
const COMPILE_TIMEOUT: Duration = Duration::from_secs(120);

#[cfg(target_os = "linux")]
const SHARED: &[&str] = &["-shared", "-fPIC"];
#[cfg(target_os = "macos")]
const SHARED: &[&str] = &["-dynamiclib"];
/// The library the dependent stand-in needs, which no test installs.
const DEPENDENCY: &str = "climon2_stand_in_dependency";
/// Linker options that record a needed library even though nothing of it is
/// used (Ubuntu's toolchain links `--as-needed` by default).
#[cfg(target_os = "linux")]
const KEEP_NEEDED: &[&str] = &["-Wl,--no-as-needed"];
#[cfg(target_os = "macos")]
const KEEP_NEEDED: &[&str] = &[];

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("loader-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// Runs `command`, waiting at most `COMPILE_TIMEOUT`; its diagnostics go to
/// `log`, which a failure reports.
fn run(command: &mut Command, log: &Path) {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(File::create(log).unwrap())
        .stderr(File::create(log).unwrap())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + COMPILE_TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{command:?} did not finish within {COMPILE_TIMEOUT:?}");
        }
        thread::sleep(Duration::from_millis(10));
    };
    let diagnostics = fs::read_to_string(log).unwrap();
    assert!(status.success(), "{command:?}: {status}\n{diagnostics}");
}

/// Compiles the stand-in describing `release`, with `options`, to `output`.
fn compile(output: &Path, release: &str, options: &[&str]) {
    let root = workspace();
    fs::create_dir_all(output.parent().unwrap()).unwrap();
    run(
        Command::new("cc")
            .args(SHARED)
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-I"])
            .arg(root.join("docs/protocol/monitor-client"))
            .arg(format!("-DRELEASE=\"{release}\""))
            .args(options)
            .arg("-o")
            .arg(output)
            .arg(root.join("crates/climon2-fixtures/stand-in/client-library.c")),
        &output.with_extension("log"),
    );
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
    fn path(&self) -> PathBuf {
        self.directory.path().canonicalize().unwrap()
    }

    fn library(&self, version: &str) -> PathBuf {
        self.path().join(version).join(client_library_name())
    }

    /// Installs the stand-in describing `release` as `version`'s library.
    fn stand_in(&self, version: &str, release: &str, options: &[&str]) -> PathBuf {
        let library = self.library(version);
        compile(&library, release, options);
        library
    }

    /// Installs a stand-in that needs a library found nowhere: it is built
    /// against one that is then deleted. On macOS that library would sit
    /// beside the stand-in; on Linux any search would fail.
    fn stand_in_needing_a_missing_library(&self, version: &str) -> PathBuf {
        let directory = scratch();
        let name = format!("{DLL_PREFIX}{DEPENDENCY}{DLL_SUFFIX}");
        let dependency = directory.path().join(&name);
        let mut command = Command::new("cc");
        command.args(SHARED);
        #[cfg(target_os = "macos")]
        command.arg(format!("-Wl,-install_name,@loader_path/{name}"));
        run(
            command
                .arg("-o")
                .arg(&dependency)
                .args(["-x", "c", "/dev/null"]),
            &directory.path().join("dependency.log"),
        );
        let search = format!("-L{}", directory.path().display());
        let link = format!("-l{DEPENDENCY}");
        self.stand_in(version, version, &[KEEP_NEEDED, &[&search, &link]].concat())
    }

    /// Installs a stand-in built for another architecture than this
    /// process's: an x86-64 one whose ELF header names AArch64 on Linux, or
    /// a thin library of the other Mac architecture.
    fn stand_in_for_another_architecture(&self, version: &str) -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            const AARCH64: u16 = 183;
            let library = self.stand_in(version, version, &[]);
            let mut bytes = fs::read(&library).unwrap();
            bytes[18..20].copy_from_slice(&AARCH64.to_le_bytes());
            fs::write(&library, bytes).unwrap();
            library
        }
        #[cfg(target_os = "macos")]
        {
            let other = if cfg!(target_arch = "aarch64") {
                "x86_64"
            } else {
                "arm64"
            };
            self.stand_in(version, version, &["-arch", other])
        }
    }

    fn load(&self, version: &str) -> Result<&'static Library, LoadError> {
        Loader::new(self.path()).load(&self::version(version))
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

/// LIB-02 Library and Fault (missing library; no substituted release): a
/// version folder without its library, and a version without a folder, are
/// refused as missing libraries naming the requested file. The valid library
/// of the version `current` names is never used instead: it is not even
/// loaded.
#[test]
fn missing_library_is_refused_without_substituting_another_release() {
    let home = Home::new();
    let current = home.stand_in("0.1.0", "0.1.0", &[]);
    fs::write(home.path().join("current"), "0.1.0\n").unwrap();
    fs::create_dir(home.path().join("0.2.0")).unwrap();
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

/// LIB-01 Library and Fault (same-release instances share one load), with the
/// exact stand-in: two loads of one version through one loader return the
/// same library, and another version's library is a separate load. Each
/// library reports the requirements its description declares and the
/// complete function table of its own image, and stays loaded.
#[test]
fn two_loads_of_one_version_share_one_library() {
    let home = Home::new();
    let first_path = home.stand_in("0.1.0", "0.1.0", &[]);
    let other_path = home.stand_in("0.2.0", "0.2.0", &[]);
    let loader = Loader::new(home.path());
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
/// answers revision 1 with success but no description, or with another
/// status such as `CM1_INTERNAL_ERROR`, breaks the bootstrap contract and is
/// refused as a malformed description. Each opened handle stays loaded.
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
            "-DBOOTSTRAP_STATUS=11",
            Failure::MalformedDescription,
            "status 11 and no description",
        ),
    ] {
        let library = home.stand_in(version, version, &[option]);
        let error = home.refusal(version);
        assert_eq!(error.failure(), failure, "{option}: {error}");
        assert!(error.to_string().contains(diagnosis), "{option}: {error}");
        assert!(loaded(&library), "{option}");
    }
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
/// another revision or a nonzero reserved field; missing requirements (no
/// instance allocation, owner stack or instance descriptor, workers without
/// stacks) or a nonzero reserved requirement; an incomplete function table.
/// Each opened handle stays loaded. Workers are optional: a release that
/// needs none needs no worker stack, and loads.
#[test]
fn malformed_description_is_refused_before_any_instance() {
    let home = Home::new();
    for (version, option, diagnosis) in [
        ("0.1.0", "-DREVISION=2", "revision 2"),
        ("0.2.0", "-DRESERVED=1", "reserved field 1"),
        ("0.3.0", "-DINSTANCE_BYTES=0", "requirements"),
        ("0.4.0", "-DOWNER_STACK_BYTES=0", "requirements"),
        ("0.5.0", "-DINSTANCE_DESCRIPTORS=0", "requirements"),
        ("0.6.0", "-DWORKER_STACK_BYTES=0", "requirements"),
        ("0.7.0", "-DREQUIREMENTS_RESERVED=1", "requirements"),
        ("0.8.0", "-DINCOMPLETE_TABLE", "function table"),
    ] {
        let library = home.stand_in(version, version, &[option]);
        let error = home.refusal(version);
        assert_eq!(
            error.failure(),
            Failure::MalformedDescription,
            "{option}: {error}"
        );
        assert!(error.to_string().contains(diagnosis), "{option}: {error}");
        assert!(loaded(&library), "{option}");
    }
    home.stand_in("0.9.0", "0.9.0", &["-DWORKERS=0", "-DWORKER_STACK_BYTES=0"]);
    home.load("0.9.0").unwrap();
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
    let text = home.library("0.3.0");
    fs::create_dir_all(text.parent().unwrap()).unwrap();
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

/// Absolute lookup (library embedding): the loader opens libraries by
/// absolute path only, so a relative executable home, which the dynamic
/// loader would resolve against the working directory, is refused at once.
#[test]
#[should_panic(expected = "must be absolute")]
fn loader_refuses_a_relative_executable_home() {
    Loader::new("executable home");
}
