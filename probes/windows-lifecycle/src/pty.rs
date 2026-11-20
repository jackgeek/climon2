//! The bundled ConPTY (conpty.dll + OpenConsole.exe from the NuGet package).
use crate::util::*;
use serde_json::{json, Value};
use std::ptr::{null, null_mut};
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Console::COORD;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::*;

type CreateFn = unsafe extern "system" fn(COORD, H, H, u32, *mut isize) -> i32;
type ReleaseFn = unsafe extern "system" fn(isize) -> i32;
type CloseFn = unsafe extern "system" fn(isize);

#[derive(Clone, Copy)]
pub struct Api {
    create: CreateFn,
    release: ReleaseFn,
    close: CloseFn,
}

pub fn conpty_path() -> String {
    std::env::var("PROBE_CONPTY").unwrap_or_else(|_| r"C:\probes\conpty\conpty.dll".into())
}

pub fn load() -> Api {
    unsafe {
        let m = LoadLibraryW(w(&conpty_path()).as_ptr());
        assert!(!m.is_null(), "LoadLibraryW(conpty.dll) failed {}", last_err());
        let get = |n: &str| {
            let c = std::ffi::CString::new(n).unwrap();
            GetProcAddress(m, c.as_ptr() as *const u8).unwrap_or_else(|| panic!("no export {n}"))
        };
        Api {
            create: std::mem::transmute::<_, CreateFn>(get("ConptyCreatePseudoConsole")),
            release: std::mem::transmute::<_, ReleaseFn>(get("ConptyReleasePseudoConsole")),
            close: std::mem::transmute::<_, CloseFn>(get("ConptyClosePseudoConsole")),
        }
    }
}

#[derive(Default)]
pub struct Drain {
    pub chunks: Vec<(f64, Vec<u8>)>,
    /// (time, GetLastError) when ReadFile on the output pipe failed.
    pub ended: Option<(f64, u32)>,
}

pub struct Pty {
    pub api: Api,
    pub hpc: isize,
    pub input: H,
    pub drain: Arc<Mutex<Drain>>,
    pub closed: bool,
}

struct SendH(usize);
unsafe impl Send for SendH {}

impl Pty {
    pub fn new(api: Api) -> Pty {
        unsafe {
            let (mut in_r, mut in_w, mut out_r, mut out_w) = (null_mut(), null_mut(), null_mut(), null_mut());
            assert!(CreatePipe(&mut in_r, &mut in_w, null(), 0) != 0);
            assert!(CreatePipe(&mut out_r, &mut out_w, null(), 0) != 0);
            let mut hpc = 0isize;
            let hr = (api.create)(COORD { X: 120, Y: 30 }, in_r, out_w, 0, &mut hpc);
            assert!(hr == 0, "ConptyCreatePseudoConsole hr={hr:#x}");
            CloseHandle(in_r);
            CloseHandle(out_w);
            let drain = Arc::new(Mutex::new(Drain::default()));
            let d2 = drain.clone();
            let oh = SendH(out_r as usize);
            std::thread::spawn(move || {
                let oh = oh;
                let h = oh.0 as H;
                let mut buf = vec![0u8; 65536];
                loop {
                    let mut n = 0u32;
                    let ok = ReadFile(h, buf.as_mut_ptr(), buf.len() as u32, &mut n, null_mut());
                    let t = now_ms();
                    if ok == 0 {
                        d2.lock().unwrap().ended = Some((t, last_err()));
                        break;
                    }
                    if n == 0 {
                        continue;
                    }
                    d2.lock().unwrap().chunks.push((t, buf[..n as usize].to_vec()));
                }
                CloseHandle(h);
            });
            Pty { api, hpc, input: in_w, drain, closed: false }
        }
    }

    pub fn write(&self, b: &[u8]) -> Value {
        let mut n = 0u32;
        let ok = unsafe { WriteFile(self.input, b.as_ptr(), b.len() as u32, &mut n, null_mut()) };
        json!({"ok": ok != 0, "n": n, "err": if ok == 0 { last_err() } else { 0 }, "t_ms": now_ms()})
    }

    pub fn release(&self) -> i32 {
        unsafe { (self.api.release)(self.hpc) }
    }

    /// Returns how long ConptyClosePseudoConsole took, in ms.
    pub fn close(&mut self) -> f64 {
        let t = now_ms();
        unsafe { (self.api.close)(self.hpc) };
        self.closed = true;
        now_ms() - t
    }

    pub fn text(&self) -> String {
        let d = self.drain.lock().unwrap();
        let mut v = vec![];
        for (_, c) in &d.chunks {
            v.extend_from_slice(c);
        }
        String::from_utf8_lossy(&v).into_owned()
    }

    pub fn wait_text(&self, needle: &str, ms: u64) -> bool {
        let end = now_ms() + ms as f64;
        while now_ms() < end {
            if self.text().contains(needle) {
                return true;
            }
            sleep_ms(50);
        }
        false
    }

    pub fn ended(&self) -> Option<(f64, u32)> {
        self.drain.lock().unwrap().ended
    }

    /// Start `cmd` in the pseudoconsole and `job`. Decision 5 asks for inherit TRUE with an
    /// empty HANDLE_LIST, which UpdateProcThreadAttribute rejects (S1), so this uses
    /// inherit FALSE and no HANDLE_LIST instead. Without STARTF_USESTDHANDLES (null handles) the
    /// command takes the host's own std handles instead of the pseudoconsole's (pty-smoke).
    pub fn spawn(&self, job: H, cmd: &str, cwd: Option<&str>) -> Result<Proc, SpawnErr> {
        let jobs = [job];
        let std = if std::env::var("PROBE_NO_USESTD").is_ok() { None } else { Some([null_mut(); 3]) };
        spawn(&Spawn {
            cmd: cmd.into(),
            inherit: false,
            cwd,
            hpc: Some(self.hpc),
            jobs: Some(&jobs),
            std,
            ..Default::default()
        })
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        if !self.closed {
            self.close();
        }
        unsafe { CloseHandle(self.input) };
    }
}

pub fn session_job() -> H {
    create_job(JOB_OBJECT_LIMIT_BREAKAWAY_OK | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)
}

pub fn escape_bytes(b: &[u8]) -> String {
    let mut s = String::new();
    for &c in b {
        match c {
            0x1b => s.push_str("\\e"),
            b'\r' => s.push_str("\\r"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7e => s.push(c as char),
            _ => s.push_str(&format!("\\x{c:02x}")),
        }
    }
    s
}

pub fn terminate_job(job: H, code: u32) -> Value {
    let ok = unsafe { TerminateJobObject(job, code) };
    json!({"ok": ok != 0, "err": if ok == 0 { last_err() } else { 0 }})
}

pub fn snapshot_job(job: H) -> Vec<(u32, String, H)> {
    job_pids(job)
        .into_iter()
        .map(|p| {
            let h = open_proc(p, PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE).unwrap_or(null_mut());
            (p, proc_name(p), h)
        })
        .collect()
}

pub fn status_of(list: &[(u32, String, H)]) -> Value {
    json!(list
        .iter()
        .map(|(p, n, h)| {
            let alive = !h.is_null() && unsafe { WaitForSingleObject(*h, 0) } == WAIT_TIMEOUT;
            json!({"pid": p, "name": n, "alive": alive, "exit": if alive || h.is_null() { Value::Null } else { json!(hex(exit_code(*h))) }})
        })
        .collect::<Vec<_>>())
}
