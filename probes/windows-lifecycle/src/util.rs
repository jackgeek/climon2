//! Shared Win32 helpers for the probes. Throwaway measuring code.
#![allow(clippy::missing_safety_doc)]

use serde_json::{json, Value};
use std::ffi::c_void;
use std::io::Write;
use std::ptr::{null, null_mut};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows_sys::Win32::System::Threading::*;

pub type H = HANDLE;

pub fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn last_err() -> u32 {
    unsafe { GetLastError() }
}

pub fn hex(v: u32) -> String {
    format!("0x{v:08X}")
}

/// Wall clock in ms since the Unix epoch, sub-ms precision. Comparable across processes.
pub fn now_ms() -> f64 {
    let mut ft = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    unsafe { GetSystemTimePreciseAsFileTime(&mut ft) };
    let v = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    (v as f64 - 116_444_736_000_000_000f64) / 10_000.0
}

/// One JSON line. Goes to the file named by PROBE_OUT when set, otherwise stdout.
pub fn emit(mut v: Value) {
    if let Value::Object(m) = &mut v {
        m.insert("pid".into(), json!(std::process::id()));
        m.insert("t_ms".into(), json!((now_ms() * 1000.0).round() / 1000.0));
    }
    let line = format!("{v}\n");
    match std::env::var("PROBE_OUT") {
        Ok(p) if !p.is_empty() => {
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                let _ = f.write_all(line.as_bytes());
            }
        }
        _ => {
            let _ = std::io::stdout().write_all(line.as_bytes());
            let _ = std::io::stdout().flush();
        }
    }
}

pub fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

pub fn exe_path() -> String {
    std::env::current_exe().unwrap().to_string_lossy().into_owned()
}

pub fn quote(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        a.to_string()
    } else {
        format!("\"{}\"", a.replace('"', "\\\""))
    }
}

/// Command line that runs this same probe executable with `args`.
pub fn self_cmd(args: &[&str]) -> String {
    let mut s = quote(&exe_path());
    for a in args {
        s.push(' ');
        s.push_str(&quote(a));
    }
    s
}

pub struct Proc {
    pub h: H,
    pub pid: u32,
}
unsafe impl Send for Proc {}

impl Proc {
    pub fn wait(&self, ms: u32) -> u32 {
        unsafe { WaitForSingleObject(self.h, ms) }
    }
    pub fn exit_code(&self) -> u32 {
        exit_code(self.h)
    }
    pub fn in_job(&self, job: H) -> Value {
        in_job(self.h, job)
    }
}

pub fn exit_code(h: H) -> u32 {
    let mut c = 0u32;
    unsafe { GetExitCodeProcess(h, &mut c) };
    c
}

pub fn in_job(h: H, job: H) -> Value {
    let mut b: BOOL = 0;
    let ok = unsafe { IsProcessInJob(h, job, &mut b) };
    if ok == 0 {
        json!({"ok": false, "err": last_err()})
    } else {
        json!(b != 0)
    }
}

pub fn open_proc(pid: u32, access: u32) -> Result<H, u32> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() {
        Err(last_err())
    } else {
        Ok(h)
    }
}

/// Everything CreateProcessW can take that the probes vary.
#[derive(Default)]
pub struct Spawn<'a> {
    pub app: Option<&'a str>,
    pub cmd: String,
    pub flags: u32,
    pub inherit: bool,
    pub cwd: Option<&'a str>,
    pub hpc: Option<isize>,
    pub jobs: Option<&'a [H]>,
    pub handle_list: Option<&'a [H]>,
    pub std: Option<[H; 3]>,
    /// Environment block (UTF-16, double-NUL terminated). None means NULL (inherit).
    pub env: Option<Vec<u16>>,
}

#[derive(Debug)]
pub struct SpawnErr {
    pub step: &'static str,
    pub err: u32,
}

impl SpawnErr {
    pub fn json(&self) -> Value {
        json!({"ok": false, "step": self.step, "err": self.err})
    }
}

pub fn spawn(s: &Spawn) -> Result<Proc, SpawnErr> {
    unsafe {
        let mut nattr = 0u32;
        if s.hpc.is_some() {
            nattr += 1;
        }
        if s.jobs.is_some() {
            nattr += 1;
        }
        if s.handle_list.is_some() {
            nattr += 1;
        }
        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        let mut attr_buf: Vec<u8> = Vec::new();
        let mut flags = s.flags;
        if nattr > 0 {
            let mut size = 0usize;
            InitializeProcThreadAttributeList(null_mut(), nattr, 0, &mut size);
            attr_buf = vec![0u8; size];
            let list = attr_buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
            if InitializeProcThreadAttributeList(list, nattr, 0, &mut size) == 0 {
                return Err(SpawnErr { step: "InitializeProcThreadAttributeList", err: last_err() });
            }
            si.lpAttributeList = list;
            flags |= EXTENDED_STARTUPINFO_PRESENT;
            if let Some(hpc) = s.hpc {
                if UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    hpc as *const c_void,
                    std::mem::size_of::<isize>(),
                    null_mut(),
                    null(),
                ) == 0
                {
                    return Err(SpawnErr { step: "Update(PSEUDOCONSOLE)", err: last_err() });
                }
            }
            if let Some(jobs) = s.jobs {
                if UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_JOB_LIST as usize,
                    jobs.as_ptr() as *const c_void,
                    std::mem::size_of_val(jobs),
                    null_mut(),
                    null(),
                ) == 0
                {
                    return Err(SpawnErr { step: "Update(JOB_LIST)", err: last_err() });
                }
            }
            if let Some(hl) = s.handle_list {
                static EMPTY: [usize; 1] = [0];
                let p = if hl.is_empty() { EMPTY.as_ptr() as *const c_void } else { hl.as_ptr() as *const c_void };
                if UpdateProcThreadAttribute(
                    list,
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    p,
                    std::mem::size_of_val(hl),
                    null_mut(),
                    null(),
                ) == 0
                {
                    return Err(SpawnErr { step: "Update(HANDLE_LIST)", err: last_err() });
                }
            }
        }
        if let Some([i, o, e]) = s.std {
            si.StartupInfo.dwFlags |= STARTF_USESTDHANDLES;
            si.StartupInfo.hStdInput = i;
            si.StartupInfo.hStdOutput = o;
            si.StartupInfo.hStdError = e;
        }
        let app = s.app.map(w);
        let mut cmd = w(&s.cmd);
        let cwd = s.cwd.map(w);
        let env_ptr = match &s.env {
            Some(e) => {
                flags |= CREATE_UNICODE_ENVIRONMENT;
                e.as_ptr() as *const c_void
            }
            None => null(),
        };
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessW(
            app.as_ref().map_or(null(), |a| a.as_ptr()),
            cmd.as_mut_ptr(),
            null(),
            null(),
            s.inherit as BOOL,
            flags,
            env_ptr,
            cwd.as_ref().map_or(null(), |c| c.as_ptr()),
            &si.StartupInfo,
            &mut pi,
        );
        let err = last_err();
        if !si.lpAttributeList.is_null() {
            DeleteProcThreadAttributeList(si.lpAttributeList);
        }
        drop(attr_buf);
        if ok == 0 {
            return Err(SpawnErr { step: "CreateProcessW", err });
        }
        CloseHandle(pi.hThread);
        Ok(Proc { h: pi.hProcess, pid: pi.dwProcessId })
    }
}

pub fn nul_handle(inheritable: bool) -> H {
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: null_mut(),
        bInheritHandle: inheritable as BOOL,
    };
    unsafe {
        CreateFileW(
            w("NUL").as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            &sa,
            OPEN_EXISTING,
            0,
            null_mut(),
        )
    }
}

pub fn create_job(limit: u32) -> H {
    unsafe {
        let job = CreateJobObjectW(null(), null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = limit;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const c_void,
            std::mem::size_of_val(&info) as u32,
        );
        assert!(ok != 0, "SetInformationJobObject failed {}", last_err());
        job
    }
}

/// LimitFlags of `job`, or of the caller's immediate job when `job` is null.
pub fn job_limits(job: H) -> Value {
    unsafe {
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        let mut ret = 0u32;
        let ok = QueryInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of_val(&info) as u32,
            &mut ret,
        );
        if ok == 0 {
            return json!({"ok": false, "err": last_err()});
        }
        let f = info.BasicLimitInformation.LimitFlags;
        json!({
            "ok": true,
            "limit_flags": hex(f),
            "breakaway_ok": f & JOB_OBJECT_LIMIT_BREAKAWAY_OK != 0,
            "silent_breakaway_ok": f & JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK != 0,
            "kill_on_job_close": f & JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE != 0,
            "active_processes": job_active(job),
        })
    }
}

pub fn job_active(job: H) -> Value {
    unsafe {
        let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
        let ok = QueryInformationJobObject(
            job,
            JobObjectBasicAccountingInformation,
            &mut info as *mut _ as *mut c_void,
            std::mem::size_of_val(&info) as u32,
            null_mut(),
        );
        if ok == 0 {
            json!({"err": last_err()})
        } else {
            json!(info.ActiveProcesses)
        }
    }
}

pub fn job_pids(job: H) -> Vec<u32> {
    unsafe {
        let mut buf = vec![0usize; 2 + 256];
        let ok = QueryInformationJobObject(
            job,
            JobObjectBasicProcessIdList,
            buf.as_mut_ptr() as *mut c_void,
            (buf.len() * std::mem::size_of::<usize>()) as u32,
            null_mut(),
        );
        if ok == 0 {
            return vec![];
        }
        let hdr = buf.as_ptr() as *const u32;
        let n = *hdr.add(1) as usize;
        buf[1..1 + n].iter().map(|&p| p as u32).collect()
    }
}

pub struct ProcEntry {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
}

pub fn processes() -> Vec<ProcEntry> {
    let mut v = vec![];
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
                v.push(ProcEntry {
                    pid: e.th32ProcessID,
                    ppid: e.th32ParentProcessID,
                    name: String::from_utf16_lossy(&e.szExeFile[..len]),
                });
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
    }
    v
}

pub fn proc_name(pid: u32) -> String {
    processes().into_iter().find(|p| p.pid == pid).map(|p| p.name).unwrap_or_default()
}

fn token_info(class: TOKEN_INFORMATION_CLASS) -> Option<Vec<u8>> {
    unsafe {
        let mut tok: H = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let mut len = 0u32;
        GetTokenInformation(tok, class, null_mut(), 0, &mut len);
        let mut buf = vec![0u8; len as usize];
        let ok = GetTokenInformation(tok, class, buf.as_mut_ptr() as *mut c_void, len, &mut len);
        CloseHandle(tok);
        (ok != 0).then_some(buf)
    }
}

pub fn integrity_rid() -> Value {
    unsafe {
        let Some(buf) = token_info(TokenIntegrityLevel) else { return json!(null) };
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let n = *GetSidSubAuthorityCount(sid);
        json!(hex(*GetSidSubAuthority(sid, (n - 1) as u32)))
    }
}

pub fn elevated() -> Value {
    match token_info(TokenElevation) {
        Some(b) => json!(u32::from_ne_bytes([b[0], b[1], b[2], b[3]]) != 0),
        None => json!(null),
    }
}

pub fn package_name() -> Value {
    unsafe {
        let mut len = 0u32;
        let rc = GetCurrentPackageFullName(&mut len, null_mut());
        if rc == 15700 {
            return json!({"rc": rc, "meaning": "APPMODEL_ERROR_NO_PACKAGE"});
        }
        let mut buf = vec![0u16; len as usize + 1];
        let rc = GetCurrentPackageFullName(&mut len, buf.as_mut_ptr());
        json!({"rc": rc, "name": String::from_utf16_lossy(&buf[..len.saturating_sub(1) as usize])})
    }
}

pub fn self_report() -> Value {
    let pid = std::process::id();
    let ps = processes();
    let ppid = ps.iter().find(|p| p.pid == pid).map(|p| p.ppid).unwrap_or(0);
    let pname = ps.iter().find(|p| p.pid == ppid).map(|p| p.name.clone()).unwrap_or_default();
    // Ancestry chain, for context.
    let mut chain = vec![];
    let mut cur = ppid;
    for _ in 0..8 {
        match ps.iter().find(|p| p.pid == cur) {
            Some(p) => {
                chain.push(format!("{}:{}", p.pid, p.name));
                cur = p.ppid;
            }
            None => break,
        }
    }
    let mut session = 0u32;
    unsafe { windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(pid, &mut session) };
    json!({
        "in_job": in_job(unsafe { GetCurrentProcess() }, null_mut()),
        "job": job_limits(null_mut()),
        "session_id": session,
        "integrity_rid": integrity_rid(),
        "elevated": elevated(),
        "parent_pid": ppid,
        "parent_name": pname,
        "ancestry": chain,
        "package": package_name(),
    })
}
