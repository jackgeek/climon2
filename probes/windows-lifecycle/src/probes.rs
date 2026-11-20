use crate::pty::*;
use crate::util::*;
use serde_json::{json, Value};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicUsize, Ordering};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub fn dispatch(a: &[&str]) {
    let arg = |i: usize| a.get(i).copied().unwrap_or("");
    let num = |i: usize, d: u64| a.get(i).and_then(|s| s.parse().ok()).unwrap_or(d);
    match arg(0) {
        // helpers
        "sleep" => sleep_ms(num(1, 1000)),
        "nop" => {}
        "exit-with" => std::process::exit(parse_u32(arg(1)) as i32),
        "self-report" => emit(json!({"probe": arg(1), "self": self_report()})),
        "child-report" => {
            emit(json!({"probe": "child-report", "self": self_report()}));
            sleep_ms(num(1, 500));
        }
        "spawn-test" => spawn_test(arg(1), &format!("{}", num(2, 500))),
        "heartbeat" => heartbeat(arg(1), num(2, 120)),
        "spawn-heartbeat" => spawn_test("H2", &format!("heartbeat|{}", arg(1))),
        "client" => client(num(1, 30), true),
        "raw-reader" => client(num(1, 30), false),
        "detach" => detach(&a[1..]),
        "windows" => emit(json!({"windows": list_windows(arg(1)).iter().map(|h| *h as usize).collect::<Vec<_>>()})),
        "close-window" => close_window(num(1, 0) as usize),
        "send-keys" => send_keys(num(1, 0) as usize, arg(2)),
        "argv0" => argv0(),
        // probes
        "j1" => j1(),
        "j2" => j2(),
        "j4" => j4(),
        "j4-child" => j4_child(arg(1)),
        "pty-smoke" => pty_smoke(arg(1)),
        "pty-where" => pty_where(arg(1), arg(2)),
        "s1" => s1(),
        "s2" => s2(),
        "s3" => s3(arg(1), num(2, 10)),
        "s4" => s4(),
        "s5" => s5(),
        "s6" => s6(),
        "s7" => s7(arg(1)),
        "c1" => c1(),
        "c2-ping" => c2_ping(arg(1)),
        "c3" => c3(),
        "break-helper" => break_helper(num(1, 0) as u32),
        "c4" => c4(),
        "c5" => c5(),
        "chain" => {
            if !arg(3).is_empty() {
                std::env::set_var("PROBE_CHAIN_DIR", arg(3));
            }
            chain(arg(1), arg(2))
        }
        "x1-conpty" => x1_conpty(),
        "x2" => x2(),
        "x2-parent" => x2_parent(arg(1)),
        "x3" => x3(),
        "x3-runner" => x3_runner(),
        "i1" => i1(),
        "i2" => i2(&a[1..]),
        "i2-open" => i2_open(&a[1..]),
        "i3" => i3(num(1, 1000) as usize),
        "q1" => q1(),
        "q2" => q2(),
        other => {
            eprintln!("unknown subcommand {other:?}");
            std::process::exit(2);
        }
    }
}

fn parse_u32(s: &str) -> u32 {
    if let Some(h) = s.strip_prefix("0x") {
        u32::from_str_radix(h, 16).unwrap()
    } else {
        s.parse::<i64>().unwrap() as u32
    }
}

fn me() -> HANDLE {
    unsafe { GetCurrentProcess() }
}

fn self_in_job() -> bool {
    in_job(me(), null_mut()) == json!(true)
}

// ---------------------------------------------------------------- H1 / H2 / J3

/// Self-report, then the decision-2 spawn: DETACHED_PROCESS, plus breakaway when in a job.
fn spawn_test(probe: &str, child: &str) {
    let rep = self_report();
    let injob = self_in_job();
    let flags = DETACHED_PROCESS | if injob { CREATE_BREAKAWAY_FROM_JOB } else { 0 };
    let cmd = if let Some(f) = child.strip_prefix("heartbeat|") {
        self_cmd(&["heartbeat", f, "120"])
    } else {
        self_cmd(&["child-report", child])
    };
    let nul = nul_handle(true);
    let hl = [nul];
    let r = spawn(&Spawn {
        cmd,
        flags,
        inherit: true,
        handle_list: Some(&hl),
        std: Some([nul, nul, nul]),
        ..Default::default()
    });
    let spawn_v = match &r {
        Ok(p) => json!({"ok": true, "child_pid": p.pid, "child_in_job": p.in_job(null_mut())}),
        Err(e) => e.json(),
    };
    emit(json!({"probe": probe, "self": rep, "spawn_flags": hex(flags), "spawn": spawn_v}));
}

fn heartbeat(file: &str, secs: u64) {
    use std::io::Write;
    for i in 0..secs {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
            let _ = writeln!(f, "{} {} {}", std::process::id(), i, now_ms() as u64);
        }
        sleep_ms(1000);
    }
}

// ---------------------------------------------------------------- H3 client / raw reader

static LOG_EVENTS_EXIT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn log_handler(ev: u32) -> BOOL {
    let name = match ev {
        CTRL_C_EVENT => "CTRL_C_EVENT",
        CTRL_BREAK_EVENT => "CTRL_BREAK_EVENT",
        CTRL_CLOSE_EVENT => "CTRL_CLOSE_EVENT",
        CTRL_LOGOFF_EVENT => "CTRL_LOGOFF_EVENT",
        CTRL_SHUTDOWN_EVENT => "CTRL_SHUTDOWN_EVENT",
        _ => "other",
    };
    emit(json!({"event": name, "ctrl_type": ev}));
    if ev == CTRL_CLOSE_EVENT && LOG_EVENTS_EXIT.load(Ordering::SeqCst) != 0 {
        std::process::exit(129);
    }
    1
}

/// `console == true`: the decision-10 client, logging every INPUT_RECORD.
/// `console == false`: a raw VT reader logging the bytes ReadFile returns (C1, C4).
fn client(secs: u64, console: bool) {
    unsafe {
        let hin = GetStdHandle(STD_INPUT_HANDLE);
        let mut m0 = 0u32;
        let gm = GetConsoleMode(hin, &mut m0);
        let mode = (m0 & !(ENABLE_PROCESSED_INPUT | ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_QUICK_EDIT_MODE))
            | ENABLE_VIRTUAL_TERMINAL_INPUT
            | ENABLE_WINDOW_INPUT
            | ENABLE_EXTENDED_FLAGS;
        let sm = SetConsoleMode(hin, mode);
        let sm_err = last_err();
        let mut m1 = 0u32;
        GetConsoleMode(hin, &mut m1);
        let hr = SetConsoleCtrlHandler(Some(log_handler), 1);
        emit(json!({
            "probe": if console { "client" } else { "raw-reader" },
            "get_mode_ok": gm != 0, "mode_before": hex(m0), "mode_set": hex(mode),
            "set_ok": sm != 0, "set_err": sm_err, "mode_after": hex(m1),
            "handler_ok": hr != 0, "console_window": GetConsoleWindow() as usize,
        }));
        std::thread::spawn(move || {
            sleep_ms(secs * 1000);
            emit(json!({"probe": "client-exit", "reason": "deadline"}));
            std::process::exit(0);
        });
        if console {
            let mut recs: [INPUT_RECORD; 16] = std::mem::zeroed();
            loop {
                let mut n = 0u32;
                if ReadConsoleInputW(hin, recs.as_mut_ptr(), 16, &mut n) == 0 {
                    emit(json!({"read_err": last_err()}));
                    sleep_ms(200);
                    continue;
                }
                for r in &recs[..n as usize] {
                    let v = match r.EventType as u32 {
                        KEY_EVENT => {
                            let k = r.Event.KeyEvent;
                            json!({"rec": "KEY", "down": k.bKeyDown != 0, "vk": hex(k.wVirtualKeyCode as u32),
                                "scan": hex(k.wVirtualScanCode as u32), "ch": hex(k.uChar.UnicodeChar as u32),
                                "ctrl": hex(k.dwControlKeyState), "repeat": k.wRepeatCount})
                        }
                        WINDOW_BUFFER_SIZE_EVENT => json!({"rec": "WINDOW_BUFFER_SIZE"}),
                        FOCUS_EVENT => json!({"rec": "FOCUS", "set": r.Event.FocusEvent.bSetFocus != 0}),
                        MENU_EVENT => json!({"rec": "MENU"}),
                        MOUSE_EVENT => json!({"rec": "MOUSE"}),
                        t => json!({"rec": t}),
                    };
                    emit(v);
                }
            }
        } else {
            let mut buf = [0u8; 256];
            loop {
                let mut n = 0u32;
                let ok = windows_sys::Win32::Storage::FileSystem::ReadFile(
                    hin,
                    buf.as_mut_ptr(),
                    256,
                    &mut n,
                    null_mut(),
                );
                if ok == 0 {
                    emit(json!({"read_err": last_err()}));
                    sleep_ms(200);
                    continue;
                }
                let b = &buf[..n as usize];
                emit(json!({"bytes": b.iter().map(|c| format!("{c:02x}")).collect::<Vec<_>>().join(" "), "esc": escape_bytes(b)}));
            }
        }
    }
}

// ---------------------------------------------------------------- detached host

/// Starts `sub...` as a detached probe host (decision 2 shape: DETACHED_PROCESS, NUL std
/// handles), optionally with CREATE_NEW_PROCESS_GROUP and/or after ignoring Ctrl+C here.
fn detach(a: &[&str]) {
    let mut flags = DETACHED_PROCESS;
    let mut i = 0;
    let mut ignore = false;
    while i < a.len() && a[i].starts_with("--") {
        match a[i] {
            "--newgroup" => flags |= CREATE_NEW_PROCESS_GROUP,
            "--ignore-ctrlc" => ignore = true,
            _ => {}
        }
        i += 1;
    }
    if ignore {
        let r = unsafe { SetConsoleCtrlHandler(None, 1) };
        emit(json!({"probe": "detach", "ignore_ctrlc_ok": r != 0, "err": last_err()}));
    }
    let nul = nul_handle(true);
    let hl = [nul];
    let p = spawn(&Spawn {
        cmd: self_cmd(&a[i..]),
        flags,
        inherit: true,
        handle_list: Some(&hl),
        std: Some([nul, nul, nul]),
        ..Default::default()
    })
    .expect("detach spawn");
    p.wait(INFINITE);
    emit(json!({"probe": "detach", "host": a[i..].join(" "), "flags": hex(flags), "host_exit": hex(p.exit_code())}));
}

// ---------------------------------------------------------------- windows (H2, H3, X1)

fn list_windows(class: &str) -> Vec<HWND> {
    unsafe extern "system" fn cb(h: HWND, l: LPARAM) -> BOOL {
        let st = &mut *(l as *mut (Vec<u16>, Vec<HWND>));
        let mut buf = [0u16; 256];
        let n = GetClassNameW(h, buf.as_mut_ptr(), 256) as usize;
        if buf[..n] == st.0[..st.0.len() - 1] {
            st.1.push(h);
        }
        1
    }
    let mut st = (w(class), Vec::<HWND>::new());
    unsafe { EnumWindows(Some(cb), &mut st as *mut _ as LPARAM) };
    st.1
}

fn close_window(h: usize) {
    let mut pid = 0u32;
    let h = h as HWND;
    unsafe { GetWindowThreadProcessId(h, &mut pid) };
    let ok = unsafe { PostMessageW(h, WM_CLOSE, 0, 0) };
    emit(json!({"probe": "close-window", "hwnd": h as usize, "owner_pid": pid, "owner": proc_name(pid), "ok": ok != 0}));
}

fn key(vk: u16, up: bool, ext: bool) -> INPUT {
    let mut i: INPUT = unsafe { std::mem::zeroed() };
    i.r#type = INPUT_KEYBOARD;
    i.Anonymous.ki = KEYBDINPUT {
        wVk: vk,
        wScan: 0,
        dwFlags: (if up { KEYEVENTF_KEYUP } else { 0 }) | (if ext { KEYEVENTF_EXTENDEDKEY } else { 0 }),
        time: 0,
        dwExtraInfo: 0,
    };
    i
}

/// Synthesised key presses with SendInput, only while `hwnd` is the foreground window.
/// `keys` is a comma list of: ctrl-c, ctrl-break, esc.
fn send_keys(hwnd: usize, keys: &str) {
    let h = hwnd as HWND;
    unsafe {
        // The ALT tap lets a background process take the foreground.
        let alt = [key(VK_MENU, false, false), key(VK_MENU, true, false)];
        SendInput(2, alt.as_ptr(), std::mem::size_of::<INPUT>() as i32);
        let fg = SetForegroundWindow(h);
        sleep_ms(500);
        for k in keys.split(',') {
            if GetForegroundWindow() != h {
                emit(json!({"probe": "send-keys", "abort": "foreground is not the target", "fg": GetForegroundWindow() as usize}));
                return;
            }
            let seq: Vec<INPUT> = match k {
                "ctrl-c" => vec![
                    key(VK_CONTROL, false, false),
                    key(b'C' as u16, false, false),
                    key(b'C' as u16, true, false),
                    key(VK_CONTROL, true, false),
                ],
                "ctrl-break" => vec![
                    key(VK_CONTROL, false, false),
                    key(VK_CANCEL, false, true),
                    key(VK_CANCEL, true, true),
                    key(VK_CONTROL, true, false),
                ],
                "esc" => vec![key(VK_ESCAPE, false, false), key(VK_ESCAPE, true, false)],
                _ => vec![],
            };
            let n = SendInput(seq.len() as u32, seq.as_ptr(), std::mem::size_of::<INPUT>() as i32);
            emit(json!({"probe": "send-keys", "key": k, "sent": n, "set_fg": fg != 0}));
            sleep_ms(700);
        }
    }
}

// ---------------------------------------------------------------- J1 / J2 / J4

fn spawn_suspended_in(jobs: &[HANDLE], cmd: String) -> Proc {
    let p = spawn(&Spawn { cmd, flags: CREATE_SUSPENDED, ..Default::default() }).expect("spawn");
    for &j in jobs {
        let ok = unsafe { AssignProcessToJobObject(j, p.h) };
        assert!(ok != 0, "assign {}", last_err());
    }
    resume(p.pid);
    p
}

fn resume(pid: u32) {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        let mut e: THREADENTRY32 = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        if Thread32First(snap, &mut e) != 0 {
            loop {
                if e.th32OwnerProcessID == pid {
                    let t = OpenThread(THREAD_SUSPEND_RESUME, 0, e.th32ThreadID);
                    ResumeThread(t);
                    CloseHandle(t);
                }
                if Thread32Next(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
}

fn j1() {
    for rep in 0..3 {
        let job = create_job(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
        let p = spawn_suspended_in(&[job], self_cmd(&["spawn-test", &format!("J1#{rep}")]));
        p.wait(10_000);
        emit(json!({"probe": "J1", "rep": rep, "runner_exit": hex(p.exit_code())}));
        unsafe { CloseHandle(job) };
    }
}

fn j2() {
    for rep in 0..3 {
        let outer = create_job(JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE);
        let inner = create_job(JOB_OBJECT_LIMIT_BREAKAWAY_OK);
        let p = spawn_suspended_in(&[outer, inner], self_cmd(&["spawn-test", &format!("J2#{rep}"), "3000"]));
        p.wait(10_000);
        emit(json!({"probe": "J2", "rep": rep, "outer_active": job_active(outer), "inner_active": job_active(inner),
            "outer_pids": job_pids(outer), "inner_pids": job_pids(inner)}));
        unsafe {
            CloseHandle(inner);
            CloseHandle(outer)
        };
    }
}

fn j4() {
    for rep in 0..3 {
        let job = session_job();
        let pidfile = format!("{}\\j4-{}-{rep}.pid", std::env::temp_dir().display(), std::process::id());
        let _ = std::fs::remove_file(&pidfile);
        let jobs = [job];
        let child = spawn(&Spawn {
            cmd: self_cmd(&["j4-child", &pidfile]),
            flags: CREATE_NO_WINDOW,
            jobs: Some(&jobs),
            ..Default::default()
        })
        .expect("j4 child");
        let mut gpid = 0u32;
        for _ in 0..100 {
            if let Ok(s) = std::fs::read_to_string(&pidfile) {
                if let Ok(v) = s.trim().parse() {
                    gpid = v;
                    break;
                }
            }
            sleep_ms(50);
        }
        let g = open_proc(gpid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE | PROCESS_TERMINATE).unwrap();
        let before = json!({"child_in_job": child.in_job(job), "grandchild_in_job_any": in_job(g, null_mut()),
            "grandchild_in_session_job": in_job(g, job)});
        unsafe { CloseHandle(job) };
        sleep_ms(1000);
        let child_alive = child.wait(0) == WAIT_TIMEOUT;
        let g_alive = unsafe { WaitForSingleObject(g, 0) } == WAIT_TIMEOUT;
        emit(json!({"probe": "J4", "rep": rep, "self_in_job": self_in_job(), "before_close": before,
            "after_close": {"child_alive": child_alive, "child_exit": hex(child.exit_code()),
            "grandchild_alive": g_alive, "grandchild_exit": hex(exit_code(g))}}));
        unsafe { TerminateProcess(g, 1) };
    }
}

fn j4_child(pidfile: &str) {
    let flags = DETACHED_PROCESS | CREATE_BREAKAWAY_FROM_JOB;
    let r = spawn(&Spawn { cmd: self_cmd(&["sleep", "30000"]), flags, ..Default::default() });
    match r {
        Ok(p) => {
            emit(json!({"probe": "J4-child", "self": self_report(), "grandchild": p.pid, "grandchild_in_job": p.in_job(null_mut())}));
            std::fs::write(pidfile, p.pid.to_string()).unwrap();
        }
        Err(e) => {
            emit(json!({"probe": "J4-child", "spawn": e.json()}));
            std::fs::write(pidfile, "0").unwrap();
        }
    }
    sleep_ms(30_000);
}

/// Diagnostic for S3/pwsh: type `line` into a shell and report where the processes it starts end up.
fn pty_where(shell: &str, line: &str) {
    let api = load();
    let job = session_job();
    let pty = Pty::new(api);
    let p = pty.spawn(job, shell, None).expect("spawn");
    pty.release();
    sleep_ms(5000);
    pty.write(format!("{line}\r").as_bytes());
    sleep_ms(4000);
    let pings: Vec<Value> = processes()
        .into_iter()
        .filter(|x| x.name.eq_ignore_ascii_case("PING.EXE"))
        .map(|x| {
            let h = open_proc(x.pid, PROCESS_QUERY_LIMITED_INFORMATION).unwrap_or(null_mut());
            json!({"pid": x.pid, "ppid": x.ppid, "parent": proc_name(x.ppid), "in_session_job": in_job(h, job), "in_any_job": in_job(h, null_mut())})
        })
        .collect();
    let pings_h: Vec<(u32, String, HANDLE)> = processes()
        .into_iter()
        .filter(|x| x.name.eq_ignore_ascii_case("PING.EXE"))
        .map(|x| (x.pid, x.name, open_proc(x.pid, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE).unwrap_or(null_mut())))
        .collect();
    emit(json!({"probe": "pty-where", "shell": shell, "leader": p.pid, "line": line,
        "job_pids": job_pids(job).iter().map(|q| format!("{q}:{}", proc_name(*q))).collect::<Vec<_>>(),
        "pings": pings, "tail": escape_bytes(tail(&pty.text(), 300).as_bytes())}));
    // Hang-up: close, 5 s, terminate the job; where are the pings?
    let mut pty = pty;
    pty.close();
    sleep_ms(3000);
    let at3 = status_of(&pings_h);
    sleep_ms(2000);
    terminate_job(job, 0xC000013A);
    sleep_ms(500);
    emit(json!({"probe": "pty-where", "shell": shell, "pings_3s_after_close": at3, "pings_after_terminate": status_of(&pings_h),
        "leader_exit": hex(p.exit_code())}));
    for (_, _, h) in &pings_h {
        unsafe { TerminateProcess(*h, 1) };
    }
    unsafe { CloseHandle(job) };
}

// ---------------------------------------------------------------- S1..S7

fn handle_count(h: HANDLE) -> u32 {
    let mut n = 0u32;
    unsafe { GetProcessHandleCount(h, &mut n) };
    n
}

fn pty_smoke(cmd: &str) {
    let api = load();
    let job = session_job();
    let pty = Pty::new(api);
    let p = pty.spawn(job, if cmd.is_empty() { "cmd.exe /D" } else { cmd }, None);
    let p = match p {
        Ok(p) => p,
        Err(e) => return emit(json!({"probe": "smoke", "spawn": e.json()})),
    };
    let w = p.wait(2000);
    let alive_before_release = w == WAIT_TIMEOUT;
    let rel = pty.release();
    pty.write(b"echo smoke-ok\r");
    let ok = pty.wait_text("smoke-ok\r\n", 3000);
    emit(json!({"probe": "smoke", "alive_2s": alive_before_release, "release_hr": hex(rel as u32), "echo_ok": ok,
        "exit": hex(p.exit_code()), "ended": pty.ended(), "out": escape_bytes(pty.text().as_bytes())}));
    unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
}

fn s1() {
    let api = load();
    emit(json!({"probe": "S1", "host": self_report(), "conpty": conpty_path()}));
    for rep in 0..3 {
        let job = session_job();
        let jobs = [job];
        // The decision-5 shape: inherit TRUE with an empty HANDLE_LIST.
        let z = Pty::new(api);
        let dz = spawn(&Spawn { cmd: "cmd.exe /D".into(), inherit: true, hpc: Some(z.hpc), jobs: Some(&jobs),
            handle_list: Some(&[]), ..Default::default() });
        emit(json!({"probe": "S1", "rep": rep, "shape": "inherit=TRUE + empty HANDLE_LIST",
            "spawn": match &dz { Ok(p) => json!({"ok": true, "pid": p.pid}), Err(e) => e.json() }}));
        drop(z);
        // The fallback every other probe uses: inherit FALSE, no HANDLE_LIST.
        let a = Pty::new(api);
        let pa = a.spawn(job, "cmd.exe /D", None).expect("fallback spawn");
        // Ten extra inheritable handles in the parent.
        let mut extra = vec![];
        for _ in 0..10 {
            let sa = windows_sys::Win32::Security::SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<windows_sys::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: null_mut(),
                bInheritHandle: 1,
            };
            extra.push(unsafe { CreateEventW(&sa, 1, 0, null()) });
        }
        let b = Pty::new(api);
        let pb = b.spawn(job, "cmd.exe /D", None).expect("control");
        // A third, inherit TRUE without HANDLE_LIST, shows what leaks when nothing restricts it.
        let c = Pty::new(api);
        let pc = spawn(&Spawn { cmd: "cmd.exe /D".into(), inherit: true, hpc: Some(c.hpc), jobs: Some(&jobs), std: Some([null_mut(); 3]), ..Default::default() })
            .expect("no-list");
        // A fourth, inherit TRUE with a one-entry HANDLE_LIST naming one of the extra handles.
        let d = Pty::new(api);
        let one = [extra[0]];
        let pd = spawn(&Spawn { cmd: "cmd.exe /D".into(), inherit: true, hpc: Some(d.hpc), jobs: Some(&jobs),
            handle_list: Some(&one), std: Some([null_mut(); 3]), ..Default::default() })
            .expect("one-entry list");
        sleep_ms(1500);
        emit(json!({"probe": "S1", "rep": rep, "shape": "fallback inherit=FALSE, no HANDLE_LIST",
            "cmd_in_session_job": pa.in_job(job), "cmd_in_any_job": pa.in_job(null_mut()),
            "handles_cmd_before_extra": handle_count(pa.h),
            "handles_control_after_10_extra_inherit_false": handle_count(pb.h),
            "handles_control_after_10_extra_inherit_true_no_list": handle_count(pc.h),
            "handles_control_after_10_extra_one_entry_list": handle_count(pd.h),
            "job_pids": job_pids(job).iter().map(|p| format!("{p}:{}", proc_name(*p))).collect::<Vec<_>>()}));
        for h in extra {
            unsafe { CloseHandle(h) };
        }
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
}

fn s2() {
    let api = load();
    for rep in 0..3 {
        for variant in ["plain", "bg-child"] {
            let job = session_job();
            let mut pty = Pty::new(api);
            let p = pty.spawn(job, "cmd.exe /D", None).expect("spawn");
            let rel = pty.release();
            pty.wait_text(">", 5000);
            if variant == "bg-child" {
                pty.write(b"start /b ping -n 12 localhost\r");
                sleep_ms(1500);
            }
            pty.write(b"exit\r");
            p.wait(10_000);
            let exit_t = now_ms();
            let others = snapshot_job(job);
            // Watch for the pipe to break.
            let mut broke = None;
            let end = now_ms() + 15_000.0;
            while now_ms() < end {
                if let Some((t, e)) = pty.ended() {
                    broke = Some((t - exit_t, e));
                    break;
                }
                sleep_ms(10);
            }
            emit(json!({"probe": "S2", "rep": rep, "variant": variant, "release_hr": hex(rel as u32),
                "leader_exit": hex(p.exit_code()), "others_at_leader_exit": status_of(&others),
                "pipe_ended_ms_after_leader_exit": broke.map(|b| b.0), "pipe_err": broke.map(|b| b.1),
                "others_at_end": status_of(&others), "job_active_at_end": job_active(job)}));
            pty.closed = true; // already released; avoid a close on drop after the pipe broke
            if broke.is_none() {
                pty.closed = false;
            }
            unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
        }
    }
}

fn shell_cmd(shell: &str) -> (&'static str, Option<&'static str>) {
    // (command line, how to start a background ping from it)
    match shell {
        "cmd" => ("cmd.exe /D", Some("start /b ping -t localhost\r")),
        "powershell" => ("powershell.exe -NoProfile", Some("Start-Process -NoNewWindow ping -ArgumentList '-t','localhost'\r")),
        "pwsh" => ("pwsh.exe -NoProfile", Some("Start-Process -NoNewWindow ping -ArgumentList '-t','localhost'\r")),
        "vim" => (r#""C:\Program Files\Vim\vim.exe""#, None),
        "edit" => ("edit.exe", None),
        _ => panic!("unknown shell"),
    }
}

fn s3(shell: &str, reps: u64) {
    let api = load();
    let warm: u64 = std::env::var("PROBE_SHELL_WAIT").ok().and_then(|s| s.parse().ok()).unwrap_or(2000);
    let (cmd, bg) = shell_cmd(shell);
    // Part 1: close without terminating; three runs.
    for rep in 0..3 {
        let job = session_job();
        let mut pty = Pty::new(api);
        let p = match pty.spawn(job, cmd, None) {
            Ok(p) => p,
            Err(e) => {
                emit(json!({"probe": "S3", "shell": shell, "spawn": e.json()}));
                return;
            }
        };
        pty.release();
        sleep_ms(warm);
        if let Some(b) = bg {
            pty.write(b.as_bytes());
            sleep_ms(1500);
        }
        let procs = snapshot_job(job);
        let t0 = now_ms();
        let close_ms = pty.close();
        let mut gone: Vec<Value> = vec![];
        let mut pending: Vec<_> = procs.iter().collect();
        let end = t0 + 15_000.0;
        while !pending.is_empty() && now_ms() < end {
            pending.retain(|(pid, name, h)| {
                if h.is_null() || unsafe { WaitForSingleObject(*h, 0) } == WAIT_OBJECT_0 {
                    gone.push(json!({"pid": pid, "name": name, "exit": hex(exit_code(*h)), "ms_after_close": now_ms() - t0,
                        "leader": *pid == p.pid}));
                    false
                } else {
                    true
                }
            });
            sleep_ms(5);
        }
        emit(json!({"probe": "S3", "part": "close-only", "shell": shell, "rep": rep, "close_call_ms": close_ms,
            "exited": gone, "still_alive_after_15s": status_of(&procs.iter().filter(|x| pending.iter().any(|y| y.0 == x.0)).cloned().collect::<Vec<_>>()),
            "pipe_end": pty.ended().map(|(t, e)| json!({"ms_after_close": t - t0, "err": e}))}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
    // Part 2: the real sequence.
    for rep in 0..reps {
        let job = session_job();
        let mut pty = Pty::new(api);
        let p = pty.spawn(job, cmd, None).expect("spawn");
        pty.release();
        sleep_ms(warm);
        if let Some(b) = bg {
            pty.write(b.as_bytes());
            sleep_ms(1500);
        }
        let procs = snapshot_job(job);
        let t0 = now_ms();
        let close_ms = pty.close();
        let rem = 5000.0 - (now_ms() - t0);
        if rem > 0.0 {
            sleep_ms(rem as u64);
        }
        let before_term = status_of(&procs);
        let t = terminate_job(job, 0xC000013A);
        sleep_ms(200);
        emit(json!({"probe": "S3", "part": "close-5s-terminate", "shell": shell, "rep": rep, "close_call_ms": close_ms,
            "leader_pid": p.pid, "at_5s_before_terminate": before_term, "terminate": t, "after": status_of(&procs)}));
        unsafe { CloseHandle(job) };
    }
}

fn s4() {
    let api = load();
    for rep in 0..3 {
        let job = session_job();
        let mut pty = Pty::new(api);
        let p = pty.spawn(job, "cmd.exe /D", None).expect("spawn");
        pty.release();
        let jobs = [job];
        let d = spawn(&Spawn {
            cmd: self_cmd(&["--out", &std::env::var("PROBE_OUT").unwrap_or_default(), "client", "60"]),
            flags: DETACHED_PROCESS,
            jobs: Some(&jobs),
            ..Default::default()
        });
        // `client` with no console: it logs any ctrl event its handler receives.
        let d = d.expect("detached");
        sleep_ms(2000);
        let t0 = now_ms();
        pty.close();
        sleep_ms(4900);
        let d_alive_49 = d.wait(0) == WAIT_TIMEOUT;
        sleep_ms(100);
        let d_alive_50 = d.wait(0) == WAIT_TIMEOUT;
        let t = terminate_job(job, 0xC000013A);
        d.wait(2000);
        emit(json!({"probe": "S4", "rep": rep, "detached_in_job": d.in_job(job), "detached_alive_4900ms": d_alive_49,
            "detached_alive_5000ms": d_alive_50, "terminate": t, "detached_exit": hex(d.exit_code()),
            "leader_exit": hex(p.exit_code()), "elapsed_ms": now_ms() - t0}));
        unsafe { CloseHandle(job) };
    }
}

fn s5() {
    let api = load();
    for rep in 0..3 {
        let job = session_job();
        let pty = Pty::new(api);
        let p = pty.spawn(job, "cmd.exe /D", None).expect("spawn");
        pty.release();
        pty.wait_text(">", 5000);
        pty.write(b"start /b ping -n 6 localhost\r");
        sleep_ms(1000);
        let procs = snapshot_job(job);
        let a0 = job_active(job);
        pty.write(b"exit\r");
        let w = p.wait(5000);
        let a1 = job_active(job);
        let t_leader = now_ms();
        let mut a2 = json!(null);
        let mut bg_done_ms = json!(null);
        let end = now_ms() + 10_000.0;
        while now_ms() < end {
            let st = status_of(&procs);
            if st.as_array().unwrap().iter().all(|x| x["alive"] == json!(false)) {
                bg_done_ms = json!(now_ms() - t_leader);
                sleep_ms(50);
                a2 = job_active(job);
                break;
            }
            sleep_ms(20);
        }
        emit(json!({"probe": "S5", "rep": rep, "leader_wait": hex(w), "leader_exit": hex(p.exit_code()),
            "active_before_exit": a0, "active_after_leader_exit": a1, "bg_done_ms_after_leader": bg_done_ms,
            "active_after_bg_exit": a2, "procs": status_of(&procs)}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
}

fn s6() {
    let exe = exe_path().replace("probe.exe", "needs-dll.exe").replace("probe-lpa.exe", "needs-dll.exe");
    for rep in 0..3 {
        let r = spawn(&Spawn { cmd: quote(&exe), flags: CREATE_NO_WINDOW, ..Default::default() });
        match r {
            Ok(p) => {
                p.wait(10_000);
                emit(json!({"probe": "S6", "rep": rep, "create_ok": true, "exit": hex(p.exit_code())}));
            }
            Err(e) => emit(json!({"probe": "S6", "rep": rep, "create": e.json()})),
        }
    }
}

fn s7(base: &str) {
    // `base` is a directory; build a chain under it longer than 260 characters.
    let mut long = base.to_string();
    while long.len() < 300 {
        long.push_str("\\abcdefghijklmnopqrstuvwxyz0123456789");
    }
    let wide = format!("\\\\?\\{long}");
    let mut p = std::path::PathBuf::from(&wide);
    let _ = std::fs::create_dir_all(&p);
    p.push("afile.txt");
    let _ = std::fs::write(&p, b"x");
    let lp = unsafe { windows_sys::Win32::System::Environment::GetCommandLineW() }; // keep import used
    let _ = lp;
    let cases = [
        ("control-short-existing", base.to_string()),
        ("long", long.clone()),
        ("long-verbatim", wide.clone()),
        ("nonexistent", format!("{base}\\does-not-exist")),
        ("file", format!("{base}\\afile-short.txt")),
    ];
    let _ = std::fs::write(format!("{base}\\afile-short.txt"), b"x");
    for (name, dir) in cases {
        let r = spawn(&Spawn { cmd: "cmd.exe /D /C cd".into(), flags: CREATE_NO_WINDOW, cwd: Some(&dir), ..Default::default() });
        let v = match r {
            Ok(p) => {
                p.wait(5000);
                json!({"ok": true, "exit": hex(p.exit_code())})
            }
            Err(e) => e.json(),
        };
        emit(json!({"probe": "S7", "exe": exe_path(), "case": name, "len": dir.len(), "result": v}));
    }
}

// ---------------------------------------------------------------- C1..C5

fn c1() {
    let api = load();
    for rep in 0..3 {
        let job = session_job();
        let pty = Pty::new(api);
        let p = pty.spawn(job, "ping -t localhost", None).expect("spawn");
        pty.release();
        sleep_ms(2500);
        let wr = pty.write(&[0x03]);
        let t0 = now_ms();
        let w = p.wait(5000);
        emit(json!({"probe": "C1", "part": "ping", "rep": rep, "write": wr, "ping_stopped": w == WAIT_OBJECT_0,
            "ms": now_ms() - t0, "exit": hex(p.exit_code()), "tail": escape_bytes(tail(&pty.text(), 200).as_bytes())}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
    for rep in 0..3 {
        let job = session_job();
        let pty = Pty::new(api);
        let p = pty.spawn(job, &self_cmd(&["raw-reader", "6"]), None).expect("spawn");
        pty.release();
        sleep_ms(2000);
        let wr = pty.write(&[0x03]);
        p.wait(10_000);
        emit(json!({"probe": "C1", "part": "raw-reader", "rep": rep, "write": wr, "reader_exit": hex(p.exit_code())}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
}

fn tail(s: &str, n: usize) -> String {
    let c: Vec<char> = s.chars().collect();
    c[c.len().saturating_sub(n)..].iter().collect()
}

/// C2: run as a detached host (see `detach`); `repair` calls SetConsoleCtrlHandler(NULL, FALSE) first.
fn c2_ping(variant: &str) {
    let api = load();
    let repair = if variant.ends_with("repair") {
        let r = unsafe { SetConsoleCtrlHandler(None, 0) };
        json!({"ret": r, "err": last_err()})
    } else {
        json!(null)
    };
    for rep in 0..3 {
        let job = session_job();
        let pty = Pty::new(api);
        let p = pty.spawn(job, "ping -t localhost", None).expect("spawn");
        pty.release();
        sleep_ms(2500);
        pty.write(&[0x03]);
        let w = p.wait(4000);
        emit(json!({"probe": "C2", "variant": variant, "rep": rep, "repair_call": repair, "ping_stopped": w == WAIT_OBJECT_0,
            "exit": if w == WAIT_OBJECT_0 { json!(hex(p.exit_code())) } else { json!(null) }}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
}

fn c3() {
    let api = load();
    // ping -t answers Ctrl+Break by printing "Control-Break" and carrying on, so a second target
    // with the default handler (this probe sleeping) shows a process actually ending.
    let sleeper = format!("{}\r", self_cmd(&["sleep", "60000"]));
    for rep in 0..3 {
        for (target, line) in [("ping", "ping -t localhost\r"), ("default-handler", sleeper.as_str())] {
        for inside in [true, false] {
            let job = session_job();
            let pty = Pty::new(api);
            let leader = pty.spawn(job, "cmd.exe /D", None).expect("spawn");
            pty.release();
            pty.wait_text(">", 5000);
            pty.write(line.as_bytes());
            sleep_ms(2500);
            let procs = snapshot_job(job);
            let jobs = [job];
            let h = spawn(&Spawn {
                cmd: self_cmd(&["break-helper", &leader.pid.to_string()]),
                flags: DETACHED_PROCESS | if inside { 0 } else { CREATE_BREAKAWAY_FROM_JOB },
                jobs: if inside { Some(&jobs) } else { None },
                ..Default::default()
            })
            .expect("helper");
            h.wait(5000);
            sleep_ms(1000);
            let after = status_of(&procs);
            let marker = format!("probe-alive-{rep}");
            pty.write(format!("echo {marker}\r").as_bytes());
            let alive = pty.wait_text(&format!("\n{marker}"), 3000) || pty.wait_text(&format!("{marker}\r\n"), 100);
            emit(json!({"probe": "C3", "rep": rep, "target": target, "helper_in_session_job": inside, "helper_exit": hex(h.exit_code()),
                "helper_was_in_job": h.in_job(job), "procs_after_break": after, "shell_answers_afterwards": alive,
                "tail": escape_bytes(tail(&pty.text(), 300).as_bytes())}));
            unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
        }
        }
    }
}

unsafe extern "system" fn true_handler(_ev: u32) -> BOOL {
    1
}

/// Exit code: 0 on success; otherwise the step number * 0x100 + low byte of GetLastError.
fn break_helper(pid: u32) {
    unsafe {
        let f0 = FreeConsole();
        let a = AttachConsole(pid);
        let ae = last_err();
        let hreg = SetConsoleCtrlHandler(Some(true_handler), 1);
        let g = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, 0);
        let ge = last_err();
        sleep_ms(200);
        let f = FreeConsole();
        emit(json!({"probe": "C3-helper", "free_first": f0, "attach": a, "attach_err": ae, "handler": hreg,
            "generate": g, "generate_err": ge, "free": f, "self_in_job": in_job(me(), null_mut())}));
        let code = if a == 0 { 0x100 + (ae & 0xff) } else if g == 0 { 0x200 + (ge & 0xff) } else { 0 };
        std::process::exit(code as i32);
    }
}

fn c4() {
    let api = load();
    for rep in 0..3 {
        for injected in [true, false] {
            let job = session_job();
            let pty = Pty::new(api);
            let p = pty.spawn(job, &self_cmd(&["raw-reader", "8"]), None).expect("spawn");
            pty.release();
            sleep_ms(1500);
            emit(json!({"probe": "C4", "rep": rep, "injected": injected, "step": "start", "reader": p.pid}));
            if injected {
                let w = pty.write(b"\x1b[3;70;0;1;8;1_");
                emit(json!({"probe": "C4", "rep": rep, "injected": injected, "step": "inject", "write": w}));
                sleep_ms(800);
            }
            let w = pty.write(b"\x1b");
            emit(json!({"probe": "C4", "rep": rep, "injected": injected, "step": "lone-esc", "write": w}));
            sleep_ms(800);
            let w = pty.write(b"a");
            emit(json!({"probe": "C4", "rep": rep, "injected": injected, "step": "a", "write": w}));
            sleep_ms(800);
            let w = pty.write(b"\x1b[1;5R");
            emit(json!({"probe": "C4", "rep": rep, "injected": injected, "step": "csi-1;5R", "write": w}));
            sleep_ms(800);
            emit(json!({"probe": "C4", "rep": rep, "injected": injected, "step": "end"}));
            unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
        }
    }
}

fn c5() {
    let api = load();
    for rep in 0..3 {
        let job = session_job();
        let pty = Pty::new(api);
        pty.spawn(job, "cmd.exe /D", None).expect("spawn");
        pty.release();
        sleep_ms(2000);
        let first = {
            let d = pty.drain.lock().unwrap();
            let mut v = vec![];
            for (_, c) in &d.chunks {
                v.extend_from_slice(c);
            }
            v
        };
        let has = String::from_utf8_lossy(&first).contains("\x1b[?9001h");
        pty.write(b"echo c5-works\r");
        let works = pty.wait_text("c5-works\r\n", 3000);
        emit(json!({"probe": "C5", "rep": rep, "first_bytes": escape_bytes(&first[..first.len().min(400)]),
            "has_9001h": has, "has_1004h": String::from_utf8_lossy(&first).contains("\x1b[?1004h"), "session_works_unanswered": works}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
    }
}

// ---------------------------------------------------------------- X1..X3

static CHILD: AtomicUsize = AtomicUsize::new(0);
static ROLE: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn chain_handler(ev: u32) -> BOOL {
    let role = ROLE.load(Ordering::SeqCst) as u8 as char;
    emit(json!({"chain": role.to_string(), "event": ev, "phase": "enter"}));
    match ev {
        CTRL_C_EVENT | CTRL_BREAK_EVENT => 1,
        _ => {
            if role == 'c' {
                emit(json!({"chain": "c", "event": ev, "phase": "exit", "code": 129}));
                std::process::exit(129);
            }
            let h = CHILD.load(Ordering::SeqCst) as HANDLE;
            let w = WaitForSingleObject(h, 3000);
            let code = if w == WAIT_OBJECT_0 { exit_code(h) } else { 129 };
            emit(json!({"chain": role.to_string(), "event": ev, "phase": "exit", "child_wait": hex(w), "code": hex(code)}));
            std::process::exit(code as i32);
        }
    }
}

/// A -> B -> C sharing one console. mode "close" (X1) or "x3".
fn chain(role: &str, mode: &str) {
    let r = role.as_bytes()[0];
    ROLE.store(r as usize, Ordering::SeqCst);
    if let Ok(dir) = std::env::var("PROBE_CHAIN_DIR") {
        std::env::set_var("PROBE_OUT", format!("{dir}\\{role}.jsonl"));
    }
    let x3_c = mode == "x3" && role == "c";
    if !x3_c {
        unsafe { SetConsoleCtrlHandler(Some(chain_handler), 1) };
    }
    emit(json!({"chain": role, "mode": mode, "phase": "start", "self_in_job": self_in_job()}));
    if role == "c" {
        sleep_ms(120_000);
        return;
    }
    let next = if role == "a" { "b" } else { "c" };
    let p = spawn(&Spawn { cmd: self_cmd(&["chain", next, mode]), ..Default::default() }).expect("chain spawn");
    CHILD.store(p.h as usize, Ordering::SeqCst);
    emit(json!({"chain": role, "phase": "spawned", "child": p.pid}));
    p.wait(INFINITE);
    let code = p.exit_code();
    emit(json!({"chain": role, "phase": "child-exited", "code": hex(code)}));
    std::process::exit(code as i32);
}

fn chain_dir(tag: &str) -> String {
    let d = format!("C:\\probes\\out\\chain-{tag}-{}", std::process::id());
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn read_chain(dir: &str) -> Value {
    let mut m = serde_json::Map::new();
    for r in ["a", "b", "c"] {
        let s = std::fs::read_to_string(format!("{dir}\\{r}.jsonl")).unwrap_or_default();
        let v: Vec<Value> = s.lines().filter_map(|l| serde_json::from_str(l).ok()).collect();
        m.insert(r.into(), json!(v));
    }
    Value::Object(m)
}

fn x1_conpty() {
    let api = load();
    for rep in 0..3 {
        let dir = chain_dir(&format!("x1-{rep}"));
        std::env::set_var("PROBE_CHAIN_DIR", &dir);
        let job = session_job();
        let mut pty = Pty::new(api);
        let a = pty.spawn(job, &self_cmd(&["chain", "a", "close"]), None).expect("spawn");
        pty.release();
        sleep_ms(2000);
        let procs = snapshot_job(job);
        let t0 = now_ms();
        let cm = pty.close();
        let w = a.wait(10_000);
        emit(json!({"probe": "X1", "host": "bundled-conpty", "rep": rep, "close_ms": cm, "close_t_ms": t0,
            "a_wait": hex(w), "a_exit_seen_by_host": hex(a.exit_code()), "a_ms_after_close": now_ms() - t0,
            "procs": status_of(&procs), "logs": read_chain(&dir)}));
        unsafe { TerminateJobObject(job, 1); CloseHandle(job) };
        std::env::remove_var("PROBE_CHAIN_DIR");
    }
}

fn x2() {
    for code in ["0xC000013A", "259", "0xFFFFFFFF"] {
        for rep in 0..3 {
            let p = spawn(&Spawn { cmd: self_cmd(&["x2-parent", code]), flags: CREATE_NO_WINDOW, ..Default::default() }).unwrap();
            p.wait(10_000);
            emit(json!({"probe": "X2", "rep": rep, "child_code": code, "grandparent_sees": hex(p.exit_code()),
                "roundtrip": p.exit_code() == parse_u32(code)}));
        }
    }
}

fn x2_parent(code: &str) {
    let p = spawn(&Spawn { cmd: self_cmd(&["exit-with", code]), ..Default::default() }).unwrap();
    p.wait(INFINITE);
    let c = p.exit_code();
    std::process::exit(c as i32);
}

/// X3: a fresh conhost console (CREATE_NEW_CONSOLE) hosting runner -> A -> B -> C.
fn x3() {
    for rep in 0..3 {
        let dir = chain_dir(&format!("x3-{rep}"));
        std::env::set_var("PROBE_CHAIN_DIR", &dir);
        let p = spawn(&Spawn { cmd: self_cmd(&["x3-runner"]), flags: CREATE_NEW_CONSOLE, ..Default::default() }).unwrap();
        p.wait(20_000);
        emit(json!({"probe": "X3", "rep": rep, "runner_exit": hex(p.exit_code()), "logs": read_chain(&dir)}));
        std::env::remove_var("PROBE_CHAIN_DIR");
    }
}

fn x3_runner() {
    let dir = std::env::var("PROBE_CHAIN_DIR").unwrap();
    std::env::set_var("PROBE_OUT", format!("{dir}\\runner.jsonl"));
    unsafe { SetConsoleCtrlHandler(Some(true_handler), 1) };
    let a = spawn(&Spawn { cmd: self_cmd(&["chain", "a", "x3"]), ..Default::default() }).unwrap();
    sleep_ms(2000);
    let g = unsafe { GenerateConsoleCtrlEvent(CTRL_C_EVENT, 0) };
    let ge = last_err();
    let w = a.wait(10_000);
    emit(json!({"probe": "X3-runner", "generate": g, "err": ge, "a_wait": hex(w), "a_exit": hex(a.exit_code())}));
    let _ = std::fs::write(format!("{dir}\\runner-done"), "");
}

// ---------------------------------------------------------------- I1..I3

fn ctime(h: HANDLE) -> Value {
    let mut c = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut e, mut k, mut u) = (c, c, c);
    let ok = unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    if ok == 0 {
        return json!({"ok": false, "err": last_err()});
    }
    let v = ((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64;
    json!({"filetime": v, "epoch_us": (v - 116_444_736_000_000_000) / 10})
}

fn ctime_raw(h: HANDLE) -> u64 {
    ctime(h)["filetime"].as_u64().unwrap_or(0)
}

fn enable_priv(name: &str) -> Value {
    use windows_sys::Win32::Security::*;
    unsafe {
        let mut tok = null_mut();
        OpenProcessToken(me(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok);
        let mut tp: TOKEN_PRIVILEGES = std::mem::zeroed();
        tp.PrivilegeCount = 1;
        LookupPrivilegeValueW(null(), w(name).as_ptr(), &mut tp.Privileges[0].Luid);
        tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
        let ok = AdjustTokenPrivileges(tok, 0, &tp, 0, null_mut(), null_mut());
        json!({"ok": ok != 0, "err": last_err()})
    }
}

fn i1() {
    use windows_sys::Win32::System::SystemInformation::*;
    let pv = enable_priv("SeSystemtimePrivilege");
    let p = spawn(&Spawn { cmd: self_cmd(&["sleep", "60000"]), flags: CREATE_NO_WINDOW, ..Default::default() }).unwrap();
    let start = std::time::Instant::now();
    let to_ft = |st: &windows_sys::Win32::Foundation::SYSTEMTIME| {
        let mut ft = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        unsafe { windows_sys::Win32::System::Time::SystemTimeToFileTime(st, &mut ft) };
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    };
    let set = |v: u64| unsafe {
        let ft = FILETIME { dwLowDateTime: v as u32, dwHighDateTime: (v >> 32) as u32 };
        let mut st = std::mem::zeroed();
        windows_sys::Win32::System::Time::FileTimeToSystemTime(&ft, &mut st);
        let ok = SetSystemTime(&st);
        json!({"ok": ok != 0, "err": last_err()})
    };
    let mut st0 = unsafe { std::mem::zeroed() };
    unsafe { GetSystemTime(&mut st0) };
    let orig = to_ft(&st0);
    let hour = 36_000_000_000u64;
    let r0 = ctime(p.h);
    let s1 = set(orig + hour);
    let r1 = ctime(p.h);
    let s2 = set(orig - hour);
    let r2 = ctime(p.h);
    let elapsed = start.elapsed().as_nanos() as u64 / 100;
    let s3 = set(orig + elapsed);
    let r3 = ctime(p.h);
    emit(json!({"probe": "I1", "priv": pv, "before": r0, "set_plus_1h": s1, "after_plus_1h": r1, "set_minus_2h": s2,
        "after_minus_2h": r2, "restore": s3, "after_restore": r3, "unchanged": r0 == r1 && r1 == r2 && r2 == r3}));
    unsafe { TerminateProcess(p.h, 0) };
}

fn open_report(pid: u32) -> Value {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE, 0, pid);
        if h.is_null() {
            return json!({"open": false, "err": last_err()});
        }
        let w = WaitForSingleObject(h, 0);
        let v = json!({"open": true, "wait0": hex(w), "exit": hex(exit_code(h)), "ctime": ctime(h)});
        CloseHandle(h);
        v
    }
}

/// I2 targets measured from this process. Extra args: `name=pid` pairs measured too.
fn i2(extra: &[&str]) {
    let all: std::collections::HashSet<u32> = processes().iter().map(|p| p.pid).collect();
    let mut never = 4_000_000u32;
    while all.contains(&never) {
        never += 4;
    }
    let mut cases = vec![("never-existed", never), ("pid0", 0), ("pid4", 4)];
    // Exited and fully released.
    let p = spawn(&Spawn { cmd: self_cmd(&["nop"]), flags: CREATE_NO_WINDOW, ..Default::default() }).unwrap();
    p.wait(INFINITE);
    let released = p.pid;
    unsafe { CloseHandle(p.h) };
    sleep_ms(200);
    cases.push(("exited-released", released));
    // Exited while another handle is held.
    let q = spawn(&Spawn { cmd: self_cmd(&["nop"]), flags: CREATE_NO_WINDOW, ..Default::default() }).unwrap();
    q.wait(INFINITE);
    cases.push(("exited-handle-held", q.pid));
    for s in extra {
        let (n, p) = s.split_once('=').unwrap();
        cases.push((Box::leak(n.to_string().into_boxed_str()), p.parse().unwrap()));
    }
    for (name, pid) in cases {
        emit(json!({"probe": "I2", "observer_elevated": elevated(), "observer_il": integrity_rid(), "case": name,
            "target_pid": pid, "target_name": proc_name(pid), "result": open_report(pid)}));
    }
}

fn i2_open(a: &[&str]) {
    for s in a {
        let (n, p) = s.split_once('=').unwrap();
        let pid: u32 = p.parse().unwrap();
        emit(json!({"probe": "I2", "observer_elevated": elevated(), "observer_il": integrity_rid(), "case": n,
            "target_pid": pid, "target_name": proc_name(pid), "result": open_report(pid)}));
    }
}

fn i3(n: usize) {
    // Created suspended and never resumed, so the loop is as tight as CreateProcessW allows.
    let mut v = Vec::with_capacity(n);
    let t0 = now_ms();
    for _ in 0..n {
        let p = spawn(&Spawn { cmd: self_cmd(&["nop"]), flags: CREATE_NO_WINDOW | CREATE_SUSPENDED, ..Default::default() }).unwrap();
        v.push(ctime_raw(p.h));
        unsafe { TerminateProcess(p.h, 0); CloseHandle(p.h) };
    }
    emit(json!({"probe": "I3", "loop_ms": now_ms() - t0}));
    let mut s = v.clone();
    s.sort();
    let mut dups = 0;
    let mut min_nz = u64::MAX;
    let mut diffs = std::collections::BTreeMap::<u64, usize>::new();
    for w2 in s.windows(2) {
        let d = w2[1] - w2[0];
        if d == 0 {
            dups += 1;
        } else {
            min_nz = min_nz.min(d);
        }
        *diffs.entry(d).or_default() += 1;
    }
    let distinct_low_digits: std::collections::BTreeSet<u64> = v.iter().map(|x| x % 10).collect();
    emit(json!({"probe": "I3", "n": n, "duplicates": dups, "min_nonzero_diff_100ns": min_nz,
        "smallest_diffs": diffs.iter().take(10).map(|(d, c)| format!("{d}x{c}")).collect::<Vec<_>>(),
        "last_digit_values": distinct_low_digits, "span_ms": (s[s.len() - 1] - s[0]) as f64 / 10_000.0}));
}

// ---------------------------------------------------------------- Q1 / Q2

fn q1() {
    let dir = "C:\\probes\\q1";
    std::fs::create_dir_all(dir).unwrap();
    let script = format!("{dir}\\echoargs.cmd");
    let mut body = String::from("@echo off\r\necho ALL=[%*]\r\n");
    for i in 1..=9 {
        body.push_str(&format!("echo {i}=[%{i}]\r\n"));
    }
    std::fs::write(&script, body).unwrap();
    let cases: Vec<(&str, Vec<&str>)> = vec![
        ("percent", vec!["a%b", "%PATH%", "%%"]),
        ("caret", vec!["a^b", "^"]),
        ("amp", vec!["a&b", "&echo INJECTED"]),
        ("pipe", vec!["a|b"]),
        ("bang", vec!["a!b", "!PATH!"]),
        ("quote", vec!["a\"b", "\""]),
        ("lt-gt", vec!["a<b", "a>b", ">C:\\probes\\q1\\pwned.txt"]),
        ("parens", vec!["(a)", ")"]),
        ("trailing-backslash", vec!["a\\", "a b\\"]),
        ("spaces", vec!["a b", " ", "  lead"]),
        ("empty", vec!["", "x", ""]),
        ("mixed", vec!["\"%&|^!<>()\\\""]),
    ];
    let _ = std::fs::remove_file("C:\\probes\\q1\\pwned.txt");
    for (name, args) in cases {
        let out = std::process::Command::new(&script).args(&args).output();
        let v = match out {
            Ok(o) => json!({"status": o.status.code(), "stdout": String::from_utf8_lossy(&o.stdout), "stderr": String::from_utf8_lossy(&o.stderr)}),
            Err(e) => json!({"error": e.to_string(), "kind": format!("{:?}", e.kind())}),
        };
        emit(json!({"probe": "Q1", "case": name, "args": args, "result": v}));
    }
    emit(json!({"probe": "Q1", "case": "redirect-file-created", "pwned_exists": std::path::Path::new("C:\\probes\\q1\\pwned.txt").exists()}));
    for (name, a) in [("CR", "a\rb"), ("LF", "a\nb"), ("NUL", "a\0b")] {
        let out = std::process::Command::new(&script).arg(a).output();
        let v = match out {
            Ok(o) => json!({"accepted": true, "stdout": String::from_utf8_lossy(&o.stdout)}),
            Err(e) => json!({"accepted": false, "error": e.to_string(), "kind": format!("{:?}", e.kind())}),
        };
        emit(json!({"probe": "Q1", "case": name, "result": v}));
    }
}

fn argv0() {
    let mut buf = [0u16; 1024];
    let n = unsafe { windows_sys::Win32::System::LibraryLoader::GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), 1024) };
    emit(json!({"probe": "Q2-child", "argv0": std::env::args().next(), "module": String::from_utf16_lossy(&buf[..n as usize]),
        "current_exe": exe_path()}));
}

fn q2() {
    for first in ["probe", "c2", "\"some other name.exe\""] {
        let r = spawn(&Spawn {
            app: Some(&exe_path()),
            cmd: format!("{first} argv0"),
            flags: CREATE_NO_WINDOW,
            ..Default::default()
        });
        match r {
            Ok(p) => {
                p.wait(5000);
            }
            Err(e) => emit(json!({"probe": "Q2", "first": first, "spawn": e.json()})),
        }
    }
}

#[allow(dead_code)]
fn unused(_: *const c_void) {}
