//! #318: the client library on Windows. `climon2_client.dll` links the C
//! runtime statically, exports only the bootstrap and imports only system
//! DLLs. The shared loader opens it by exact path with
//! `LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32`, never
//! frees it, and classifies every refusal from the loader's own results
//! (library embedding, "Windows").
#![cfg(windows)]
use climon2_core::{VersionName, layout::ExecutableHome, native};
use climon2_platform::library::{Failure, LoadError, Loader};
use std::{
    fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows_sys::Win32::{Foundation::HMODULE, System::LibraryLoader::GetModuleHandleW};

/// The workspace's own build of the library, beside this test.
fn built() -> PathBuf {
    let path = std::env::current_exe()
        .unwrap()
        .with_file_name("climon2_client.dll");
    assert!(path.is_file(), "missing {}", path.display());
    path
}

/// A little-endian integer of `N` bytes at `offset`.
fn le<const N: usize>(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0u8; 8];
    value[..N].copy_from_slice(&bytes[offset..offset + N]);
    u64::from_le_bytes(value)
}

/// What a PE32+ image exports and imports, read from its file.
struct Pe {
    bytes: Vec<u8>,
    /// Each section's virtual address, virtual size and file offset.
    sections: Vec<(u64, u64, u64)>,
    /// Where the optional header's data directories start.
    directories: usize,
}

impl Pe {
    fn read(path: &Path) -> Self {
        let bytes = fs::read(path).unwrap();
        let header = le::<4>(&bytes, 0x3C) as usize;
        assert_eq!(&bytes[header..header + 4], b"PE\0\0");
        let coff = header + 4;
        let count = le::<2>(&bytes, coff + 2) as usize;
        let optional = coff + 20;
        assert_eq!(le::<2>(&bytes, optional), 0x20B, "not PE32+");
        let table = optional + le::<2>(&bytes, coff + 16) as usize;
        let sections = (0..count)
            .map(|index| table + 40 * index)
            .map(|entry| {
                (
                    le::<4>(&bytes, entry + 12),
                    le::<4>(&bytes, entry + 8),
                    le::<4>(&bytes, entry + 20),
                )
            })
            .collect();
        Self {
            bytes,
            sections,
            directories: optional + 112,
        }
    }

    /// The machine the COFF header names.
    fn machine(&self) -> u16 {
        le::<2>(&self.bytes, le::<4>(&self.bytes, 0x3C) as usize + 4) as u16
    }

    fn offset(&self, rva: u64) -> usize {
        let (start, _, file) = self
            .sections
            .iter()
            .find(|(start, size, _)| (*start..start + size).contains(&rva))
            .unwrap_or_else(|| panic!("no section holds RVA {rva:#x}"));
        (rva - start + file) as usize
    }

    /// Data directory `index`'s RVA, if it has one.
    fn directory(&self, index: usize) -> Option<u64> {
        Some(le::<4>(&self.bytes, self.directories + 8 * index)).filter(|rva| *rva != 0)
    }

    fn text(&self, rva: u64) -> String {
        let start = self.offset(rva);
        let end = start + self.bytes[start..].iter().position(|b| *b == 0).unwrap();
        String::from_utf8(self.bytes[start..end].to_vec()).unwrap()
    }

    fn exports(&self) -> Vec<String> {
        let Some(directory) = self.directory(0).map(|rva| self.offset(rva)) else {
            return Vec::new();
        };
        let names = le::<4>(&self.bytes, directory + 24) as usize;
        let table = self.offset(le::<4>(&self.bytes, directory + 32));
        (0..names)
            .map(|index| self.text(le::<4>(&self.bytes, table + 4 * index)))
            .collect()
    }

    /// The DLLs it imports, directly or delay-loaded.
    fn imports(&self) -> Vec<String> {
        // Import descriptors are 20 bytes with the name at 12; delay-load
        // ones are 32 bytes with the name at 4. Each list ends in zeros.
        [(1, 20, 12), (13, 32, 4)]
            .into_iter()
            .filter_map(|(index, size, name)| {
                Some((self.offset(self.directory(index)?), size, name))
            })
            .flat_map(|(table, size, name)| {
                (0..)
                    .map(move |entry| le::<4>(&self.bytes, table + size * entry + name))
                    .take_while(|rva| *rva != 0)
                    .map(|rva| self.text(rva))
                    .collect::<Vec<_>>()
            })
            .collect()
    }
}

#[test]
fn the_library_exports_only_the_bootstrap() {
    let library = Pe::read(&built());
    assert_eq!(library.machine(), 0x8664);
    assert_eq!(library.exports(), [native::BOOTSTRAP.to_str().unwrap()]);
}

/// The static C runtime leaves no import of the Visual C++ runtime or the
/// Universal C runtime, so the library loads on a clean Windows 11 and each
/// loaded release keeps its own C runtime state.
#[test]
fn the_library_links_the_c_runtime_statically_and_imports_only_system_dlls() {
    let imports = Pe::read(&built()).imports();
    assert!(!imports.is_empty());
    let system = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32");
    for import in &imports {
        let name = import.to_ascii_lowercase();
        let runtime = ["vcruntime", "msvcp", "ucrtbase", "api-ms-win-crt-"];
        assert!(
            !runtime.iter().any(|prefix| name.starts_with(prefix)),
            "{imports:?}"
        );
        assert!(
            name.starts_with("api-ms-win-") || system.join(&name).is_file(),
            "{import} is not a system DLL: {imports:?}"
        );
    }
}

/// This release, which the library describes.
fn release() -> VersionName {
    env!("CARGO_PKG_VERSION").parse().unwrap()
}

/// A private climon2 executable home.
struct Home(tempfile::TempDir);

impl Home {
    fn new() -> Self {
        Self(
            tempfile::Builder::new()
                .prefix("loader-")
                .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
                .unwrap(),
        )
    }

    /// The canonical home, as the client executable resolves it.
    fn home(&self) -> ExecutableHome {
        ExecutableHome::new(&self.0.path().canonicalize().unwrap()).unwrap()
    }

    /// This release's library path, with its version folder created.
    fn library(&self) -> PathBuf {
        let library = self.home().client_library(&release());
        fs::create_dir_all(library.parent().unwrap()).unwrap();
        library
    }

    /// Installs `bytes` as this release's library.
    fn install(&self, bytes: &[u8]) -> PathBuf {
        let library = self.library();
        fs::write(&library, bytes).unwrap();
        library
    }

    fn load(&self) -> Result<(), LoadError> {
        Loader::new(self.home()).load(&release()).map(drop)
    }

    fn refusal(&self) -> LoadError {
        self.load().map(|()| "loaded").unwrap_err()
    }
}

/// The module loaded from exactly `path`, if any: a full path names only
/// the module loaded from that file.
fn module(path: &Path) -> Option<HMODULE> {
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a NUL-terminated path; the call only looks the module up.
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    (!module.is_null()).then_some(module)
}

/// The built library with every occurrence of `from` replaced by `to`, of
/// the same length.
fn patched(from: &str, to: &str) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let mut bytes = fs::read(built()).unwrap();
    let mut found = false;
    for start in 0..=bytes.len() - from.len() {
        if bytes[start..].starts_with(from.as_bytes()) {
            bytes[start..start + to.len()].copy_from_slice(to.as_bytes());
            found = true;
        }
    }
    assert!(found, "the library holds no {from:?}");
    bytes
}

/// A dependency the library imports by name, which the copy that
/// `missing_dependency` patches can no longer find under that name.
const DEPENDENCY: &str = "bcryptprimitives.dll";
const RENAMED: &str = "bcryptprimitivez.dll";

/// The library, importing `RENAMED` instead of `DEPENDENCY`.
fn missing_dependency() -> Vec<u8> {
    patched(DEPENDENCY, RENAMED)
}

#[test]
fn the_library_loads_by_its_exact_path_and_stays_loaded() {
    let home = Home::new();
    let library = home.install(&fs::read(built()).unwrap());
    home.load().unwrap();
    assert!(module(&library).is_some());
}

/// Two releases' libraries share a file name: Windows must load each from
/// its own file rather than reuse the first by name.
#[test]
fn two_libraries_of_one_name_load_as_two_modules() {
    let (first, second) = (Home::new(), Home::new());
    let paths = [&first, &second].map(|home| home.install(&fs::read(built()).unwrap()));
    first.load().unwrap();
    second.load().unwrap();
    let [one, two] = paths.map(|path| module(&path).unwrap());
    assert_ne!(one, two);
}

#[test]
fn a_missing_library_is_refused_as_missing() {
    let home = Home::new();
    let library = home.library();
    let error = home.refusal();
    assert_eq!(error.failure(), Failure::MissingLibrary, "{error}");
    assert_eq!(error.library(), library);
}

/// A 32-bit DLL, a file that is no image at all, an empty file and a
/// truncated x86-64 DLL are each another architecture's or none.
#[test]
fn a_file_that_is_no_x86_64_dll_is_refused_as_another_architecture() {
    let system = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("SysWOW64");
    let x86 = fs::read(system.join("version.dll")).unwrap();
    assert_eq!(le::<2>(&x86, le::<4>(&x86, 0x3C) as usize + 4), 0x14C);
    let truncated = fs::read(built()).unwrap()[..4096].to_vec();
    for (what, bytes) in [
        ("an x86 DLL", x86),
        ("text", b"not a library".to_vec()),
        ("an empty file", Vec::new()),
        ("a truncated x86-64 DLL", truncated),
    ] {
        let home = Home::new();
        home.install(&bytes);
        let error = home.refusal();
        assert_eq!(
            error.failure(),
            Failure::IncompatibleArchitecture,
            "{what}: {error}"
        );
    }
}

/// A dependency is found beside the library or in System32, never in the
/// working directory or on `PATH`.
/// The working directory and `PATH` a load would search if it searched them,
/// restored on drop so no directory stays in use. No other test of this
/// file reads either: each loads only files of its own home, by full path.
struct Searched {
    directory: PathBuf,
    path: std::ffi::OsString,
}

impl Searched {
    fn from(directory: &Path, path: &std::ffi::OsStr) -> Self {
        let searched = Self {
            directory: std::env::current_dir().unwrap(),
            path: std::env::var_os("PATH").unwrap(),
        };
        // SAFETY: as above, no other thread reads PATH meanwhile.
        unsafe { std::env::set_var("PATH", path) };
        std::env::set_current_dir(directory).unwrap();
        searched
    }
}

impl Drop for Searched {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.directory);
        // SAFETY: as above.
        unsafe { std::env::set_var("PATH", &self.path) };
    }
}

#[test]
fn a_dependency_resolves_only_beside_the_library_or_in_system32() {
    let system = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32");
    let missing = Home::new();
    let library = missing.install(&missing_dependency());
    let elsewhere = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    fs::copy(system.join(DEPENDENCY), elsewhere.path().join(RENAMED)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(elsewhere.path().to_owned())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let _restored = Searched::from(elsewhere.path(), &path);
    let error = missing.refusal();
    assert_eq!(error.failure(), Failure::MissingDependency, "{error}");
    assert!(module(&library).is_none());

    let beside = Home::new();
    let library = beside.install(&missing_dependency());
    fs::copy(system.join(DEPENDENCY), library.with_file_name(RENAMED)).unwrap();
    beside.load().unwrap();
}

/// A library without the bootstrap is refused, and stays loaded: nothing is
/// ever freed.
#[test]
fn a_library_without_the_bootstrap_is_refused_and_stays_loaded() {
    let home = Home::new();
    let library = home.install(&patched(
        "climon2_client_get_interface",
        "climon2_client_get_interfacx",
    ));
    let error = home.refusal();
    assert_eq!(error.failure(), Failure::MissingSymbol, "{error}");
    assert!(error.to_string().contains("climon2_client_get_interface"));
    assert!(module(&library).is_some());
}
