//! #329: the Windows archive's end-to-end tests, which the Windows Archive job
//! runs on the zip the Windows Dist job built, named by
//! `CLIMON2_E2E_ARCHIVE` with `install.ps1` and `SHA256SUMS` beside it.
//! Without that variable each test passes without running.
//!
//! The archive's own `install.exe` installs it, and the download script
//! installs it under Windows PowerShell 5.1 and PowerShell 7 from a local web
//! server, as `irm | iex` and as a script block with `-Dir` and
//! `-NoModifyPath`. Every install gets its own `LOCALAPPDATA`, `TEMP` and
//! climon2 home. The tests that change the real user `PATH` run only when
//! `CLIMON2_E2E_REAL_USER_PATH=1` too, as the Archive job sets it on its
//! throwaway runner; they put `PATH` and the installer's record back after.
//!
//! Run as `hold_role` with `ROLE=hold`, this test binary is a program that runs
//! until its standard input closes, so a test can make a file of a home
//! "running".
#![cfg(windows)]

use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs,
    io::{Read, Write},
    net::TcpListener,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    ptr,
    sync::Mutex,
};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS},
    System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, REG_OPTION_NON_VOLATILE, REG_VALUE_TYPE,
        RegCreateKeyExW, RegDeleteValueW, RegQueryValueExW, RegSetValueExW,
    },
};

const ROLE: &str = "CLIMON2_ARCHIVE_TEST_ROLE";
const SHELLS: [&str; 2] = ["powershell.exe", "pwsh.exe"];

/// Runs until standard input closes, when started as the hold role.
#[test]
fn hold_role() {
    if std::env::var_os(ROLE).is_some() {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
    }
}

/// The archive under test, or `None` outside the Archive job.
fn archive() -> Option<PathBuf> {
    let archive = std::env::var_os("CLIMON2_E2E_ARCHIVE").map(PathBuf::from);
    if archive.is_none() {
        eprintln!("CLIMON2_E2E_ARCHIVE is not set; not testing an archive");
    }
    archive
}

/// Whether this run may change the real user `PATH`.
fn real_user_path() -> bool {
    let allowed = std::env::var_os("CLIMON2_E2E_REAL_USER_PATH").is_some_and(|value| value == "1");
    if !allowed {
        eprintln!("CLIMON2_E2E_REAL_USER_PATH is not 1; not changing the user PATH");
    }
    allowed
}

/// The archive's release folder, expanded as a manual install expands it.
struct Release {
    root: tempfile::TempDir,
    folder: PathBuf,
    version: String,
}

impl Release {
    fn expand(archive: &Path) -> Self {
        let root = tempfile::tempdir().unwrap();
        let name = archive.file_stem().unwrap().to_str().unwrap().to_owned();
        let version = name
            .strip_prefix("climon2-")
            .and_then(|rest| rest.strip_suffix("-windows-x86_64-msvc"))
            .expect("climon2-<version>-windows-x86_64-msvc.zip")
            .to_owned();
        let status = Command::new(system32("tar.exe"))
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(root.path())
            .status()
            .unwrap();
        assert!(status.success());
        let folder = root.path().join(&name);
        Self {
            root,
            folder,
            version,
        }
    }

    fn scratch(&self, name: &str) -> PathBuf {
        let path = self.root.path().join(name);
        fs::create_dir_all(&path).unwrap();
        path
    }

    /// Runs the archive's `install.exe` with `args`.
    fn install(&self, args: &[&OsStr]) -> Output {
        Command::new(self.folder.join("install.exe"))
            .args(args)
            .env("LOCALAPPDATA", self.scratch("local"))
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn install_into(&self, home: &Path) -> Output {
        self.install(&[
            "--dir".as_ref(),
            home.as_os_str(),
            "--no-modify-path".as_ref(),
        ])
    }

    fn version_folder(&self) -> BTreeMap<String, Vec<u8>> {
        contents(&self.folder.join(&self.version))
    }
}

fn system32(name: &str) -> PathBuf {
    Path::new(&std::env::var_os("SystemRoot").unwrap())
        .join("System32")
        .join(name)
}

fn contents(folder: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(folder)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().into_string().unwrap(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

#[track_caller]
fn succeeded(output: &Output) -> String {
    assert!(output.status.success(), "{output:#?}");
    assert_eq!(text(&output.stderr), "", "{output:#?}");
    text(&output.stdout)
}

fn hint(home: &Path) -> String {
    format!(
        "climon2: PATH was not changed. To run c2, add this entry to your user PATH:\n{}\n",
        home.display()
    )
}

/// A copy of this test binary at `path`, running as the hold role until dropped.
struct Running(Child);

impl Running {
    fn as_copy(path: &Path) -> Self {
        fs::copy(std::env::current_exe().unwrap(), path).unwrap();
        Self(
            Command::new(path)
                .args(["hold_role", "--exact", "--quiet"])
                .env(ROLE, "hold")
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap(),
        )
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn created(path: &Path) -> std::time::SystemTime {
    fs::metadata(path).unwrap().created().unwrap()
}

#[test]
fn the_archive_installs_and_its_launcher_runs_the_cli() {
    let Some(archive) = archive() else { return };
    let release = Release::expand(&archive);
    let home = release.root.path().join("home with space");
    assert_eq!(succeeded(&release.install_into(&home)), hint(&home));
    assert_eq!(
        contents(&home.join(&release.version)),
        release.version_folder()
    );
    assert_eq!(
        fs::read_to_string(home.join("current")).unwrap(),
        format!("{}\n", release.version)
    );
    let c2 = |args: &[&str]| {
        Command::new(home.join("c2.exe"))
            .args(args)
            .env("CLIMON2_HOME", release.scratch("climon2 home"))
            .output()
            .unwrap()
    };
    assert_eq!(
        succeeded(&c2(&["--version"])),
        format!("climon2 {}\n", release.version)
    );
    assert_eq!(succeeded(&c2(&["list"])), "No climon2 sessions running.\n");
}

#[test]
fn the_archive_installer_keeps_swaps_or_refuses_the_same_version() {
    let Some(archive) = archive() else { return };
    let release = Release::expand(&archive);
    let home = release.scratch("home");
    let folder = home.join(&release.version);
    succeeded(&release.install_into(&home));
    let kept = created(&folder);
    succeeded(&release.install_into(&home));
    assert_eq!(created(&folder), kept, "identical: kept");

    fs::write(folder.join("climon2-client.exe"), "rebuilt").unwrap();
    succeeded(&release.install_into(&home));
    assert_eq!(contents(&folder), release.version_folder());
    assert_ne!(created(&folder), kept, "different and unused: swapped");

    let client = folder.join("climon2-client.exe");
    {
        let _running = Running::as_copy(&client);
        let before = contents(&folder);
        let output = release.install_into(&home);
        assert_eq!(output.status.code(), Some(125), "{output:#?}");
        assert_eq!(
            text(&output.stderr),
            format!(
                "climon2: error: {} is running and the version being installed differs \
                 from it; end that version's sessions, then install again\n",
                client.display()
            )
        );
        assert_eq!(contents(&folder), before, "different and in use: refused");
    }
    succeeded(&release.install_into(&home));
    assert_eq!(contents(&folder), release.version_folder());
}

#[test]
fn the_archive_installer_keeps_a_running_launcher() {
    let Some(archive) = archive() else { return };
    let release = Release::expand(&archive);
    let home = release.scratch("home");
    succeeded(&release.install_into(&home));
    let launcher = home.join("c2.exe");
    {
        let _running = Running::as_copy(&launcher);
        let running = fs::read(&launcher).unwrap();
        let stdout = succeeded(&release.install_into(&home));
        let (kept, rest) = stdout.split_once('\n').unwrap();
        assert!(
            kept.starts_with(&format!(
                "climon2: kept {}, which is in use and could not be replaced (",
                launcher.display()
            )) && kept.ends_with("); it runs this version too"),
            "{stdout}"
        );
        assert_eq!(rest, hint(&home));
        assert_eq!(fs::read(&launcher).unwrap(), running);
    }
    succeeded(&release.install_into(&home));
    assert_eq!(
        fs::read(&launcher).unwrap(),
        fs::read(release.folder.join("c2.exe")).unwrap()
    );
}

#[test]
fn the_archive_installer_fails_at_once_while_another_holds_the_lock() {
    let Some(archive) = archive() else { return };
    let release = Release::expand(&archive);
    let home = release.scratch("home");
    let lock = fs::File::create(home.join("install.lock")).unwrap();
    lock.try_lock().unwrap();
    let output = release.install_into(&home);
    assert_eq!(output.status.code(), Some(125), "{output:#?}");
    assert!(
        text(&output.stderr).contains("is another installer running?"),
        "{output:#?}"
    );
    drop(lock);
    succeeded(&release.install_into(&home));
}

/// Serves the release's published files under `v<version>/`, as a release
/// download URL does, until the test process ends.
fn serve(release: &Release, archive: &Path, checksums: &[u8]) -> String {
    let dist = archive.parent().unwrap();
    let mut files = BTreeMap::new();
    let zip = archive.file_name().unwrap().to_str().unwrap();
    for (name, data) in [
        (zip, fs::read(archive).unwrap()),
        ("install.ps1", fs::read(dist.join("install.ps1")).unwrap()),
        ("SHA256SUMS", checksums.to_vec()),
    ] {
        files.insert(format!("/v{}/{name}", release.version), data);
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
                request.push(byte[0]);
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split(' ').nth(1).unwrap_or_default();
            let (status, body) = match files.get(path) {
                Some(body) => ("200 OK", body.as_slice()),
                None => ("404 Not Found", &b""[..]),
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\n\
                 Content-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    base
}

/// Runs `script` in `shell` with the download script's base URL, and a
/// `LOCALAPPDATA` and `TEMP` of the release's own.
fn powershell(release: &Release, shell: &str, base: &str, script: &str) -> Output {
    let temp = release.scratch(&format!("temp-{shell}"));
    Command::new(shell)
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .env("CLIMON2_RELEASE_BASE_URL", base)
        .env("LOCALAPPDATA", release.scratch(&format!("local-{shell}")))
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

#[test]
fn install_ps1_installs_in_both_powershells_as_a_script_block_with_options() {
    let Some(archive) = archive() else { return };
    let release = Release::expand(&archive);
    let sums = fs::read(archive.with_file_name("SHA256SUMS")).unwrap();
    let base = serve(&release, &archive, &sums);
    for shell in SHELLS {
        let home = release.root.path().join(format!("{shell} home's"));
        let script = format!(
            "& ([scriptblock]::Create((irm {base}/v{}/install.ps1))) -Dir {} -NoModifyPath; \
             'still here'",
            release.version,
            quoted(&home)
        );
        let output = powershell(&release, shell, &base, &script);
        assert_eq!(succeeded(&output), hint(&home) + "still here\n", "{shell}");
        assert_eq!(
            contents(&home.join(&release.version)),
            release.version_folder()
        );
        let temp = release.scratch(&format!("temp-{shell}"));
        assert_eq!(
            fs::read_dir(&temp).unwrap().count(),
            0,
            "{shell}: temporary folder removed"
        );
    }
}

#[test]
fn install_ps1_refuses_a_wrong_checksum_in_both_powershells_without_exiting() {
    let Some(archive) = archive() else { return };
    let release = Release::expand(&archive);
    let zip = archive.file_name().unwrap().to_str().unwrap().to_owned();
    let sums = format!("{}  {zip}\n", "0".repeat(64));
    let base = serve(&release, &archive, sums.as_bytes());
    let install = format!("irm {base}/v{}/install.ps1", release.version);
    for shell in SHELLS {
        for form in [
            format!("{install} | iex"),
            format!("& ([scriptblock]::Create(({install}))) -NoModifyPath"),
        ] {
            let script = format!("try {{ {form} }} catch {{ \"caught: $_\" }}; 'still here'");
            let output = powershell(&release, shell, &base, &script);
            assert_eq!(
                text(&output.stdout),
                "caught: climon2-install.ps1: error: archive checksum verification failed\n\
                 still here\n",
                "{shell}: {form}: {output:#?}"
            );
            let local = release.scratch(&format!("local-{shell}"));
            assert!(
                !local.join("climon2").exists(),
                "{shell}: nothing installed"
            );
        }
    }
}

/// The user's `PATH` and the installer's record, put back when dropped.
struct UserPath {
    path: Option<(Vec<u8>, REG_VALUE_TYPE)>,
    record: Option<(Vec<u8>, REG_VALUE_TYPE)>,
    _serial: std::sync::MutexGuard<'static, ()>,
}

static SERIAL: Mutex<()> = Mutex::new(());

fn wide(text: &str) -> Vec<u16> {
    OsStr::new(text).encode_wide().chain([0]).collect()
}

fn key(path: &str) -> HKEY {
    let mut key = ptr::null_mut();
    let status = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(path).as_ptr(),
            0,
            ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    assert_eq!(status, ERROR_SUCCESS);
    key
}

fn read(key_path: &str, name: &str) -> Option<(Vec<u8>, REG_VALUE_TYPE)> {
    let key = key(key_path);
    let name = wide(name);
    let (mut kind, mut size) = (0, 0);
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            ptr::null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return None;
    }
    let mut data = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            key,
            name.as_ptr(),
            ptr::null(),
            &mut kind,
            data.as_mut_ptr(),
            &mut size,
        )
    };
    assert_eq!(status, ERROR_SUCCESS);
    Some((data, kind))
}

fn write(key_path: &str, name: &str, value: &Option<(Vec<u8>, REG_VALUE_TYPE)>) {
    let key = key(key_path);
    let name = wide(name);
    unsafe {
        match value {
            Some((data, kind)) => RegSetValueExW(
                key,
                name.as_ptr(),
                0,
                *kind,
                data.as_ptr(),
                data.len() as u32,
            ),
            None => RegDeleteValueW(key, name.as_ptr()),
        }
    };
}

fn as_text(value: Option<(Vec<u8>, REG_VALUE_TYPE)>) -> String {
    let Some((data, _)) = value else {
        return String::new();
    };
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16(&units).unwrap()
}

impl UserPath {
    fn save() -> Self {
        let serial = SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Self {
            path: read("Environment", "Path"),
            record: read(r"Software\climon2", "PathEntry"),
            _serial: serial,
        }
    }

    /// How many entries of the user `PATH` are exactly `home`.
    fn entries(&self, home: &Path) -> usize {
        as_text(read("Environment", "Path"))
            .split(';')
            .filter(|entry| Path::new(entry) == home)
            .count()
    }

    fn record(&self) -> String {
        as_text(read(r"Software\climon2", "PathEntry"))
    }
}

impl Drop for UserPath {
    fn drop(&mut self) {
        write("Environment", "Path", &self.path);
        write(r"Software\climon2", "PathEntry", &self.record);
    }
}

#[test]
fn the_archive_installer_adds_the_home_to_the_user_path_once() {
    let Some(archive) = archive() else { return };
    if !real_user_path() {
        return;
    }
    let release = Release::expand(&archive);
    let path = UserPath::save();
    let (first, second) = (release.scratch("first"), release.scratch("second"));
    for _ in 0..2 {
        release.install(&["--dir".as_ref(), first.as_os_str()]);
        assert_eq!(path.entries(&first), 1);
        assert_eq!(path.record(), first.display().to_string());
    }
    succeeded(&release.install(&["--dir".as_ref(), second.as_os_str()]));
    assert_eq!((path.entries(&first), path.entries(&second)), (0, 1));
    assert_eq!(path.record(), second.display().to_string());
    let third = release.scratch("third");
    assert_eq!(succeeded(&release.install_into(&third)), hint(&third));
    assert_eq!((path.entries(&second), path.entries(&third)), (1, 0));
    assert_eq!(path.record(), second.display().to_string());
}

#[test]
fn install_ps1_installs_in_both_powershells_through_iex() {
    let Some(archive) = archive() else { return };
    if !real_user_path() {
        return;
    }
    let release = Release::expand(&archive);
    let sums = fs::read(archive.with_file_name("SHA256SUMS")).unwrap();
    let base = serve(&release, &archive, &sums);
    let path = UserPath::save();
    for shell in SHELLS {
        let script = format!(
            "irm {base}/v{}/install.ps1 | iex; 'still here'",
            release.version
        );
        let output = powershell(&release, shell, &base, &script);
        let home = release.scratch(&format!("local-{shell}")).join("climon2");
        assert_eq!(
            succeeded(&output),
            format!(
                "climon2: added {} to your user PATH; terminals opened before now do not see it\n\
                 still here\n",
                home.display()
            ),
            "{shell}"
        );
        assert_eq!(
            contents(&home.join(&release.version)),
            release.version_folder()
        );
        assert_eq!(path.entries(&home), 1, "{shell}");
        assert_eq!(path.record(), home.display().to_string());
    }
}
