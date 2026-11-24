//! The pinned ConPTY engine: `conpty.dll` and `OpenConsole.exe` from the
//! `Microsoft.Windows.Console.ConPTY` package named in `pins/conpty.txt`, the
//! same version and hashes the product ships (ADR 0003).
//!
//! The harness fetches the package by its pinned version, checks the
//! package's SHA-256, takes the two x64 files out and checks theirs, and
//! keeps them in a cache folder named after the version. A cached file is
//! checked again before every use; a download or a file whose hash differs
//! fails. `conpty.dll` is loaded by full path, as the daemon loads it, and
//! starts `OpenConsole.exe` from beside itself.

use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsStr,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
use windows_sys::Win32::{
    Foundation::HANDLE,
    System::{
        Console::COORD,
        LibraryLoader::{
            GetModuleFileNameW, GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR,
            LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
        },
    },
};

/// The checked-in pin, shared with the product's build.
const PIN: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../pins/conpty.txt"
));
/// Where nuget.org serves a package version.
const FEED: &str = "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty";
/// The x64 files inside the package.
const DLL_ENTRY: &str = "runtimes/win-x64/native/conpty.dll";
const CONSOLE_ENTRY: &str = "build/native/runtimes/x64/OpenConsole.exe";
/// Overrides the folder that holds one subfolder per pinned version.
const CACHE_VARIABLE: &str = "CLIMON2_TEST_CONPTY_CACHE";

/// A pinned package version and the SHA-256 of the package and of each file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pin {
    pub(crate) version: String,
    package: String,
    dll: String,
    console: String,
}

impl Pin {
    /// Parses `pins/conpty.txt`. Every name must appear once, each hash must
    /// be 64 lowercase hex digits and the version must be a stable release.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let mut values = [None, None, None, None];
        const NAMES: [&str; 4] = ["version", "package", "conpty.dll", "OpenConsole.exe"];
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line
                .split_once(' ')
                .ok_or_else(|| format!("pin line without a value: {line:?}"))?;
            let slot = NAMES
                .iter()
                .position(|known| *known == name)
                .ok_or_else(|| format!("unknown pin name {name:?}"))?;
            if values[slot].replace(value.trim().to_owned()).is_some() {
                return Err(format!("pin name {name:?} appears twice"));
            }
        }
        let [version, package, dll, console] = values;
        let take = |value: Option<String>, name: &str| {
            value.ok_or_else(|| format!("pin name {name:?} missing"))
        };
        let version = take(version, NAMES[0])?;
        if version.contains('-') || version.split('.').count() != 3 {
            return Err(format!(
                "pinned ConPTY version {version:?} is not a stable release"
            ));
        }
        let hash = |value: Option<String>, name: &str| {
            let value = take(value, name)?;
            if value.len() == 64
                && value
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
            {
                Ok(value)
            } else {
                Err(format!(
                    "pinned SHA-256 of {name} is not 64 lowercase hex digits"
                ))
            }
        };
        Ok(Self {
            version,
            package: hash(package, NAMES[1])?,
            dll: hash(dll, NAMES[2])?,
            console: hash(console, NAMES[3])?,
        })
    }

    fn url(&self) -> String {
        let version = &self.version;
        format!("{FEED}/{version}/microsoft.windows.console.conpty.{version}.nupkg")
    }

    /// The two files and their pinned hashes, as (entry in the package, file name, hash).
    fn files(&self) -> [(&'static str, &'static str, &str); 2] {
        [
            (DLL_ENTRY, "conpty.dll", &self.dll),
            (CONSOLE_ENTRY, "OpenConsole.exe", &self.console),
        ]
    }
}

/// Fails unless the file at `path` has the SHA-256 `expected`.
pub(crate) fn verify(path: &Path, expected: &str) -> io::Result<()> {
    let actual = format!("{:x}", Sha256::digest(fs::read(path)?));
    if actual == expected {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "SHA-256 mismatch for {}: pinned {expected}, found {actual}",
            path.display()
        ),
    ))
}

/// The version folder under `root` holding both pinned files, each verified.
/// When the folder is missing or a file in it differs, `fetch` downloads the
/// package to the given path, which is verified, unpacked and verified again
/// before the folder is replaced.
pub(crate) fn provide(
    pin: &Pin,
    root: &Path,
    fetch: impl FnOnce(&str, &Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let folder = root.join(&pin.version);
    if verified(pin, &folder).is_ok() {
        return Ok(folder);
    }
    fs::create_dir_all(root)?;
    let staging = tempfile::tempdir_in(root)?;
    let package = staging.path().join("package.nupkg");
    fetch(&pin.url(), &package)?;
    verify(&package, &pin.package)?;
    let unpacked = staging.path().join("unpacked");
    fs::create_dir(&unpacked)?;
    let entries = pin.files().map(|(entry, _, _)| entry);
    run(Command::new(system_tool("tar.exe"))
        .arg("-xf")
        .arg(&package)
        .arg("-C")
        .arg(&unpacked)
        .args(entries))?;
    let ready = staging.path().join("ready");
    fs::create_dir(&ready)?;
    for (entry, name, _) in pin.files() {
        fs::rename(unpacked.join(entry), ready.join(name))?;
    }
    verified(pin, &ready)?;
    // Another process may have provided the folder meanwhile, or left a bad one.
    if folder.exists() && verified(pin, &folder).is_err() {
        fs::remove_dir_all(&folder)?;
    }
    if let Err(error) = fs::rename(&ready, &folder) {
        verified(pin, &folder).map_err(|_| error)?;
    }
    Ok(folder)
}

fn verified(pin: &Pin, folder: &Path) -> io::Result<()> {
    pin.files()
        .into_iter()
        .try_for_each(|(_, name, hash)| verify(&folder.join(name), hash))
}

/// Downloads `url` to `to` with the system's `curl.exe`, failing on any HTTP error.
pub(crate) fn download(url: &str, to: &Path) -> io::Result<()> {
    run(Command::new(system_tool("curl.exe"))
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--retry", "3", "--output"])
        .arg(to)
        .arg(url))
}

/// A tool Windows ships in System32, by absolute path.
pub(crate) fn system_tool(name: &str) -> PathBuf {
    let root = env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into());
    Path::new(&root).join("System32").join(name)
}

pub(crate) fn run(command: &mut Command) -> io::Result<()> {
    let output = command.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{command:?} failed with {}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    )))
}

/// The cache root: `CLIMON2_TEST_CONPTY_CACHE`, else `conpty` beside the
/// profile folder of the running test (`target/debug/deps/…` → `target/conpty`).
fn cache_root() -> PathBuf {
    if let Some(root) = env::var_os(CACHE_VARIABLE) {
        return PathBuf::from(root);
    }
    let executable = env::current_exe().expect("locate test executable");
    executable
        .ancestors()
        .nth(3)
        .expect("test executable under target/<profile>/deps")
        .join("conpty")
}

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut isize) -> i32;
type ResizeFn = unsafe extern "system" fn(isize, COORD) -> i32;
type ReleaseFn = unsafe extern "system" fn(isize) -> i32;
type CloseFn = unsafe extern "system" fn(isize);
/// What `GetProcAddress` returns for any export.
type Export = unsafe extern "system" fn() -> isize;

/// The loaded pinned ConPTY: its folder and its pseudoconsole calls. It is
/// loaded once per process and never freed.
pub struct ConPty {
    folder: PathBuf,
    version: String,
    create: CreateFn,
    resize: ResizeFn,
    release: ReleaseFn,
    close: CloseFn,
}

impl ConPty {
    /// The pinned ConPTY, fetched, verified and loaded on first use.
    pub fn pinned() -> io::Result<&'static Self> {
        static PINNED: OnceLock<Result<ConPty, String>> = OnceLock::new();
        PINNED
            .get_or_init(|| {
                let pin = Pin::parse(PIN)?;
                let folder =
                    provide(&pin, &cache_root(), download).map_err(|error| error.to_string())?;
                Self::load(folder, pin.version).map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|error| io::Error::other(format!("pinned ConPTY unavailable: {error}")))
    }

    /// The version folder under `root` holding the pinned `conpty.dll` and
    /// `OpenConsole.exe`, each verified, fetched as for [`ConPty::pinned`]
    /// but not loaded: the files a Windows release ships.
    pub fn provide_files(root: &Path) -> io::Result<PathBuf> {
        provide(&Pin::parse(PIN).map_err(io::Error::other)?, root, download)
    }

    /// The folder holding the verified `conpty.dll` and `OpenConsole.exe`.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// The pinned package version.
    pub fn version(&self) -> &str {
        &self.version
    }

    fn load(folder: PathBuf, version: String) -> io::Result<Self> {
        let path = folder.join("conpty.dll");
        let wide = wide(path.as_os_str());
        // The path is NUL-terminated; the search flags keep the DLL's own
        // imports to its folder and System32.
        let module = unsafe {
            LoadLibraryExW(
                wide.as_ptr(),
                std::ptr::null_mut(),
                LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
            )
        };
        if module.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut loaded = vec![0u16; 32_768];
        // The buffer's length is passed; the call writes at most that many units.
        let length =
            unsafe { GetModuleFileNameW(module, loaded.as_mut_ptr(), loaded.len() as u32) };
        let loaded = PathBuf::from(String::from_utf16_lossy(&loaded[..length as usize]));
        if !same_path(&loaded, &path) {
            return Err(io::Error::other(format!(
                "loaded {} instead of {}",
                loaded.display(),
                path.display()
            )));
        }
        let export = |name: &[u8]| {
            // `name` is a NUL-terminated export name of the module just loaded.
            unsafe { GetProcAddress(module, name.as_ptr()) }.ok_or_else(|| {
                io::Error::other(format!(
                    "conpty.dll lacks {}",
                    String::from_utf8_lossy(&name[..name.len() - 1])
                ))
            })
        };
        // Each export has the documented signature of its type.
        unsafe {
            Ok(Self {
                create: std::mem::transmute::<Export, CreateFn>(export(
                    b"ConptyCreatePseudoConsole\0",
                )?),
                resize: std::mem::transmute::<Export, ResizeFn>(export(
                    b"ConptyResizePseudoConsole\0",
                )?),
                release: std::mem::transmute::<Export, ReleaseFn>(export(
                    b"ConptyReleasePseudoConsole\0",
                )?),
                close: std::mem::transmute::<Export, CloseFn>(export(
                    b"ConptyClosePseudoConsole\0",
                )?),
                folder,
                version,
            })
        }
    }

    /// `ConptyCreatePseudoConsole` reading `input` and writing `output`.
    pub(crate) fn create(
        &self,
        rows: u16,
        cols: u16,
        input: HANDLE,
        output: HANDLE,
        flags: u32,
    ) -> io::Result<isize> {
        let mut console = 0;
        // Both handles are open pipe ends; the call writes one HPCON.
        hresult(unsafe { (self.create)(size(rows, cols), input, output, flags, &mut console) })?;
        Ok(console)
    }

    pub(crate) fn resize(&self, console: isize, rows: u16, cols: u16) -> io::Result<()> {
        // `console` is open: only `close` ends it.
        hresult(unsafe { (self.resize)(console, size(rows, cols)) })
    }

    /// `ConptyReleasePseudoConsole`: the console ends once its last process leaves.
    pub(crate) fn release(&self, console: isize) -> io::Result<()> {
        // As in `resize`.
        hresult(unsafe { (self.release)(console) })
    }

    /// `ConptyClosePseudoConsole`; `console` must not be used again.
    pub(crate) fn close(&self, console: isize) {
        // As in `resize`; the caller drops `console` after this.
        unsafe { (self.close)(console) }
    }
}

fn size(rows: u16, cols: u16) -> COORD {
    COORD {
        X: cols as i16,
        Y: rows as i16,
    }
}

fn hresult(result: i32) -> io::Result<()> {
    if result >= 0 {
        return Ok(());
    }
    Err(io::Error::other(format!("HRESULT {:#010x}", result as u32)))
}

fn same_path(left: &Path, right: &Path) -> bool {
    let normal = |path: &Path| {
        fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_owned())
            .to_string_lossy()
            .to_lowercase()
    };
    normal(left) == normal(right)
}

/// `text` as a NUL-terminated UTF-16 string.
pub(crate) fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DLL: &str = "e971856f89748dc3b458125e9afdd79b5e9574a7c937676116d3cbbd3b71f492";

    fn pin_text(version: &str) -> String {
        format!(
            "# comment\nversion {version}\npackage {DLL}\nconpty.dll {DLL}\nOpenConsole.exe {DLL}\n"
        )
    }

    #[test]
    fn the_checked_in_pin_names_a_stable_release() {
        let pin = Pin::parse(PIN).unwrap();
        assert_eq!(pin.version, "1.25.261109001");
        assert_eq!(pin.dll, DLL);
        assert_eq!(
            pin.url(),
            "https://api.nuget.org/v3-flatcontainer/microsoft.windows.console.conpty/\
             1.25.261109001/microsoft.windows.console.conpty.1.25.261109001.nupkg"
        );
    }

    #[test]
    fn a_preview_version_or_a_malformed_hash_is_refused() {
        assert!(Pin::parse(&pin_text("1.26.261109003-preview")).is_err());
        assert!(Pin::parse(&pin_text("1.25")).is_err());
        let upper = pin_text("1.25.261109001").replace("package e9", "package E9");
        assert!(Pin::parse(&upper).is_err());
        let missing = pin_text("1.25.261109001").replace("OpenConsole.exe", "#");
        assert!(Pin::parse(&missing).is_err());
        let twice = pin_text("1.25.261109001") + &format!("package {DLL}\n");
        assert!(Pin::parse(&twice).is_err());
    }

    #[test]
    fn a_downloaded_package_with_another_hash_fails_and_leaves_no_folder() {
        let pin = Pin::parse(PIN).unwrap();
        let root = tempfile::tempdir().unwrap();
        let error = provide(&pin, root.path(), |url, to| {
            assert_eq!(url, pin.url());
            fs::write(to, b"not the pinned package")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("SHA-256 mismatch"), "{error}");
        assert!(!root.path().join(&pin.version).exists());
    }

    #[test]
    fn a_cached_file_with_another_hash_is_fetched_again() {
        let pinned = ConPty::pinned().unwrap();
        let pin = Pin::parse(PIN).unwrap();
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join(&pin.version);
        fs::create_dir(&folder).unwrap();
        for name in ["conpty.dll", "OpenConsole.exe"] {
            fs::copy(pinned.folder().join(name), folder.join(name)).unwrap();
        }
        assert_eq!(
            provide(&pin, root.path(), |_, _| unreachable!()).unwrap(),
            folder
        );
        fs::write(folder.join("OpenConsole.exe"), b"tampered").unwrap();
        let fetched = std::cell::Cell::new(false);
        let error = provide(&pin, root.path(), |_, to| {
            fetched.set(true);
            fs::write(to, b"still not the package")
        })
        .unwrap_err();
        assert!(fetched.get());
        assert!(error.to_string().contains("SHA-256 mismatch"), "{error}");
    }

    #[test]
    fn the_pinned_files_load_from_their_verified_folder() {
        let conpty = ConPty::pinned().unwrap();
        assert_eq!(conpty.version(), "1.25.261109001");
        verify(&conpty.folder().join("conpty.dll"), DLL).unwrap();
        assert!(conpty.folder().join("OpenConsole.exe").is_file());
    }
}
