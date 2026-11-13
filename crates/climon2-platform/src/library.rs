//! The one validated loader for client libraries (library embedding, #84).
//!
//! The monitor and the client executable load a session's exact-release
//! client library through [`Loader`]: `<executable home>/<version>/<library
//! file>`, by absolute path, with eager and local symbol resolution. It never
//! falls back to `current`, another version or a search path. The library's
//! revision-1 description is validated before anything can use it, so every
//! refusal is an explicit [`LoadError`] before any instance exists.
//!
//! One load is shared by every user of a version. Nothing is ever unloaded:
//! every handle the dynamic loader opens, including one later refused, stays
//! loaded until the process exits. A refusal is not remembered, so a later
//! load tries again and a repaired installation can succeed.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("client library loading requires macOS or Linux");

#[cfg(target_os = "linux")]
#[path = "library/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "library/macos.rs"]
mod platform;

use climon2_core::{
    VersionName,
    layout::client_library_name,
    native::{
        self, Bootstrap, Call, Communication, Control, Created, Delivery, Description, Drained,
        Instance, Interface, Metrics, Request, Requirements, RunReport, Startup,
    },
};
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString, c_void},
    fmt,
    fs::File,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr,
    sync::{Mutex, PoisonError},
};

/// Why a client library cannot be used: an explicit availability error,
/// never evidence that a session ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Failure {
    /// The library file cannot be opened.
    MissingLibrary,
    /// The file is a library for this process's architecture, but the
    /// dynamic loader cannot load it, most likely for want of a library it
    /// needs.
    MissingDependency,
    /// The file is not a library for this process's architecture.
    IncompatibleArchitecture,
    /// macOS: dyld loaded another file of the same name, found through
    /// `DYLD_LIBRARY_PATH`, instead of the one requested.
    SubstitutedImage,
    /// The library does not export `climon2_client_get_interface`.
    MissingSymbol,
    /// The library's bootstrap refuses revision 1.
    UnsupportedRevision,
    /// The library describes another release than the version requested.
    WrongRelease,
    /// The revision-1 description breaks the native contract.
    MalformedDescription,
}

/// A refused client library: which file, why, and the loader's diagnosis.
#[derive(Debug)]
pub struct LoadError {
    library: PathBuf,
    failure: Failure,
    detail: String,
}

impl LoadError {
    /// The library file the loader was asked for.
    pub fn library(&self) -> &Path {
        &self.library
    }

    pub fn failure(&self) -> Failure {
        self.failure
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot load the client library {}: {}",
            self.library.display(),
            self.detail
        )
    }
}

impl std::error::Error for LoadError {}

/// A refusal before its library path is attached.
type Refusal = (Failure, String);

/// Every revision-1 call of a validated library; none is missing.
#[derive(Clone, Copy)]
pub struct Functions {
    pub create: unsafe extern "C" fn(*const Startup, *mut Created) -> Call,
    pub run: unsafe extern "C" fn(*mut Instance, *mut RunReport) -> Call,
    pub destroy: unsafe extern "C" fn(*mut Instance) -> Call,
    pub submit: unsafe extern "C" fn(*mut Communication, *const Request) -> Call,
    pub drain: unsafe extern "C" fn(*mut Communication, *mut Drained) -> Call,
    pub release_delivery: unsafe extern "C" fn(*mut Delivery) -> Call,
    pub shutdown: unsafe extern "C" fn(*mut Control) -> Call,
    pub release_communication: unsafe extern "C" fn(*mut Communication) -> Call,
    pub release_control: unsafe extern "C" fn(*mut Control) -> Call,
    pub notification: unsafe extern "C" fn(*mut Communication, *mut i32) -> Call,
    pub snapshot: unsafe extern "C" fn(*mut Communication, *mut Metrics) -> Call,
}

/// A loaded client library whose revision-1 description was validated. It
/// exists only through [`Loader::load`] and stays loaded until process exit.
pub struct Library {
    bootstrap: Bootstrap,
    requirements: Requirements,
    functions: Functions,
}

impl Library {
    /// The qualified resource requirements the release declares.
    pub fn requirements(&self) -> &Requirements {
        &self.requirements
    }

    /// The release's revision-1 calls.
    pub fn functions(&self) -> &Functions {
        &self.functions
    }

    /// The library's frozen bootstrap, for a caller that observes the C ABI
    /// itself, such as the Library-layer test host. Embedders use
    /// [`Library::functions`].
    pub fn bootstrap(&self) -> Bootstrap {
        self.bootstrap
    }
}

/// Loads the client libraries of one climon2 executable home, once per
/// version.
pub struct Loader {
    home: PathBuf,
    libraries: Mutex<BTreeMap<VersionName, &'static Library>>,
}

impl Loader {
    /// The loader of `executable_home`'s libraries, which must be absolute
    /// and canonical, like [`climon2_core::layout::ExecutableLayout::root`]:
    /// dyld names a loaded image by the path it opened.
    ///
    /// # Panics
    ///
    /// When `executable_home` is relative: the dynamic loader would resolve
    /// it against the working directory.
    pub fn new(executable_home: impl Into<PathBuf>) -> Self {
        let home = executable_home.into();
        assert!(
            home.is_absolute(),
            "the climon2 executable home must be absolute: {}",
            home.display()
        );
        Self {
            home,
            libraries: Mutex::default(),
        }
    }

    /// `version`'s library: the shared load if there is one, otherwise a new
    /// load, validated before anything can use it.
    pub fn load(&self, version: &VersionName) -> Result<&'static Library, LoadError> {
        if let Some(library) = self.shared(version) {
            return Ok(library);
        }
        let path = self.home.join(version.as_str()).join(client_library_name());
        let library = open(&path, version).map_err(|(failure, detail)| LoadError {
            library: path,
            failure,
            detail,
        })?;
        Ok(self.share(version, library))
    }

    fn shared(&self, version: &VersionName) -> Option<&'static Library> {
        let libraries = self
            .libraries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        libraries.get(version).copied()
    }

    /// The one shared load of `version`. Two first loads racing on different
    /// threads open the same handle, and the first to finish is shared.
    fn share(&self, version: &VersionName, library: Library) -> &'static Library {
        let mut libraries = self
            .libraries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        libraries
            .entry(version.clone())
            .or_insert_with(|| Box::leak(Box::new(library)))
    }
}

/// Opens `path` (never closed) and validates that its revision-1 description
/// describes `release`.
fn open(path: &Path, release: &VersionName) -> Result<Library, Refusal> {
    let bootstrap = bootstrap(path)?;
    validate(bootstrap, describe(bootstrap)?, release)
}

/// The bootstrap of the library at `path`, opened with eager and local
/// symbol resolution and never closed.
fn bootstrap(path: &Path) -> Result<Bootstrap, Refusal> {
    let name = CString::new(path.as_os_str().as_bytes()).expect("a path without NUL");
    // SAFETY: a NUL-terminated path; the handle is deliberately never closed.
    let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return Err(unloadable(path, dlerror()));
    }
    // SAFETY: a live handle and a NUL-terminated symbol name.
    let symbol = unsafe { libc::dlsym(handle, native::BOOTSTRAP.as_ptr()) };
    if symbol.is_null() {
        return Err((
            Failure::MissingSymbol,
            format!("it does not export {}", native::BOOTSTRAP.to_string_lossy()),
        ));
    }
    if let Some(loaded) = platform::substitute(symbol, &name) {
        return Err((
            Failure::SubstitutedImage,
            format!("the dynamic loader loaded {loaded} instead, found through a search path"),
        ));
    }
    // SAFETY: the frozen bootstrap symbol has the `Bootstrap` signature.
    Ok(unsafe { std::mem::transmute::<*mut c_void, Bootstrap>(symbol) })
}

/// The dynamic loader's message about its last failure, for diagnosis only.
fn dlerror() -> String {
    // SAFETY: dlerror returns null or a NUL-terminated message it owns.
    let message = unsafe { libc::dlerror() };
    if message.is_null() {
        return String::new();
    }
    // SAFETY: the message is nonnull and NUL-terminated.
    unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned()
}

/// Why the dynamic loader could not load `path`, decided by the loader's own
/// checks of the file it was given; the dynamic loader's `message` is only
/// quoted.
fn unloadable(path: &Path, message: String) -> Refusal {
    match File::open(path) {
        Err(error) => (
            Failure::MissingLibrary,
            format!("it cannot be opened: {error}"),
        ),
        Ok(file) if platform::runs_here(&file) => (
            Failure::MissingDependency,
            format!("the dynamic loader cannot load it or a library it needs: {message}"),
        ),
        Ok(_) => (
            Failure::IncompatibleArchitecture,
            format!(
                "it is not {}; the dynamic loader said: {message}",
                platform::ARCHITECTURE
            ),
        ),
    }
}

/// The library `bootstrap` and its `description` make, if that describes
/// `release` and keeps the native contract.
fn validate(
    bootstrap: Bootstrap,
    description: &Description,
    release: &VersionName,
) -> Result<Library, Refusal> {
    shape(description)?;
    identity(description.release, release)?;
    Ok(Library {
        bootstrap,
        requirements: requirements(description.requirements)?,
        functions: functions(&description.functions)?,
    })
}

/// Revision 1's description, as the bootstrap contract allows it to answer.
fn describe(bootstrap: Bootstrap) -> Result<&'static Description, Refusal> {
    let mut description: *const Description = ptr::null();
    // SAFETY: the output is writable storage for one pointer.
    let status = unsafe { bootstrap(native::REVISION, &mut description) };
    // SAFETY: a description the bootstrap returns is immutable until exit.
    match (status, unsafe { description.as_ref() }) {
        (native::SUCCESS, Some(description)) => Ok(description),
        (native::UNSUPPORTED_REVISION, _) => Err((
            Failure::UnsupportedRevision,
            "it does not support revision 1 of the native client interface".into(),
        )),
        (status, description) => Err((
            Failure::MalformedDescription,
            format!(
                "its bootstrap answered revision 1 with status {status} and {}",
                description.map_or("no description", |_| "a description")
            ),
        )),
    }
}

/// Whether the description is a revision-1 one with its reserved field zero.
fn shape(description: &Description) -> Result<(), Refusal> {
    if description.revision != native::REVISION || description.reserved != 0 {
        return Err((
            Failure::MalformedDescription,
            format!(
                "it describes revision {} with reserved field {}",
                description.revision, description.reserved
            ),
        ));
    }
    Ok(())
}

/// The longest release text that can name a version folder: the longest file
/// name.
const RELEASE_BYTES: u64 = 255;

/// Whether the release text names exactly `release`.
fn identity(text: native::Bytes, release: &VersionName) -> Result<(), Refusal> {
    if text.data.is_null() || text.length > RELEASE_BYTES {
        return Err((
            Failure::MalformedDescription,
            format!(
                "its release text of {} bytes at {:p} cannot name a version folder",
                text.length, text.data
            ),
        ));
    }
    // SAFETY: a nonnull release text covers `length` readable bytes, at most 255.
    let reported = unsafe { std::slice::from_raw_parts(text.data, text.length as usize) };
    if reported != release.as_str().as_bytes() {
        return Err((
            Failure::WrongRelease,
            format!(
                "it describes release {:?}, not {release}",
                String::from_utf8_lossy(reported)
            ),
        ));
    }
    Ok(())
}

/// The requirements, if present: every instance needs local allocation, an
/// owner stack and an instance descriptor, and every worker a stack.
fn requirements(requirements: Requirements) -> Result<Requirements, Refusal> {
    let Requirements {
        instance_bytes,
        owner_stack_bytes,
        worker_stack_bytes,
        workers,
        instance_descriptors,
        reserved,
        ..
    } = requirements;
    let unstacked_workers = workers != 0 && worker_stack_bytes == 0;
    let needed = [
        instance_bytes,
        owner_stack_bytes,
        u64::from(instance_descriptors),
    ];
    if needed.contains(&0) || unstacked_workers || reserved != 0 {
        return Err((
            Failure::MalformedDescription,
            "its requirements are missing or reserved: every instance needs local \
             allocation, an owner stack and a descriptor, every worker a stack, \
             and the reserved field is zero"
                .into(),
        ));
    }
    Ok(requirements)
}

/// The table, if every entry is present.
fn functions(table: &Interface) -> Result<Functions, Refusal> {
    let Interface {
        create: Some(create),
        run: Some(run),
        destroy: Some(destroy),
        submit: Some(submit),
        drain: Some(drain),
        release_delivery: Some(release_delivery),
        shutdown: Some(shutdown),
        release_communication: Some(release_communication),
        release_control: Some(release_control),
        notification: Some(notification),
        snapshot: Some(snapshot),
    } = *table
    else {
        return Err((
            Failure::MalformedDescription,
            "its function table is incomplete".into(),
        ));
    };
    Ok(Functions {
        create,
        run,
        destroy,
        submit,
        drain,
        release_delivery,
        shutdown,
        release_communication,
        release_control,
        notification,
        snapshot,
    })
}
