//! #329 / docs/for-humans/windows/distribution.md, "Installing" and "Upgrading": the
//! Windows installer places a release into an executable home, keeps one
//! installer at a time, keeps, swaps or refuses a same-version folder, keeps a
//! running launcher and adds the home to the user's PATH exactly once.
//!
//! The installer under test is this test binary, copied into a release folder
//! as `install.exe` and run with `ROLE=install`: it redirects
//! `HKEY_CURRENT_USER` to a registry key of the test's own, then runs the
//! installer library as `install.exe` does. The real user PATH is therefore
//! never touched, whatever the installer does. Each install also gets its own
//! `LOCALAPPDATA` and working directory, and can count its own broadcasts.
//! Run with `ROLE=hold`, the binary is a program that runs until its standard
//! input closes, so a test can make a file of the home "running".
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    if let Ok(role) = std::env::var(support::ROLE) {
        support::act(&role);
    }
    harness::run(&[
        (
            "a_fresh_install_lays_out_the_release_and_adds_the_home_to_path",
            tests::a_fresh_install_lays_out_the_release_and_adds_the_home_to_path,
        ),
        (
            "the_dir_option_names_the_home_and_each_option_comes_once",
            tests::the_dir_option_names_the_home_and_each_option_comes_once,
        ),
        (
            "without_dir_local_application_data_must_be_absolute",
            tests::without_dir_local_application_data_must_be_absolute,
        ),
        (
            "a_missing_release_file_fails_the_install_naming_it",
            tests::a_missing_release_file_fails_the_install_naming_it,
        ),
        (
            "an_interrupted_installs_stage_is_removed",
            tests::an_interrupted_installs_stage_is_removed,
        ),
        (
            "a_second_installer_fails_at_once_and_the_lock_file_stays",
            tests::a_second_installer_fails_at_once_and_the_lock_file_stays,
        ),
        (
            "an_identical_reinstall_keeps_the_version_folder_even_in_use",
            tests::an_identical_reinstall_keeps_the_version_folder_even_in_use,
        ),
        (
            "a_different_unused_version_folder_is_swapped_for_the_release",
            tests::a_different_unused_version_folder_is_swapped_for_the_release,
        ),
        (
            "a_different_version_folder_in_use_is_refused_and_nothing_changes",
            tests::a_different_version_folder_in_use_is_refused_and_nothing_changes,
        ),
        (
            "a_different_launcher_is_replaced_and_a_running_one_is_kept",
            tests::a_different_launcher_is_replaced_and_a_running_one_is_kept,
        ),
        (
            "path_gets_one_literal_entry_of_the_same_registry_type",
            tests::path_gets_one_literal_entry_of_the_same_registry_type,
        ),
        (
            "a_directory_change_replaces_the_recorded_entry",
            tests::a_directory_change_replaces_the_recorded_entry,
        ),
        (
            "an_entry_the_user_added_counts_as_present_and_is_not_recorded",
            tests::an_entry_the_user_added_counts_as_present_and_is_not_recorded,
        ),
        (
            "no_modify_path_touches_neither_path_nor_record_and_prints_the_entry",
            tests::no_modify_path_touches_neither_path_nor_record_and_prints_the_entry,
        ),
        (
            "a_path_value_that_is_not_a_string_is_left_unchanged",
            tests::a_path_value_that_is_not_a_string_is_left_unchanged,
        ),
        (
            "a_home_that_is_not_unicode_is_not_added_to_path",
            tests::a_home_that_is_not_unicode_is_not_added_to_path,
        ),
        (
            "a_registry_failure_fails_the_install",
            tests::a_registry_failure_fails_the_install,
        ),
        (
            "a_path_change_is_broadcast_to_running_programs",
            tests::a_path_change_is_broadcast_to_running_programs,
        ),
    ]);
}

/// The installer's own registry key, the release folder, and the roles this
/// binary plays.
#[cfg(windows)]
mod support {
    use std::{
        collections::BTreeMap,
        ffi::{OsStr, OsString},
        fs,
        io::Read,
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        process::{Child, Command, Stdio},
        ptr,
        sync::atomic::{AtomicU32, Ordering},
        time::{Duration, Instant},
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LocalFree},
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        },
        System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_ALL_ACCESS, KEY_READ, REG_OPTION_NON_VOLATILE, REG_SZ,
            REG_VALUE_TYPE, RegCreateKeyExW, RegDeleteKeyW, RegDeleteTreeW, RegOverridePredefKey,
            RegQueryValueExW, RegSetKeySecurity, RegSetValueExW,
        },
    };

    /// The environment variable naming the role this binary plays.
    pub const ROLE: &str = "CLIMON2_INSTALL_TEST_ROLE";
    /// The environment variable naming the installer's registry key.
    const KEY: &str = "CLIMON2_INSTALL_TEST_KEY";
    pub const VERSION: &str = env!("CARGO_PKG_VERSION");
    /// A version folder's files: the agreed archive layout
    /// (docs/for-humans/windows/distribution.md, "The archive").
    pub const VERSION_FILES: [&str; 8] = [
        "climon2-cli.exe",
        "climon2-daemon.exe",
        "climon2-client.exe",
        "climon2-monitor.exe",
        "climon2_client.dll",
        "conpty.dll",
        "OpenConsole.exe",
        "THIRD-PARTY-NOTICES.txt",
    ];
    /// The version folder's program this binary is copied as, so it can run.
    pub const RUNNABLE: &str = "climon2-cli.exe";

    pub fn act(role: &str) -> ! {
        match role {
            "install" => install(),
            "hold" => hold(),
            other => panic!("unknown role {other:?}"),
        }
    }

    /// `install.exe`, with `HKEY_CURRENT_USER` redirected to the test's key.
    fn install() -> ! {
        let key = open(&std::env::var(KEY).unwrap());
        assert_eq!(
            unsafe { RegOverridePredefKey(HKEY_CURRENT_USER, key) },
            ERROR_SUCCESS
        );
        match crate::broadcast::counted(|| climon2_install::run(std::env::args_os().skip(1))) {
            Ok(()) => std::process::exit(0),
            Err(error) => {
                eprintln!("climon2: error: {error}");
                std::process::exit(125)
            }
        }
    }

    /// Runs until standard input closes.
    fn hold() -> ! {
        let _ = std::io::stdin().read_to_end(&mut Vec::new());
        std::process::exit(0)
    }

    pub fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
        text.as_ref().encode_wide().chain([0]).collect()
    }

    /// Access to change a key's access-control list, which its owner always has.
    const WRITE_DAC: u32 = 0x0004_0000;

    /// The key at `path` under the real `HKEY_CURRENT_USER`, created when missing.
    fn open(path: &str) -> HKEY {
        open_for(path, KEY_ALL_ACCESS)
    }

    fn open_for(path: &str, access: u32) -> HKEY {
        let mut key = ptr::null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(path).as_ptr(),
                0,
                ptr::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                ptr::null(),
                &mut key,
                ptr::null_mut(),
            )
        };
        assert_eq!(status, ERROR_SUCCESS, "{path}");
        key
    }

    /// A registry key standing in for `HKEY_CURRENT_USER`, deleted when dropped.
    pub struct Registry(String);

    static NEXT: AtomicU32 = AtomicU32::new(0);
    const PARENT: &str = r"Software\climon2-install-tests";

    impl Registry {
        fn new() -> Self {
            let path = format!(
                r"{PARENT}\{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            );
            open(&path);
            Self(path)
        }

        fn value(&self, key: &str, name: &str) -> Option<(Vec<u8>, REG_VALUE_TYPE)> {
            let key = open_for(&format!(r"{}\{key}", self.0), KEY_READ);
            let name = wide(name);
            let (mut kind, mut size) = (0, 0);
            let query = |data: *mut u8, kind: &mut u32, size: &mut u32| unsafe {
                RegQueryValueExW(key, name.as_ptr(), ptr::null(), kind, data, size)
            };
            let status = query(ptr::null_mut(), &mut kind, &mut size);
            if status == ERROR_FILE_NOT_FOUND {
                return None;
            }
            assert_eq!(status, ERROR_SUCCESS);
            let mut data = vec![0u8; size as usize];
            assert_eq!(
                query(data.as_mut_ptr(), &mut kind, &mut size),
                ERROR_SUCCESS
            );
            Some((data, kind))
        }

        fn text(&self, key: &str, name: &str) -> Option<(String, REG_VALUE_TYPE)> {
            self.value(key, name).map(|(data, kind)| {
                let units: Vec<u16> = data
                    .chunks_exact(2)
                    .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                    .collect();
                assert_eq!(units.last(), Some(&0), "stored with its terminator");
                (String::from_utf16(&units[..units.len() - 1]).unwrap(), kind)
            })
        }

        fn set(&self, key: &str, name: &str, kind: REG_VALUE_TYPE, data: &[u8]) {
            let key = open(&format!(r"{}\{key}", self.0));
            let status = unsafe {
                RegSetValueExW(
                    key,
                    wide(name).as_ptr(),
                    0,
                    kind,
                    data.as_ptr(),
                    data.len() as u32,
                )
            };
            assert_eq!(status, ERROR_SUCCESS);
        }

        fn set_text(&self, key: &str, name: &str, kind: REG_VALUE_TYPE, text: &str) {
            let data: Vec<u8> = wide(text)
                .iter()
                .flat_map(|unit| unit.to_le_bytes())
                .collect();
            self.set(key, name, kind, &data);
        }

        /// The user PATH's stored text and registry type.
        pub fn path(&self) -> Option<(String, REG_VALUE_TYPE)> {
            self.text("Environment", "Path")
        }

        pub fn set_path(&self, kind: REG_VALUE_TYPE, text: &str) {
            self.set_text("Environment", "Path", kind, text);
        }

        pub fn set_path_value(&self, kind: REG_VALUE_TYPE, data: &[u8]) {
            self.set("Environment", "Path", kind, data);
        }

        /// The entry the installer recorded as its own.
        pub fn record(&self) -> Option<String> {
            self.text(r"Software\climon2", "PathEntry")
                .map(|(text, _)| text)
        }

        pub fn set_record(&self, entry: &str) {
            self.set_text(r"Software\climon2", "PathEntry", REG_SZ, entry);
        }

        /// Gives `key` the access-control list `sddl` describes. As its
        /// owner, the test can always give access back.
        pub fn secure(&self, key: &str, sddl: &str) {
            let key = open_for(&format!(r"{}\{key}", self.0), WRITE_DAC);
            let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
            let converted = unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    wide(sddl).as_ptr(),
                    SDDL_REVISION_1,
                    &mut descriptor,
                    ptr::null_mut(),
                )
            };
            assert_ne!(converted, 0);
            let status = unsafe { RegSetKeySecurity(key, DACL_SECURITY_INFORMATION, descriptor) };
            unsafe { LocalFree(descriptor) };
            assert_eq!(status, ERROR_SUCCESS);
        }
    }

    impl Drop for Registry {
        fn drop(&mut self) {
            self.secure("Environment", "D:(A;;KA;;;WD)");
            unsafe {
                RegDeleteTreeW(open(&self.0), ptr::null());
                RegDeleteKeyW(
                    open(PARENT),
                    wide(self.0.rsplit('\\').next().unwrap()).as_ptr(),
                );
                // Fails while another test's key is still in it.
                RegDeleteKeyW(HKEY_CURRENT_USER, wide(PARENT).as_ptr());
            }
        }
    }

    /// How an install ended.
    #[derive(Debug)]
    pub struct Outcome {
        pub status: i32,
        pub stdout: String,
        pub stderr: String,
        pub took: Duration,
    }

    impl Outcome {
        #[track_caller]
        pub fn succeeded(&self) -> &Self {
            assert_eq!(self.status, 0, "{self:#?}");
            assert_eq!(self.stderr, "", "{self:#?}");
            self
        }

        #[track_caller]
        pub fn failed(&self, message: &str) -> &Self {
            assert_eq!(self.status, 125, "{self:#?}");
            assert!(
                self.stderr.starts_with("climon2: error: ") && self.stderr.contains(message),
                "{self:#?}"
            );
            assert_eq!(self.stderr.lines().count(), 1, "{self:#?}");
            self
        }
    }

    /// A release folder, a profile and a registry key for one test.
    pub struct World {
        root: tempfile::TempDir,
        pub registry: Registry,
        variables: BTreeMap<OsString, OsString>,
    }

    impl World {
        /// A release whose files hold their own names and "one", except that
        /// the version folder's CLI is this binary, so it can run.
        pub fn new() -> Self {
            let root = tempfile::tempdir().unwrap();
            for folder in ["release", "local", "work"] {
                fs::create_dir(root.path().join(folder)).unwrap();
            }
            let world = Self {
                root,
                registry: Registry::new(),
                variables: BTreeMap::new(),
            };
            let release = world.release();
            copy_self(&release.join("install.exe"));
            fs::write(release.join("c2.exe"), "c2.exe one").unwrap();
            fs::create_dir(release.join(VERSION)).unwrap();
            for name in VERSION_FILES {
                fs::write(release.join(VERSION).join(name), format!("{name} one")).unwrap();
            }
            copy_self(&release.join(VERSION).join(RUNNABLE));
            world
        }

        pub fn root(&self) -> &Path {
            self.root.path()
        }

        pub fn release(&self) -> PathBuf {
            self.root().join("release")
        }

        /// Where the installer goes without `--dir`.
        pub fn home(&self) -> PathBuf {
            self.local().join("climon2")
        }

        pub fn local(&self) -> PathBuf {
            self.root().join("local")
        }

        pub fn work(&self) -> PathBuf {
            self.root().join("work")
        }

        /// Sets `name` for every later install; `LOCALAPPDATA` replaces the
        /// world's own.
        pub fn set(&mut self, name: &str, value: impl Into<OsString>) {
            self.variables.insert(name.into(), value.into());
        }

        pub fn install(&self, args: &[&str]) -> Outcome {
            let args: Vec<OsString> = args.iter().map(OsString::from).collect();
            self.install_os(&args)
        }

        pub fn install_os(&self, args: &[OsString]) -> Outcome {
            let mut command = Command::new(self.release().join("install.exe"));
            command
                .args(args)
                .current_dir(self.work())
                .env(ROLE, "install")
                .env(KEY, &self.registry.0)
                .env("LOCALAPPDATA", self.local())
                .envs(&self.variables)
                .stdin(Stdio::null());
            let started = Instant::now();
            let output = command.output().unwrap();
            Outcome {
                status: output.status.code().unwrap(),
                stdout: String::from_utf8(output.stdout).unwrap(),
                stderr: String::from_utf8(output.stderr).unwrap(),
                took: started.elapsed(),
            }
        }
    }

    /// Copies this binary to `path`.
    pub fn copy_self(path: &Path) {
        fs::copy(std::env::current_exe().unwrap(), path).unwrap();
    }

    /// The names in `folder` with their contents.
    pub fn contents(folder: &Path) -> BTreeMap<String, Vec<u8>> {
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

    /// A program of the home, running until dropped.
    pub struct Running(Child);

    impl Running {
        pub fn start(program: &Path) -> Self {
            Self(
                Command::new(program)
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
}

/// Counts the broadcasts that the environment changed which an install sends
/// itself, from a hidden top-level window on the thread that runs it. Every
/// broadcast reaches the window, but only the install's own calls its
/// procedure directly; another program's, a test's or a worker's, arrives
/// from another thread. A broadcast waits for the window's answer, so the
/// count is complete when the installer returns.
#[cfg(windows)]
mod broadcast {
    use std::sync::atomic::{AtomicU32, Ordering};
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, ISMEX_NOSEND, InSendMessageEx, RegisterClassW,
            WM_SETTINGCHANGE, WNDCLASSW, WS_OVERLAPPED,
        },
    };

    /// The environment variable naming the file an install writes the
    /// number of its own broadcasts to.
    pub const COUNT: &str = "CLIMON2_INSTALL_TEST_BROADCASTS";
    static SEEN: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        // Not sent from another thread: sent by this one, the installer's.
        let own = unsafe { InSendMessageEx(std::ptr::null_mut()) } == ISMEX_NOSEND;
        if own && unsafe { environment(message, wparam, lparam) } {
            SEEN.fetch_add(1, Ordering::SeqCst);
        }
        unsafe { DefWindowProcW(window, message, wparam, lparam) }
    }

    /// Whether the message says the environment changed: it names the
    /// environment and carries no system setting.
    unsafe fn environment(message: u32, wparam: WPARAM, lparam: LPARAM) -> bool {
        message == WM_SETTINGCHANGE
            && wparam == 0
            && lparam != 0
            && unsafe { area(lparam) } == "Environment"
    }

    /// The text a settings change names.
    unsafe fn area(lparam: LPARAM) -> String {
        let text = lparam as *const u16;
        let length = (0..)
            .take_while(|&at| unsafe { *text.add(at) } != 0)
            .count();
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(text, length) })
    }

    /// Runs `install`, and when the test names a file, starts the window on
    /// this thread first and writes the count to the file after.
    pub fn counted<T>(install: impl FnOnce() -> T) -> T {
        let Some(file) = std::env::var_os(COUNT) else {
            return install();
        };
        watch();
        let result = install();
        std::fs::write(file, SEEN.load(Ordering::SeqCst).to_string()).unwrap();
        result
    }

    /// Creates the window on this thread, which only answers messages while
    /// it waits in a send of its own.
    fn watch() {
        unsafe {
            let class = super::support::wide("climon2-install-tests");
            let instance = GetModuleHandleW(std::ptr::null());
            let mut description: WNDCLASSW = std::mem::zeroed();
            description.lpfnWndProc = Some(procedure);
            description.hInstance = instance;
            description.lpszClassName = class.as_ptr();
            assert_ne!(RegisterClassW(&description), 0);
            let window = CreateWindowExW(
                0,
                class.as_ptr(),
                class.as_ptr(),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            assert!(!window.is_null());
        }
    }
}

/// Runs the tests whose names contain any of the non-flag arguments (every
/// test without one) at once, and exits nonzero if any failed.
#[cfg(windows)]
mod harness {
    pub fn run(tests: &[(&'static str, fn())]) {
        let filters: Vec<String> = std::env::args()
            .skip(1)
            .filter(|argument| !argument.starts_with('-'))
            .collect();
        let running: Vec<_> = tests
            .iter()
            .filter(|(name, _)| {
                filters.is_empty() || filters.iter().any(|f| name.contains(f.as_str()))
            })
            .map(|&(name, test)| (name, std::thread::spawn(test)))
            .collect();
        let total = running.len();
        let mut failed = 0;
        for (name, thread) in running {
            let outcome = if thread.join().is_ok() {
                "ok"
            } else {
                "FAILED"
            };
            failed += usize::from(outcome == "FAILED");
            println!("test {name} ... {outcome}");
        }
        println!("{} passed; {failed} failed", total - failed);
        std::process::exit(i32::from(failed > 0));
    }
}

#[cfg(windows)]
mod tests {
    use crate::support::*;
    use std::{collections::BTreeMap, fs, os::windows::ffi::OsStrExt, path::Path, time::Duration};
    use windows_sys::Win32::System::Registry::{REG_DWORD, REG_EXPAND_SZ, REG_SZ};

    const USAGE: &str = "usage: install [--dir <directory>] [--no-modify-path]";

    fn added(home: &Path) -> String {
        format!(
            "climon2: added {} to your user PATH; terminals opened before now do not see it\n",
            home.display()
        )
    }

    fn release_version(world: &World) -> BTreeMap<String, Vec<u8>> {
        contents(&world.release().join(VERSION))
    }

    #[track_caller]
    fn assert_installed(world: &World, home: &Path) {
        assert_eq!(contents(&home.join(VERSION)), release_version(world));
        assert_eq!(fs::read(home.join("c2.exe")).unwrap(), b"c2.exe one");
        assert_eq!(
            fs::read_to_string(home.join("current")).unwrap(),
            format!("{VERSION}\n")
        );
        assert!(home.join("install.lock").is_file());
        assert!(!home.join(".climon2-install-stage").exists());
    }

    fn created(path: &Path) -> std::time::SystemTime {
        fs::metadata(path).unwrap().created().unwrap()
    }

    pub fn a_fresh_install_lays_out_the_release_and_adds_the_home_to_path() {
        let world = World::new();
        let home = world.home();
        let outcome = world.install(&[]);
        assert_eq!(outcome.succeeded().stdout, added(&home));
        assert_installed(&world, &home);
        let mut names: Vec<_> = fs::read_dir(&home)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [VERSION, "c2.exe", "current", "install.lock"]);
        // A user without a PATH of their own gets one of the type Windows uses.
        let entry = home.to_str().unwrap().to_owned();
        assert_eq!(world.registry.path(), Some((entry.clone(), REG_EXPAND_SZ)));
        assert_eq!(world.registry.record(), Some(entry));
    }

    pub fn the_dir_option_names_the_home_and_each_option_comes_once() {
        let world = World::new();
        world
            .install(&["--dir", "relative", "--no-modify-path"])
            .succeeded();
        assert_installed(&world, &world.work().join("relative"));
        let absolute = world.root().join("absolute home");
        world
            .install(&["--no-modify-path", "--dir", absolute.to_str().unwrap()])
            .succeeded();
        assert_installed(&world, &absolute);
        for args in [
            &["--bogus"][..],
            &["--dir"],
            &["--dir", ""],
            &["--dir", "a", "--dir", "b"],
            &["--no-modify-path", "--no-modify-path"],
            &["--no-modify-path", "extra"],
        ] {
            world.install(args).failed(USAGE);
        }
        assert!(!world.home().exists());
        assert!(!world.work().join("a").exists());
        assert_eq!(world.registry.path(), None);
    }

    pub fn without_dir_local_application_data_must_be_absolute() {
        let mut world = World::new();
        world.set("LOCALAPPDATA", "relative");
        world
            .install(&["--no-modify-path"])
            .failed("LOCALAPPDATA must name an absolute directory without --dir");
        assert!(!world.work().join("relative").exists());
    }

    pub fn a_missing_release_file_fails_the_install_naming_it() {
        let world = World::new();
        let missing = world.release().join(VERSION).join("conpty.dll");
        fs::remove_file(&missing).unwrap();
        world
            .install(&[])
            .failed(&format!("cannot copy {}: ", missing.display()));
        let home = world.home();
        assert!(!home.join(VERSION).exists());
        assert!(!home.join("current").exists());
        assert!(!home.join(".climon2-install-stage").exists());
        assert_eq!(world.registry.path(), None);
        fs::write(&missing, "conpty.dll one").unwrap();
        let launcher = world.release().join("c2.exe");
        fs::remove_file(&launcher).unwrap();
        world
            .install(&[])
            .failed(&format!("cannot copy {}: ", launcher.display()));
        assert!(!home.join("current").exists());
    }

    pub fn an_interrupted_installs_stage_is_removed() {
        let world = World::new();
        let stage = world.home().join(".climon2-install-stage");
        fs::create_dir_all(stage.join("version")).unwrap();
        fs::write(stage.join("version").join("left over"), "x").unwrap();
        world.install(&["--no-modify-path"]).succeeded();
        assert_installed(&world, &world.home());
    }

    pub fn a_second_installer_fails_at_once_and_the_lock_file_stays() {
        let world = World::new();
        let home = world.home();
        fs::create_dir_all(&home).unwrap();
        let lock = fs::File::create(home.join("install.lock")).unwrap();
        lock.try_lock().unwrap();
        let outcome = world.install(&[]);
        outcome.failed(&format!(
            "cannot lock {}; is another installer running?",
            home.join("install.lock").display()
        ));
        assert!(outcome.took < Duration::from_secs(10), "{outcome:#?}");
        assert!(!home.join(VERSION).exists());
        assert_eq!(world.registry.path(), None);
        drop(lock);
        world.install(&["--no-modify-path"]).succeeded();
        world.install(&["--no-modify-path"]).succeeded();
        assert_installed(&world, &home);
    }

    pub fn an_identical_reinstall_keeps_the_version_folder_even_in_use() {
        let world = World::new();
        let home = world.home();
        world.install(&["--no-modify-path"]).succeeded();
        let folder = created(&home.join(VERSION));
        let launcher = created(&home.join("c2.exe"));
        let _running = Running::start(&home.join(VERSION).join(RUNNABLE));
        world.install(&["--no-modify-path"]).succeeded();
        assert_eq!(created(&home.join(VERSION)), folder);
        assert_eq!(created(&home.join("c2.exe")), launcher);
        assert_installed(&world, &home);
    }

    pub fn a_different_unused_version_folder_is_swapped_for_the_release() {
        let world = World::new();
        let home = world.home();
        let folder = home.join(VERSION);
        // A rebuilt file, a file the release no longer has, and one it now lacks.
        for change in ["changed", "added", "removed"] {
            world.install(&["--no-modify-path"]).succeeded();
            let before = created(&folder);
            match change {
                "changed" => fs::write(folder.join("climon2-daemon.exe"), "two").unwrap(),
                "added" => fs::write(folder.join("stray.exe"), "stray").unwrap(),
                _ => fs::remove_file(folder.join("conpty.dll")).unwrap(),
            }
            world.install(&["--no-modify-path"]).succeeded();
            assert_installed(&world, &home);
            assert_ne!(created(&folder), before, "{change}");
        }
    }

    pub fn a_different_version_folder_in_use_is_refused_and_nothing_changes() {
        let world = World::new();
        let home = world.home();
        world.install(&["--no-modify-path"]).succeeded();
        let folder = home.join(VERSION);
        let daemon = folder.join("climon2-daemon.exe");
        copy_self(&daemon);
        fs::write(home.join("c2.exe"), "c2.exe two").unwrap();
        fs::write(home.join("current"), "0.0.1\n").unwrap();
        let before = contents(&folder);
        {
            let _running = Running::start(&daemon);
            world.install(&[]).failed(&format!(
                "{} is running and the version being installed differs from it; \
                 end that version's sessions, then install again",
                daemon.display()
            ));
            assert_eq!(contents(&folder), before);
            assert_eq!(fs::read(home.join("c2.exe")).unwrap(), b"c2.exe two");
            assert_eq!(fs::read_to_string(home.join("current")).unwrap(), "0.0.1\n");
            assert_eq!(world.registry.path(), None);
            assert!(!home.join(".climon2-install-stage").exists());
        }
        world.install(&["--no-modify-path"]).succeeded();
        assert_installed(&world, &home);
    }

    pub fn a_different_launcher_is_replaced_and_a_running_one_is_kept() {
        let world = World::new();
        let home = world.home();
        let launcher = home.join("c2.exe");
        world.install(&["--no-modify-path"]).succeeded();
        fs::write(&launcher, "c2.exe two").unwrap();
        let outcome = world.install(&["--no-modify-path"]);
        assert!(!outcome.succeeded().stdout.contains("kept"), "{outcome:#?}");
        assert_installed(&world, &home);

        copy_self(&launcher);
        let running = fs::read(&launcher).unwrap();
        fs::write(home.join("current"), "0.0.1\n").unwrap();
        let _running = Running::start(&launcher);
        let outcome = world.install(&[]);
        let stdout: Vec<_> = outcome.succeeded().stdout.lines().collect();
        assert_eq!(stdout.len(), 2, "{outcome:#?}");
        let (start, end) = (
            format!(
                "climon2: kept {}, which is in use and could not be replaced (",
                launcher.display()
            ),
            "); it runs this version too",
        );
        assert!(
            stdout[0].starts_with(&start) && stdout[0].ends_with(end),
            "{outcome:#?}"
        );
        assert_eq!(format!("{}\n", stdout[1]), added(&home));
        assert_eq!(fs::read(&launcher).unwrap(), running);
        assert_eq!(
            fs::read_to_string(home.join("current")).unwrap(),
            format!("{VERSION}\n")
        );
        assert_eq!(contents(&home.join(VERSION)), release_version(&world));
    }

    pub fn path_gets_one_literal_entry_of_the_same_registry_type() {
        for (kind, before) in [
            (REG_SZ, r"C:\one;D:\two"),
            (REG_EXPAND_SZ, r"%SystemRoot%\system32;;C:\three;"),
            (REG_SZ, ""),
        ] {
            let world = World::new();
            world.registry.set_path(kind, before);
            let home = world.home();
            let entry = home.to_str().unwrap();
            let after = if before.is_empty() || before.ends_with(';') {
                format!("{before}{entry}")
            } else {
                format!("{before};{entry}")
            };
            assert_eq!(world.install(&[]).succeeded().stdout, added(&home));
            assert_eq!(world.registry.path(), Some((after.clone(), kind)));
            assert_eq!(world.registry.record().as_deref(), Some(entry));
            // Reinstalls and upgrades leave it as it is.
            assert_eq!(world.install(&[]).succeeded().stdout, "");
            assert_eq!(world.registry.path(), Some((after, kind)));
            assert_eq!(world.registry.record().as_deref(), Some(entry));
        }
    }

    pub fn a_directory_change_replaces_the_recorded_entry() {
        let world = World::new();
        world.registry.set_path(REG_SZ, r"C:\one");
        let first = world.root().join("first");
        let second = world.root().join("second");
        world
            .install(&["--dir", first.to_str().unwrap()])
            .succeeded();
        world
            .registry
            .set_path(REG_SZ, &format!(r"C:\one;{};D:\two", first.display()));
        let outcome = world.install(&["--dir", second.to_str().unwrap()]);
        assert_eq!(outcome.succeeded().stdout, added(&second));
        let entry = second.to_str().unwrap();
        assert_eq!(
            world.registry.path(),
            Some((format!(r"C:\one;D:\two;{entry}"), REG_SZ))
        );
        assert_eq!(world.registry.record().as_deref(), Some(entry));
    }

    pub fn an_entry_the_user_added_counts_as_present_and_is_not_recorded() {
        let mut world = World::new();
        world.set("CLIMON2_INSTALL_TEST_BASE", world.local());
        let old = world.root().join("old");
        world.install(&["--dir", old.to_str().unwrap()]).succeeded();
        let before = format!(
            r"C:\one;{};%CLIMON2_INSTALL_TEST_BASE%\CLIMON2\",
            old.display()
        );
        world.registry.set_path(REG_EXPAND_SZ, &before);
        assert_eq!(world.install(&[]).succeeded().stdout, "");
        assert_eq!(
            world.registry.path(),
            Some((
                r"C:\one;%CLIMON2_INSTALL_TEST_BASE%\CLIMON2\".to_owned(),
                REG_EXPAND_SZ
            ))
        );
        assert_eq!(world.registry.record(), None);
        assert_installed(&world, &world.home());
    }

    pub fn no_modify_path_touches_neither_path_nor_record_and_prints_the_entry() {
        let world = World::new();
        let before = r"C:\one;D:\old";
        world.registry.set_path(REG_SZ, before);
        world.registry.set_record(r"D:\old");
        let outcome = world.install(&["--no-modify-path"]);
        assert_eq!(
            outcome.succeeded().stdout,
            format!(
                "climon2: PATH was not changed. To run c2, add this entry to your user PATH:\n{}\n",
                world.home().display()
            )
        );
        assert_eq!(world.registry.path(), Some((before.to_owned(), REG_SZ)));
        assert_eq!(world.registry.record().as_deref(), Some(r"D:\old"));
        assert_installed(&world, &world.home());
    }

    pub fn a_path_value_that_is_not_a_string_is_left_unchanged() {
        let world = World::new();
        world
            .registry
            .set_path_value(REG_DWORD, &7u32.to_le_bytes());
        world
            .install(&[])
            .failed("the registry value Path has type 4, not a string; it was left unchanged");
        world.registry.set_record(r"D:\old");
        // An unpaired surrogate cannot be read back as text.
        world.registry.set_path_value(REG_SZ, &[0x00, 0xd8, 0, 0]);
        world.install(&[]).failed("invalid utf-16");
        assert_eq!(world.registry.record().as_deref(), Some(r"D:\old"));
    }

    pub fn a_home_that_is_not_unicode_is_not_added_to_path() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        let world = World::new();
        world.registry.set_path(REG_SZ, r"C:\one");
        let mut home: Vec<u16> = world.root().as_os_str().encode_wide().collect();
        home.extend([u16::from(b'\\'), 0xd800]);
        let home = OsString::from_wide(&home);
        let outcome = world.install_os(&[OsString::from("--dir"), home.clone()]);
        outcome.failed("cannot be written to PATH");
        assert_eq!(world.registry.path(), Some((r"C:\one".to_owned(), REG_SZ)));
        assert_eq!(world.registry.record(), None);
        world
            .install_os(&["--no-modify-path".into(), "--dir".into(), home])
            .succeeded();
    }

    pub fn a_registry_failure_fails_the_install() {
        let world = World::new();
        world.registry.set_path(REG_SZ, r"C:\one");
        // Everyone may read the user's environment, and nobody may change it.
        world.registry.secure("Environment", "D:P(A;;KR;;;WD)");
        world.install(&[]).failed("(os error 5)");
        assert_eq!(world.registry.path(), Some((r"C:\one".to_owned(), REG_SZ)));
        assert_eq!(world.registry.record(), None);
    }

    pub fn a_path_change_is_broadcast_to_running_programs() {
        let mut world = World::new();
        let count = world.root().join("broadcasts");
        world.set(crate::broadcast::COUNT, &count);
        // How many times an install with `args` broadcast the change itself.
        let broadcasts = |args: &[&str]| {
            world.install(args).succeeded();
            fs::read_to_string(&count).unwrap()
        };
        assert_eq!(broadcasts(&["--no-modify-path"]), "0");
        assert_eq!(broadcasts(&[]), "1");
        // A reinstall changes nothing, so it broadcasts nothing.
        assert_eq!(broadcasts(&[]), "0");
    }
}
