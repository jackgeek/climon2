//! The pinned full-screen apps (docs/for-humans/windows/qualification.md,
//! "Full-screen apps"): vim for Windows and Microsoft Edit, which the CI
//! runner lacks and whose `edit.exe` Windows 11 shadows with an older one.
//!
//! Each app's pin, `pins/vim.txt` or `pins/edit.txt`, names the release zip's
//! URL, the zip's SHA-256, the program inside it and the SHA-256 of each file
//! the harness relies on. The harness downloads the zip, checks its hash,
//! unpacks it and checks each pinned file, and keeps it in a cache folder
//! named after the zip's hash. Each pinned file is checked again before
//! every use, as the pinned ConPTY's are; a download or a file whose hash
//! differs fails.

use super::conpty::{download, run, system_tool, verify};
use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

/// Overrides the folder that holds one subfolder per pinned zip.
const CACHE_VARIABLE: &str = "CLIMON2_TEST_APP_CACHE";

/// A full-screen app the qualification runs in a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum App {
    /// vim for Windows, from the `vim-win32-installer` release zip.
    Vim,
    /// Microsoft Edit, from its release zip.
    Edit,
}

impl App {
    fn pin_text(self) -> &'static str {
        match self {
            Self::Vim => include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../pins/vim.txt")),
            Self::Edit => {
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../pins/edit.txt"))
            }
        }
    }

    /// The app's program by absolute path, fetched and verified on first
    /// use, and verified again on each later one.
    pub fn program(self) -> io::Result<PathBuf> {
        static VIM: OnceLock<Result<Provided, String>> = OnceLock::new();
        static EDIT: OnceLock<Result<Provided, String>> = OnceLock::new();
        let cell = match self {
            Self::Vim => &VIM,
            Self::Edit => &EDIT,
        };
        let provided = cell
            .get_or_init(|| {
                let pin = AppPin::parse(self.pin_text())?;
                let folder =
                    provide(&pin, &cache_root(), download).map_err(|error| error.to_string())?;
                Ok(Provided { pin, folder })
            })
            .as_ref()
            .map_err(|error| io::Error::other(format!("pinned {self:?} unavailable: {error}")))?;
        verified(&provided.pin, &provided.folder)?;
        Ok(provided.folder.join(&provided.pin.program))
    }
}

/// A verified cache folder and the pin it holds.
struct Provided {
    pin: AppPin,
    folder: PathBuf,
}

/// An app's pin: where its zip is, the zip's SHA-256, the program in it and
/// the SHA-256 of each pinned file, the program among them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppPin {
    url: String,
    package: String,
    program: String,
    files: Vec<(String, String)>,
}

impl AppPin {
    /// Parses a pin. `url`, `package` and `program` appear once each, every
    /// `file` line names an entry and its hash, each hash is 64 lowercase hex
    /// digits, the URL is `https`, and the program is a pinned file.
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let (mut url, mut package, mut program, mut files) = (None, None, None, Vec::new());
        for line in text.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (name, value) = line
                .split_once(' ')
                .ok_or_else(|| format!("pin line without a value: {line:?}"))?;
            let value = value.trim().to_owned();
            let slot = match name {
                "url" => &mut url,
                "package" => &mut package,
                "program" => &mut program,
                "file" => {
                    let (entry, hash) = value
                        .rsplit_once(' ')
                        .ok_or_else(|| format!("file line without a hash: {line:?}"))?;
                    files.push((entry.trim().to_owned(), sha256(hash)?));
                    continue;
                }
                _ => return Err(format!("unknown pin name {name:?}")),
            };
            if slot.replace(value).is_some() {
                return Err(format!("pin name {name:?} appears twice"));
            }
        }
        let url = url.ok_or("pin name \"url\" missing")?;
        if !url.starts_with("https://") {
            return Err(format!("pinned URL {url:?} is not https"));
        }
        let program = program.ok_or("pin name \"program\" missing")?;
        if !files.iter().any(|(entry, _)| *entry == program) {
            return Err(format!("program {program:?} is not a pinned file"));
        }
        Ok(Self {
            url,
            package: sha256(&package.ok_or("pin name \"package\" missing")?)?,
            program,
            files,
        })
    }
}

/// `hash` if it is 64 lowercase hex digits.
fn sha256(hash: &str) -> Result<String, String> {
    if hash.len() == 64
        && hash
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(hash.to_owned())
    } else {
        Err(format!(
            "pinned SHA-256 {hash:?} is not 64 lowercase hex digits"
        ))
    }
}

/// The folder under `root` named after the zip's hash, holding the unpacked
/// zip with each pinned file verified. When the folder is missing or a file
/// in it differs, `fetch` downloads the zip to the given path, which is
/// verified, unpacked and verified again before the folder is replaced.
pub(crate) fn provide(
    pin: &AppPin,
    root: &Path,
    fetch: impl FnOnce(&str, &Path) -> io::Result<()>,
) -> io::Result<PathBuf> {
    let folder = root.join(&pin.package);
    if verified(pin, &folder).is_ok() {
        return Ok(folder);
    }
    fs::create_dir_all(root)?;
    let staging = tempfile::tempdir_in(root)?;
    let package = staging.path().join("package.zip");
    fetch(&pin.url, &package)?;
    verify(&package, &pin.package)?;
    let ready = staging.path().join("ready");
    fs::create_dir(&ready)?;
    run(Command::new(system_tool("tar.exe"))
        .arg("-xf")
        .arg(&package)
        .arg("-C")
        .arg(&ready))?;
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

fn verified(pin: &AppPin, folder: &Path) -> io::Result<()> {
    pin.files
        .iter()
        .try_for_each(|(entry, hash)| verify(&folder.join(entry), hash))
}

/// The cache root: `CLIMON2_TEST_APP_CACHE`, else `apps` beside the profile
/// folder of the running test (`target/debug/deps/…` → `target/apps`).
fn cache_root() -> PathBuf {
    if let Some(root) = env::var_os(CACHE_VARIABLE) {
        return PathBuf::from(root);
    }
    let executable = env::current_exe().expect("locate test executable");
    executable
        .ancestors()
        .nth(3)
        .expect("test executable under target/<profile>/deps")
        .join("apps")
}

#[cfg(test)]
mod tests {
    use super::*;

    const HASH: &str = "86d17c662662d538c9741aa8ccad7f9db8843eb719a27d72b0448f356a55f930";

    fn pin_text(url: &str, program: &str) -> String {
        format!(
            "# comment\nurl {url}\npackage {HASH}\nprogram {program}\nfile app/app.exe {HASH}\n"
        )
    }

    #[test]
    fn the_checked_in_pins_name_each_program_and_its_hash() {
        let vim = AppPin::parse(App::Vim.pin_text()).unwrap();
        assert_eq!(vim.program, "vim/vim92/vim.exe");
        assert!(vim.url.ends_with("/gvim_9.2.1259_x64.zip"), "{}", vim.url);
        let edit = AppPin::parse(App::Edit.pin_text()).unwrap();
        assert_eq!(edit.program, "edit-2.0.0-x86_64-windows/edit.exe");
        assert!(
            edit.url.ends_with("/v2.0.0/edit-2.0.0-x86_64-windows.zip"),
            "{}",
            edit.url
        );
    }

    #[test]
    fn a_malformed_pin_is_refused() {
        let good = pin_text("https://example.test/app.zip", "app/app.exe");
        assert!(AppPin::parse(&good).is_ok());
        for bad in [
            pin_text("http://example.test/app.zip", "app/app.exe"),
            pin_text("https://example.test/app.zip", "app/other.exe"),
            good.replace("package 86", "package 8"),
            good.replace("file app/app.exe 86", "file app/app.exe 8"),
            good.replace("url ", "# "),
            good.clone() + "program app/app.exe\n",
            good.clone() + "version 1\n",
        ] {
            assert!(AppPin::parse(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_downloaded_zip_with_another_hash_fails_and_leaves_no_folder() {
        let pin = AppPin::parse(&pin_text("https://example.test/app.zip", "app/app.exe")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let error = provide(&pin, root.path(), |url, to| {
            assert_eq!(url, "https://example.test/app.zip");
            fs::write(to, b"not the pinned zip")
        })
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("SHA-256 mismatch"), "{error}");
        assert!(!root.path().join(HASH).exists());
    }

    #[test]
    fn a_cached_program_with_another_hash_is_fetched_again() {
        let pin = AppPin::parse(&pin_text("https://example.test/app.zip", "app/app.exe")).unwrap();
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join(HASH).join("app/app.exe");
        fs::create_dir_all(program.parent().unwrap()).unwrap();
        fs::write(&program, b"tampered").unwrap();
        let fetched = std::cell::Cell::new(false);
        let error = provide(&pin, root.path(), |_, to| {
            fetched.set(true);
            fs::write(to, b"still not the zip")
        })
        .unwrap_err();
        assert!(fetched.get());
        assert!(error.to_string().contains("SHA-256 mismatch"), "{error}");
    }

    #[test]
    fn each_pinned_program_is_fetched_verified_and_runs() {
        for app in [App::Vim, App::Edit] {
            let program = app.program().unwrap();
            assert!(program.is_absolute() && program.is_file(), "{program:?}");
            let output = Command::new(&program).arg("--version").output().unwrap();
            assert!(output.status.success(), "{app:?}: {output:?}");
        }
    }
}
