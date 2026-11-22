//! #315 / ADR 0002 / docs/windows-lifecycle.md: the real `c2.exe` hands off
//! to the current version's CLI by spawning it, waiting and exiting with its
//! raw status, and keeps running through console events while the CLI does.
//!
//! The companion CLI is this test binary, copied into a version folder beside
//! a `role` file that says what it does; a binary with no `role` beside it runs
//! the tests. Console events are tested in a pseudoconsole of their own, so
//! they never reach the process running the tests.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() {
    if let Some(role) = companion::role() {
        companion::act(&role);
    }
    // The ignore flag is inherited. Clear any this run was given, so the test
    // sees whether the launcher passes one on.
    assert_ne!(unsafe { win32::SetConsoleCtrlHandler(None, 0) }, 0);
    harness::run(&[
        (
            "arguments_stdio_and_raw_statuses_round_trip",
            tests::arguments_stdio_and_raw_statuses_round_trip,
        ),
        (
            "a_missing_current_is_retried",
            tests::a_missing_current_is_retried,
        ),
        (
            "a_denied_current_is_retried",
            tests::a_denied_current_is_retried,
        ),
        (
            "a_current_missing_for_a_second_fails",
            tests::a_current_missing_for_a_second_fails,
        ),
        (
            "malformed_current_and_missing_cli_are_own_failures",
            tests::malformed_current_and_missing_cli_are_own_failures,
        ),
        (
            "ctrl_c_and_ctrl_break_reach_the_cli_not_the_launcher",
            tests::ctrl_c_and_ctrl_break_reach_the_cli_not_the_launcher,
        ),
        (
            "on_close_the_launcher_exits_with_the_cli_status",
            tests::on_close_the_launcher_exits_with_the_cli_status,
        ),
        (
            "on_close_the_launcher_gives_up_after_three_seconds",
            tests::on_close_the_launcher_gives_up_after_three_seconds,
        ),
    ]);
}

#[cfg(windows)]
mod win32 {
    use std::ffi::c_void;

    pub type Handle = *mut c_void;
    pub type Bool = i32;
    pub const CTRL_C_EVENT: u32 = 0;
    pub const CTRL_BREAK_EVENT: u32 = 1;
    pub const CTRL_CLOSE_EVENT: u32 = 2;
    pub const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
    pub const STARTF_USESTDHANDLES: u32 = 0x0000_0100;
    pub const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;
    pub const PROCESS_TERMINATE: u32 = 0x0001;

    #[repr(C)]
    pub struct Coord {
        pub x: i16,
        pub y: i16,
    }

    #[repr(C)]
    pub struct StartupInfoW {
        pub cb: u32,
        pub reserved: *mut u16,
        pub desktop: *mut u16,
        pub title: *mut u16,
        pub x: u32,
        pub y: u32,
        pub x_size: u32,
        pub y_size: u32,
        pub x_count_chars: u32,
        pub y_count_chars: u32,
        pub fill_attribute: u32,
        pub flags: u32,
        pub show_window: u16,
        pub reserved2_size: u16,
        pub reserved2: *mut u8,
        pub std_input: Handle,
        pub std_output: Handle,
        pub std_error: Handle,
    }

    #[repr(C)]
    pub struct StartupInfoExW {
        pub startup_info: StartupInfoW,
        pub attribute_list: *mut c_void,
    }

    #[repr(C)]
    pub struct ProcessInformation {
        pub process: Handle,
        pub thread: Handle,
        pub process_id: u32,
        pub thread_id: u32,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn SetConsoleCtrlHandler(
            handler: Option<unsafe extern "system" fn(u32) -> Bool>,
            add: Bool,
        ) -> Bool;
        pub fn GenerateConsoleCtrlEvent(event: u32, group: u32) -> Bool;
        pub fn FreeConsole() -> Bool;
        pub fn CreatePipe(
            read: *mut Handle,
            write: *mut Handle,
            attributes: *const c_void,
            size: u32,
        ) -> Bool;
        pub fn CreatePseudoConsole(
            size: Coord,
            input: Handle,
            output: Handle,
            flags: u32,
            console: *mut Handle,
        ) -> i32;
        pub fn ClosePseudoConsole(console: Handle);
        pub fn InitializeProcThreadAttributeList(
            list: *mut c_void,
            count: u32,
            flags: u32,
            size: *mut usize,
        ) -> Bool;
        pub fn UpdateProcThreadAttribute(
            list: *mut c_void,
            flags: u32,
            attribute: usize,
            value: *const c_void,
            size: usize,
            previous: *mut c_void,
            returned: *mut usize,
        ) -> Bool;
        pub fn DeleteProcThreadAttributeList(list: *mut c_void);
        pub fn CreateProcessW(
            application: *const u16,
            command_line: *mut u16,
            process_attributes: *const c_void,
            thread_attributes: *const c_void,
            inherit_handles: Bool,
            flags: u32,
            environment: *const c_void,
            directory: *const u16,
            startup: *const StartupInfoExW,
            information: *mut ProcessInformation,
        ) -> Bool;
        pub fn WaitForSingleObject(handle: Handle, milliseconds: u32) -> u32;
        pub fn GetExitCodeProcess(process: Handle, code: *mut u32) -> Bool;
        pub fn OpenProcess(access: u32, inherit: Bool, process_id: u32) -> Handle;
        pub fn TerminateProcess(process: Handle, code: u32) -> Bool;
        pub fn CloseHandle(handle: Handle) -> Bool;
    }
}

/// What the companion CLI does, read from the `role` file beside it.
#[cfg(windows)]
mod companion {
    use crate::win32::*;
    use std::{
        fs,
        io::{Read, Write},
        path::PathBuf,
        sync::atomic::{AtomicU32, Ordering},
        time::{Duration, Instant},
    };

    fn folder() -> PathBuf {
        std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
    }

    pub fn role() -> Option<String> {
        fs::read_to_string(folder().join("role")).ok()
    }

    /// Runs the role and exits; it never returns to the test runner.
    pub fn act(role: &str) -> ! {
        let mut words = role.split_whitespace();
        let status = match words.next() {
            Some("report") => report(words.next().unwrap().parse().unwrap()),
            Some("ctrl") => ctrl(),
            Some("linger") => linger(),
            Some("detached") => detached(),
            other => panic!("unknown companion role {other:?}"),
        };
        std::process::exit(status as i32)
    }

    /// Records its arguments and working directory, copies stdin to stdout,
    /// writes to stderr and exits with `status`.
    fn report(status: u32) -> u32 {
        let mut arguments = Vec::new();
        for argument in std::env::args_os().skip(1) {
            arguments.push(argument.into_string().unwrap());
        }
        let cwd = std::env::current_dir().unwrap();
        fs::write(
            folder().join("report"),
            format!("{}\0{}", cwd.display(), arguments.join("\0")),
        )
        .unwrap();
        let mut input = Vec::new();
        std::io::stdin().read_to_end(&mut input).unwrap();
        std::io::stdout().write_all(&input).unwrap();
        eprint!("err");
        status
    }

    static SEEN: AtomicU32 = AtomicU32::new(0);

    unsafe extern "system" fn count(event: u32) -> Bool {
        SEEN.fetch_or(1 << event, Ordering::SeqCst);
        1
    }

    /// Raises Ctrl+C and Ctrl+Break on its console, then outlives the
    /// launcher's three-second close wait, so a launcher that took either for
    /// a close would exit 129 first. Exits 42 when both reached it.
    fn ctrl() -> u32 {
        let both = 1 << CTRL_C_EVENT | 1 << CTRL_BREAK_EVENT;
        unsafe {
            assert_ne!(SetConsoleCtrlHandler(Some(count), 1), 0);
            assert_ne!(GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0), 0);
            assert_ne!(GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0), 0);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while SEEN.load(Ordering::SeqCst) != both && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        std::thread::sleep(Duration::from_millis(3500));
        let seen = SEEN.load(Ordering::SeqCst);
        fs::write(folder().join("seen"), seen.to_string()).unwrap();
        if seen == both { 42 } else { 43 }
    }

    unsafe extern "system" fn hold(event: u32) -> Bool {
        if event == CTRL_CLOSE_EVENT {
            std::thread::sleep(Duration::from_secs(30));
        }
        1
    }

    /// Survives its console closing until it is ended, after writing its
    /// process ID to `ready`.
    fn linger() -> u32 {
        unsafe { assert_ne!(SetConsoleCtrlHandler(Some(hold), 1), 0) };
        fs::write(folder().join("ready"), std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        0
    }

    /// Leaves its console, so no console event reaches it, and runs until it
    /// is ended, after writing its process ID to `ready`.
    fn detached() -> u32 {
        unsafe { assert_ne!(FreeConsole(), 0) };
        fs::write(folder().join("ready"), std::process::id().to_string()).unwrap();
        std::thread::sleep(Duration::from_secs(60));
        0
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
    use crate::win32::*;
    use std::{
        ffi::c_void,
        fs,
        io::{Read, Write},
        os::windows::{ffi::OsStrExt, io::FromRawHandle},
        path::{Path, PathBuf},
        process::{Child, Command, ExitStatus, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    /// The `c2.exe` built beside this test binary (`<profile>/deps/..`).
    /// `CARGO_BIN_EXE_c2` is fixed when this binary is compiled, so a mutation
    /// worker that reuses a seeded build would run the unmutated launcher.
    fn built_launcher() -> PathBuf {
        let deps = std::env::current_exe()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let launcher = deps.parent().unwrap().join("c2.exe");
        assert!(launcher.is_file(), "no launcher at {}", launcher.display());
        launcher
    }

    struct Layout(PathBuf);

    impl Layout {
        fn new() -> Self {
            let root = std::env::current_dir()
                .unwrap()
                .join("target")
                .join(format!(
                    "launcher tests 雪 {}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&root).unwrap();
            fs::copy(built_launcher(), root.join("c2.exe")).unwrap();
            Self(root)
        }

        fn launcher(&self) -> PathBuf {
            self.0.join("c2.exe")
        }

        /// Installs the companion as `version`'s CLI, doing `role`.
        fn companion(&self, version: &str, role: &str) -> PathBuf {
            let folder = self.0.join(version);
            fs::create_dir_all(&folder).unwrap();
            fs::copy(
                std::env::current_exe().unwrap(),
                folder.join("climon2-cli.exe"),
            )
            .unwrap();
            fs::write(folder.join("role"), role).unwrap();
            folder
        }

        fn current(&self, contents: &[u8]) {
            fs::write(self.0.join("current"), contents).unwrap();
        }

        fn launch(&self) -> Command {
            let mut command = Command::new(self.launcher());
            command
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            command
        }
    }

    impl Drop for Layout {
        fn drop(&mut self) {
            // A companion ended at the close can hold its folder for a moment.
            let deadline = Instant::now() + Duration::from_secs(10);
            while fs::remove_dir_all(&self.0).is_err() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    /// The child's output, or a panic once `limit` passes, so a launcher that
    /// never exits fails the test instead of hanging it.
    fn finish(mut child: Child, limit: Duration) -> (ExitStatus, Vec<u8>, Vec<u8>) {
        let deadline = Instant::now() + limit;
        while child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                child.kill().unwrap();
                panic!("the launcher did not exit within {limit:?}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        child
            .stdout
            .take()
            .unwrap()
            .read_to_end(&mut stdout)
            .unwrap();
        child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        (child.wait().unwrap(), stdout, stderr)
    }

    fn raw(status: ExitStatus) -> u32 {
        status.code().unwrap() as u32
    }

    /// ADR 0002: arguments, cwd and stdio pass through, and the launcher exits
    /// with the CLI's full 32-bit status.
    pub fn arguments_stdio_and_raw_statuses_round_trip() {
        let arguments = [
            "",
            "--flag",
            "space 雪",
            "quote\"d",
            "back\\slash\\",
            "tab\there",
        ];
        for status in [0, 37, 0xC000_013A, 259, 0xFFFF_FFFF] {
            let layout = Layout::new();
            let folder = layout.companion("1.2.3-rc.2+build.7", &format!("report {status}"));
            layout.current(b"1.2.3-rc.2+build.7\r\n");
            let mut child = layout
                .launch()
                .args(arguments)
                .current_dir(&folder)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"input\0bytes")
                .unwrap();
            let (exit, stdout, stderr) = finish(child, Duration::from_secs(20));
            assert_eq!(raw(exit), status);
            assert_eq!(stdout, b"input\0bytes");
            assert_eq!(stderr, b"err");
            let report = fs::read_to_string(folder.join("report")).unwrap();
            let expected = format!("{}\0{}", folder.display(), arguments.join("\0"));
            assert_eq!(report, expected);
        }
    }

    /// Replaces a `current` that cannot be read with a valid one after a
    /// moment, as an installer renaming a new `current` over the old one does.
    fn replaced_later(
        layout: &Layout,
        unreadable: impl FnOnce(&Path),
        readable: impl FnOnce(&Path),
    ) {
        layout.companion("1.0.0", "report 7");
        let current = layout.0.join("current");
        unreadable(&current);
        let child = layout.launch().spawn().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        readable(&current);
        fs::write(&current, "1.0.0").unwrap();
        let (exit, _, stderr) = finish(child, Duration::from_secs(20));
        assert_eq!(raw(exit), 7, "{}", String::from_utf8_lossy(&stderr));
    }

    /// docs/windows-distribution.md: a reader can find `current` missing
    /// while an installer renames a new one over it.
    pub fn a_missing_current_is_retried() {
        replaced_later(&Layout::new(), |_| {}, |_| {});
    }

    /// ADR 0002: access denied is retried too. Reading a directory is denied.
    pub fn a_denied_current_is_retried() {
        replaced_later(
            &Layout::new(),
            |current| fs::create_dir(current).unwrap(),
            |current| fs::remove_dir(current).unwrap(),
        );
    }

    pub fn a_current_missing_for_a_second_fails() {
        let layout = Layout::new();
        layout.companion("1.0.0", "report 7");
        let started = Instant::now();
        let (exit, stdout, stderr) =
            finish(layout.launch().spawn().unwrap(), Duration::from_secs(20));
        assert!(
            started.elapsed() >= Duration::from_secs(1),
            "{:?}",
            started.elapsed()
        );
        assert_own_failure(exit, &stdout, &stderr, "current");
    }

    /// The launcher's own failures, as on Unix: one line and 125.
    pub fn malformed_current_and_missing_cli_are_own_failures() {
        let layout = Layout::new();
        for contents in [&b"1.2"[..], b"1.2.3\r", b"..\\1.2.3", &[0xff]] {
            layout.current(contents);
            let (exit, stdout, stderr) =
                finish(layout.launch().spawn().unwrap(), Duration::from_secs(20));
            assert_own_failure(exit, &stdout, &stderr, "current");
        }
        layout.current(b"1.2.3\n");
        let (exit, stdout, stderr) =
            finish(layout.launch().spawn().unwrap(), Duration::from_secs(20));
        assert_own_failure(exit, &stdout, &stderr, "climon2-cli");
    }

    fn assert_own_failure(exit: ExitStatus, stdout: &[u8], stderr: &[u8], context: &str) {
        let diagnostic = String::from_utf8_lossy(stderr);
        assert_eq!(exit.code(), Some(125), "{diagnostic}");
        assert!(stdout.is_empty());
        assert!(diagnostic.starts_with("climon2: error: "), "{diagnostic}");
        assert_eq!(diagnostic.lines().count(), 1, "{diagnostic}");
        assert!(diagnostic.contains(context), "{diagnostic}");
    }

    /// The launcher started alone in a pseudoconsole, which nothing else in
    /// the test run shares.
    struct Console {
        console: Handle,
        process: Handle,
    }

    impl Console {
        fn start(program: &Path) -> Self {
            let (mut input_read, mut input_write) = (std::ptr::null_mut(), std::ptr::null_mut());
            let (mut output_read, mut output_write) = (std::ptr::null_mut(), std::ptr::null_mut());
            let mut console = std::ptr::null_mut();
            unsafe {
                assert_ne!(
                    CreatePipe(&mut input_read, &mut input_write, std::ptr::null(), 0),
                    0
                );
                assert_ne!(
                    CreatePipe(&mut output_read, &mut output_write, std::ptr::null(), 0),
                    0
                );
                let size = Coord { x: 80, y: 25 };
                assert_eq!(
                    CreatePseudoConsole(size, input_read, output_write, 0, &mut console),
                    0
                );
                CloseHandle(input_read);
                CloseHandle(output_write);
            }
            // The pseudoconsole stalls if nobody reads its output.
            let output = unsafe { fs::File::from_raw_handle(output_read) };
            std::thread::spawn(move || std::io::copy(&mut { output }, &mut std::io::sink()));
            // Its input stays open for as long as the test runs.
            std::mem::forget(unsafe { fs::File::from_raw_handle(input_write) });
            Self {
                console,
                process: spawn_in(console, program),
            }
        }

        /// Closes the pseudoconsole, which sends every process on it a close
        /// event, as closing a terminal window does.
        fn close(&self) {
            let console = self.console as usize;
            std::thread::spawn(move || unsafe { ClosePseudoConsole(console as Handle) });
        }

        /// The launcher's raw status, or a panic once `limit` passes.
        fn status(&self, limit: Duration) -> u32 {
            let waited = unsafe { WaitForSingleObject(self.process, limit.as_millis() as u32) };
            if waited != 0 {
                unsafe { TerminateProcess(self.process, 1) };
                panic!("the launcher did not exit within {limit:?}");
            }
            let mut code = 0;
            assert_ne!(unsafe { GetExitCodeProcess(self.process, &mut code) }, 0);
            code
        }
    }

    fn spawn_in(console: Handle, program: &Path) -> Handle {
        let mut size = 0;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut size) };
        let mut list = vec![0u8; size];
        let list = list.as_mut_ptr().cast::<c_void>();
        let mut command_line: Vec<u16> =
            std::ffi::OsString::from(format!("\"{}\"", program.display()))
                .encode_wide()
                .chain([0])
                .collect();
        let mut information = ProcessInformation {
            process: std::ptr::null_mut(),
            thread: std::ptr::null_mut(),
            process_id: 0,
            thread_id: 0,
        };
        unsafe {
            assert_ne!(InitializeProcThreadAttributeList(list, 1, 0, &mut size), 0);
            assert_ne!(
                UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                    console,
                    std::mem::size_of::<Handle>(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                ),
                0
            );
            let mut startup: StartupInfoExW = std::mem::zeroed();
            startup.startup_info.cb = std::mem::size_of::<StartupInfoExW>() as u32;
            startup.startup_info.flags = STARTF_USESTDHANDLES;
            startup.attribute_list = list;
            let created = CreateProcessW(
                std::ptr::null(),
                command_line.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                0,
                EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                std::ptr::null(),
                &startup,
                &mut information,
            );
            DeleteProcThreadAttributeList(list);
            assert_ne!(created, 0, "{}", std::io::Error::last_os_error());
            CloseHandle(information.thread);
        }
        information.process
    }

    /// The companion's process ID, once it has written it.
    fn ready(folder: &Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Ok(pid) = fs::read_to_string(folder.join("ready"))
                && let Ok(pid) = pid.parse()
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "the companion never started");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Ends the companion with `status`; false if it had already gone.
    fn end(pid: u32, status: u32) -> bool {
        unsafe {
            let process = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if process.is_null() {
                return false;
            }
            let ended = TerminateProcess(process, status) != 0;
            CloseHandle(process);
            ended
        }
    }

    /// docs/windows-lifecycle.md: the launcher handles Ctrl+C and Ctrl+Break
    /// with a handler routine, so it survives them, and the CLI still gets
    /// both, which an inherited ignore flag would stop for Ctrl+C.
    pub fn ctrl_c_and_ctrl_break_reach_the_cli_not_the_launcher() {
        let layout = Layout::new();
        layout.companion("1.0.0", "ctrl");
        layout.current(b"1.0.0");
        let console = Console::start(&layout.launcher());
        let status = console.status(Duration::from_secs(30));
        let seen = fs::read_to_string(layout.0.join("1.0.0").join("seen"));
        assert_eq!(status, 42, "{seen:?}");
    }

    /// On close the launcher waits for the CLI and exits with its status. The
    /// CLI here holds its close event until the test ends it, which is when
    /// this pseudoconsole tells the launcher: the launcher's status must not
    /// lose that race to the close.
    pub fn on_close_the_launcher_exits_with_the_cli_status() {
        let layout = Layout::new();
        let folder = layout.companion("1.0.0", "linger");
        layout.current(b"1.0.0");
        let console = Console::start(&layout.launcher());
        let companion = ready(&folder);
        console.close();
        std::thread::sleep(Duration::from_millis(500));
        assert!(end(companion, 7), "the companion ended before the launcher");
        assert_eq!(console.status(Duration::from_secs(20)), 7);
    }

    /// On close the launcher waits three seconds for the CLI, then exits 129.
    pub fn on_close_the_launcher_gives_up_after_three_seconds() {
        let layout = Layout::new();
        let folder = layout.companion("1.0.0", "detached");
        layout.current(b"1.0.0");
        let console = Console::start(&layout.launcher());
        let companion = ready(&folder);
        let closed = Instant::now();
        console.close();
        let status = console.status(Duration::from_secs(20));
        let waited = closed.elapsed();
        end(companion, 0);
        assert_eq!(status, 129, "{waited:?}");
        assert!(waited >= Duration::from_secs(3), "{waited:?}");
    }
}
