//! Windows install probes for issue #215. Measures facts only.
//!
//! `winprobe run <base-dir> <out.jsonl>` runs probes 1-6 and 9 under
//! `<base-dir>` and writes one JSON object per line. The other subcommands are
//! the helper processes the probes start.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

#[cfg(windows)]
mod raw {
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
        fn DeleteFileW(path: *const u16) -> i32;
        fn LoadLibraryW(path: *const u16) -> *mut core::ffi::c_void;
    }

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    /// `MoveFileExW(from, to, MOVEFILE_REPLACE_EXISTING)`.
    pub fn move_replace(from: &Path, to: &Path) -> io::Result<()> {
        let (f, t) = (wide(from), wide(to));
        if unsafe { MoveFileExW(f.as_ptr(), t.as_ptr(), 1) } != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub fn delete(path: &Path) -> io::Result<()> {
        let p = wide(path);
        if unsafe { DeleteFileW(p.as_ptr()) } != 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    pub fn load_library(path: &Path) -> io::Result<()> {
        let p = wide(path);
        if unsafe { LoadLibraryW(p.as_ptr()) }.is_null() {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(not(windows))]
mod raw {
    //! Stubs so the crate type-checks on the Linux development host.
    use std::io;
    use std::path::Path;
    pub fn move_replace(from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    pub fn delete(path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
    pub fn load_library(_path: &Path) -> io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- recording

struct Out {
    file: File,
    probe: String,
    /// When set, renames use only `MoveFileExW` and deletes only `DeleteFileW`.
    raw_only: bool,
    counter: u32,
    base: PathBuf,
    self_exe: PathBuf,
    v2_exe: PathBuf,
    dll: PathBuf,
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o
}

impl Out {
    fn line(&mut self, step: &str, ok: bool, os_error: Option<i32>, detail: &str) {
        let code = os_error.map_or("null".to_string(), |c| c.to_string());
        let l = format!(
            "{{\"probe\":\"{}\",\"step\":\"{}\",\"ok\":{},\"os_error\":{},\"detail\":\"{}\"}}",
            esc(&self.probe),
            esc(step),
            ok,
            code,
            esc(detail)
        );
        println!("{l}");
        let _ = writeln!(self.file, "{l}");
    }

    fn rec<T>(&mut self, step: &str, r: &io::Result<T>) -> bool {
        match r {
            Ok(_) => self.line(step, true, None, ""),
            Err(e) => self.line(step, false, e.raw_os_error(), &e.to_string()),
        }
        r.is_ok()
    }

    fn note(&mut self, step: &str, ok: bool, detail: &str) {
        self.line(step, ok, None, detail);
    }

    /// `std::fs::rename`; on failure also plain `MoveFileExW(REPLACE_EXISTING)`.
    fn ren(&mut self, step: &str, from: &Path, to: &Path) -> bool {
        if !self.raw_only {
            let r = fs::rename(from, to);
            if self.rec(&format!("{step} [std::fs::rename]"), &r) {
                return true;
            }
        }
        let r = raw::move_replace(from, to);
        self.rec(&format!("{step} [MoveFileExW REPLACE_EXISTING]"), &r)
    }

    /// `std::fs::remove_file`; on failure also plain `DeleteFileW`.
    fn del(&mut self, step: &str, path: &Path) -> bool {
        if !self.raw_only {
            let r = fs::remove_file(path);
            if self.rec(&format!("{step} [std::fs::remove_file]"), &r) {
                return true;
            }
        }
        let r = raw::delete(path);
        self.rec(&format!("{step} [DeleteFileW]"), &r)
    }

    /// Delete with retries for up to 5 s; records attempts and elapsed time.
    fn del_retry(&mut self, step: &str, path: &Path) {
        let start = Instant::now();
        let mut attempts = 0u32;
        let mut first_err: Option<io::Error> = None;
        loop {
            attempts += 1;
            match fs::remove_file(path) {
                Ok(()) => {
                    let fe = first_err.map_or("none".to_string(), |e| {
                        format!("{:?} {}", e.raw_os_error(), e)
                    });
                    self.line(
                        step,
                        true,
                        None,
                        &format!(
                            "attempts={attempts} elapsed_ms={} first_error={fe}",
                            start.elapsed().as_millis()
                        ),
                    );
                    return;
                }
                Err(e) => {
                    if start.elapsed() > Duration::from_secs(5) {
                        self.line(
                            step,
                            false,
                            e.raw_os_error(),
                            &format!("still failing after 5 s, attempts={attempts}: {e}"),
                        );
                        return;
                    }
                    first_err.get_or_insert(e);
                    sleep(Duration::from_millis(10));
                }
            }
        }
    }

    fn fresh(&mut self, name: &str) -> PathBuf {
        self.counter += 1;
        let d = self.base.join(format!("{name}-{}", self.counter));
        fs::create_dir_all(&d).expect("create case dir");
        d
    }

    fn hb_path(&mut self) -> PathBuf {
        self.counter += 1;
        let d = self.base.join("hb");
        fs::create_dir_all(&d).expect("create hb dir");
        d.join(format!("hb-{}.txt", self.counter))
    }
}

// ------------------------------------------------------------ child control

/// A helper process that writes a growing counter to a heartbeat file.
struct Proc {
    child: Child,
    hb: PathBuf,
}

impl Proc {
    fn start(out: &mut Out, exe: &Path, dll: Option<&Path>, cwd: Option<&Path>) -> Proc {
        let hb = out.hb_path();
        let mut c = Command::new(exe);
        c.arg("serve").arg(&hb);
        if let Some(d) = dll {
            c.arg(d);
        }
        if let Some(d) = cwd {
            c.current_dir(d);
        }
        let child = c
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let p = Proc { child, hb };
        wait_for(&p.hb, 15);
        p
    }

    fn beat(&self) -> u64 {
        fs::read_to_string(&self.hb)
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }

    /// Alive means: not exited, and the heartbeat advances.
    fn alive(&mut self) -> (bool, String) {
        let before = self.beat();
        sleep(Duration::from_millis(300));
        let after = self.beat();
        let exited = self.child.try_wait().ok().flatten();
        (
            exited.is_none() && after > before,
            format!("heartbeat {before}->{after}, exit={exited:?}"),
        )
    }

    fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        self.stop();
    }
}

fn check_alive(out: &mut Out, step: &str, p: &mut Proc) {
    let (ok, d) = p.alive();
    out.note(step, ok, &d);
}

fn wait_for(path: &Path, secs: u64) -> bool {
    let start = Instant::now();
    while !path.exists() {
        if start.elapsed() > Duration::from_secs(secs) {
            return false;
        }
        sleep(Duration::from_millis(10));
    }
    true
}

fn run_capture(exe: &Path) -> io::Result<String> {
    let o = Command::new(exe).stdin(Stdio::null()).output()?;
    Ok(format!(
        "exit={:?} stdout={}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).trim()
    ))
}

fn run_new(out: &mut Out, step: &str, exe: &Path) {
    match run_capture(exe) {
        Ok(s) => out.note(step, s.contains("v2-ok"), &s),
        Err(e) => out.line(step, false, e.raw_os_error(), &e.to_string()),
    }
}

// ------------------------------------------------- probes 1-3: exe matrix

#[derive(Clone, Copy, PartialEq, Debug)]
enum Layout {
    Single,
    HardLink,
    Copy,
}

const NAMES: [&str; 2] = ["climon2.exe", "c2.exe"];

fn lay_out(out: &mut Out, layout: Layout, dir: &Path) {
    let main = dir.join(NAMES[0]);
    fs::copy(&out.self_exe, &main).expect("copy launcher");
    match layout {
        Layout::Single => {}
        Layout::HardLink => fs::hard_link(&main, dir.join(NAMES[1])).expect("hard link"),
        Layout::Copy => {
            fs::copy(&out.self_exe, dir.join(NAMES[1])).expect("copy sibling");
        }
    }
}

/// One operation on `target` while a process runs from `running`, each in a
/// fresh folder with a fresh process.
fn exe_matrix(out: &mut Out, layout: Layout) {
    let names: &[&str] = if layout == Layout::Single { &NAMES[..1] } else { &NAMES };
    for running in names {
        for target in names {
            for op in ["delete", "rename-aside", "rename-over"] {
                let dir = out.fresh("exe");
                lay_out(out, layout, &dir);
                let mut p = Proc::start(out, &dir.join(running), None, None);
                let tag = format!("{layout:?}: running={running} target={target}: {op}");
                let t = dir.join(target);
                let aside = dir.join(format!("{target}.old-1"));
                match op {
                    "delete" => {
                        out.del(&tag, &t);
                    }
                    "rename-aside" => {
                        if out.ren(&tag, &t, &aside) {
                            out.del(&format!("{tag}: then delete the aside file"), &aside);
                        }
                    }
                    _ => {
                        let new = dir.join("new.tmp");
                        fs::copy(&out.v2_exe, &new).expect("copy v2");
                        if out.ren(&tag, &new, &t) {
                            run_new(out, &format!("{tag}: run the new file"), &t);
                        }
                    }
                }
                check_alive(out, &format!("{tag}: process still healthy"), &mut p);
                let listing = list(&dir);
                out.note(&format!("{tag}: folder afterwards"), true, &listing);
                p.stop();
            }
        }
    }
}

fn list(dir: &Path) -> String {
    let mut v: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v.join(" ")
}

fn p1(out: &mut Out) {
    out.probe = format!("1{}", if out.raw_only { "-raw" } else { "" });
    exe_matrix(out, Layout::Single);

    // The full upgrade sequence on one process.
    let dir = out.fresh("p1seq");
    lay_out(out, Layout::Single, &dir);
    let exe = dir.join("climon2.exe");
    let mut p = Proc::start(out, &exe, None, None);
    let old1 = dir.join("climon2.exe.old-1");
    let old2 = dir.join("climon2.exe.old-2");
    out.ren("seq a: rename running exe aside", &exe, &old1);
    check_alive(out, "seq a: process still healthy", &mut p);
    let tmp = dir.join("climon2.exe.tmp");
    let r = fs::copy(&out.v2_exe, &tmp);
    out.rec("seq b: write new exe to temp name", &r);
    out.ren("seq b: rename new exe into the original name", &tmp, &exe);
    run_new(out, "seq b: run the new exe at the original name", &exe);
    check_alive(out, "seq b: old process still healthy", &mut p);
    let moved = out.ren("seq: rename the aside file a second time", &old1, &old2);
    let old = if moved { old2 } else { old1 };
    let new2 = dir.join("new2.tmp");
    let _ = fs::copy(&out.v2_exe, &new2);
    out.ren("seq d: rename a new file over the aside (running) image", &new2, &old);
    out.del("seq c: delete aside file while its process runs", &old);
    check_alive(out, "seq c: old process still healthy", &mut p);
    p.stop();
    out.del_retry("seq c: delete aside file after its process exited", &old);
}

fn p2(out: &mut Out) {
    out.probe = format!("2{}", if out.raw_only { "-raw" } else { "" });
    exe_matrix(out, Layout::HardLink);

    for both_running in [false, true] {
        let tag = if both_running {
            "both names running"
        } else {
            "only climon2.exe running"
        };
        let dir = out.fresh("p2seq");
        lay_out(out, Layout::HardLink, &dir);
        let (a, b) = (dir.join(NAMES[0]), dir.join(NAMES[1]));
        let mut p = Proc::start(out, &a, None, None);
        let mut q = both_running.then(|| Proc::start(out, &b, None, None));
        let (ao, bo) = (dir.join("climon2.exe.old-1"), dir.join("c2.exe.old-1"));
        out.ren(&format!("{tag}: rename climon2.exe aside"), &a, &ao);
        out.ren(&format!("{tag}: rename c2.exe aside"), &b, &bo);
        let tmp = dir.join("climon2.exe.tmp");
        let _ = fs::copy(&out.v2_exe, &tmp);
        out.ren(&format!("{tag}: rename new climon2.exe in"), &tmp, &a);
        let r = fs::hard_link(&a, &b);
        out.rec(&format!("{tag}: hard-link new c2.exe"), &r);
        run_new(out, &format!("{tag}: run new climon2.exe"), &a);
        run_new(out, &format!("{tag}: run new c2.exe"), &b);
        check_alive(out, &format!("{tag}: old process still healthy"), &mut p);
        if let Some(q) = q.as_mut() {
            check_alive(out, &format!("{tag}: old c2 process still healthy"), q);
        }
        out.del(&format!("{tag}: delete first aside name while running"), &ao);
        out.del(&format!("{tag}: delete second aside name while running"), &bo);
        check_alive(out, &format!("{tag}: old process healthy after deletes"), &mut p);
        p.stop();
        if let Some(q) = q.as_mut() {
            q.stop();
        }
        for f in [&ao, &bo] {
            if f.exists() {
                out.del_retry(&format!("{tag}: delete aside name after exit"), f);
            }
        }
        let listing = list(&dir);
        out.note(&format!("{tag}: folder afterwards"), true, &listing);
    }
}

fn p3(out: &mut Out) {
    out.probe = format!("3{}", if out.raw_only { "-raw" } else { "" });
    exe_matrix(out, Layout::Copy);
}

// ------------------------------------------------------ probe 4: folders

fn p4(out: &mut Out) {
    out.probe = format!("4{}", if out.raw_only { "-raw" } else { "" });
    for case in ["running-exe", "loaded-dll", "idle-files", "open-file", "process-cwd"] {
        let root = out.fresh("p4");
        let v = root.join("0.1.0");
        fs::create_dir_all(&v).unwrap();
        fs::write(v.join("data.txt"), "data").unwrap();
        let host = root.join("host.exe");
        fs::copy(&out.self_exe, &host).unwrap();
        let mut holder: Option<Child> = None;
        let mut p: Option<Proc> = match case {
            "running-exe" => {
                let exe = v.join("climon2-daemon.exe");
                fs::copy(&out.self_exe, &exe).unwrap();
                Some(Proc::start(out, &exe, None, None))
            }
            "loaded-dll" => {
                let dll = v.join("probedll.dll");
                fs::copy(&out.dll, &dll).unwrap();
                Some(Proc::start(out, &host, Some(&dll), None))
            }
            "open-file" => {
                let ready = root.join("ready");
                holder = Some(
                    Command::new(&host)
                        .args(["hold-open"])
                        .arg(v.join("data.txt"))
                        .arg(&ready)
                        .arg(root.join("go"))
                        .arg(root.join("held.out"))
                        .spawn()
                        .unwrap(),
                );
                wait_for(&ready, 15);
                None
            }
            "process-cwd" => Some(Proc::start(out, &host, None, Some(&v))),
            _ => None,
        };
        if let Some(p) = p.as_mut() {
            check_alive(out, &format!("{case}: process healthy before"), p);
        }
        let aside = root.join("0.1.0.old-1");
        let new = root.join("0.1.0.new");
        fs::create_dir_all(&new).unwrap();
        fs::write(new.join("data.txt"), "new").unwrap();
        out.ren(&format!("{case}: rename new folder over the existing one"), &new, &v);
        if out.ren(&format!("{case}: rename folder aside"), &v, &aside) {
            out.ren(&format!("{case}: rename new folder into the old name"), &new, &v);
        }
        if let Some(p) = p.as_mut() {
            check_alive(out, &format!("{case}: process healthy after"), p);
        }
        let listing = list(&root);
        out.note(&format!("{case}: root afterwards"), true, &listing);
        if let Some(p) = p.as_mut() {
            p.stop();
        }
        if let Some(mut h) = holder {
            let _ = h.kill();
            let _ = h.wait();
        }
        if v.exists() && !aside.exists() {
            out.ren(&format!("{case}: rename folder aside after the process exited"), &v, &aside);
        }
    }

    let root = out.fresh("p4empty");
    let (a, b) = (root.join("a"), root.join("b"));
    fs::create_dir_all(&a).unwrap();
    fs::create_dir_all(&b).unwrap();
    fs::write(a.join("f"), "x").unwrap();
    out.ren("idle: rename folder over an existing EMPTY folder", &a, &b);
}

// ------------------------------------------------- probe 5: pointer file

fn replace(cur: &Path, content: &str, use_raw: bool) -> (io::Result<()>, io::Result<()>) {
    let tmp = cur.with_extension("tmp");
    let w = fs::write(&tmp, content);
    if w.is_err() {
        return (w, Ok(()));
    }
    let r = if use_raw {
        raw::move_replace(&tmp, cur)
    } else {
        fs::rename(&tmp, cur)
    };
    (w, r)
}

fn tally(map: &mut BTreeMap<String, u64>, r: &io::Result<()>) {
    let k = match r {
        Ok(()) => "ok".to_string(),
        Err(e) => format!("os_error_{}", e.raw_os_error().unwrap_or(-1)),
    };
    *map.entry(k).or_default() += 1;
}

fn p5(out: &mut Out) {
    out.probe = "5".into();

    // (a) another process holds the file open for reading.
    let dir = out.fresh("p5a");
    let cur = dir.join("current");
    fs::write(&cur, "v0\n").unwrap();
    let (ready, go, held) = (dir.join("ready"), dir.join("go"), dir.join("held.out"));
    let mut h = Command::new(&out.self_exe)
        .arg("hold-open")
        .args([&cur, &ready, &go, &held])
        .spawn()
        .unwrap();
    wait_for(&ready, 15);
    let (w, r) = replace(&cur, "v1\n", false);
    out.rec("a: write temp while a reader holds `current` open", &w);
    out.rec("a: std::fs::rename temp over held-open `current`", &r);
    let (_, r) = replace(&cur, "v2\n", true);
    out.rec("a: MoveFileExW REPLACE_EXISTING temp over held-open `current`", &r);
    let mut fails = BTreeMap::new();
    for i in 3..103 {
        let (_, r) = replace(&cur, &format!("v{i}\n"), false);
        tally(&mut fails, &r);
    }
    out.note("a: 100 more std replacements while held open", true, &format!("{fails:?}"));
    let now = fs::read_to_string(&cur);
    out.note("a: path content after replacements", now.is_ok(), &format!("{now:?}"));
    fs::write(&go, "").unwrap();
    wait_for(&held, 15);
    sleep(Duration::from_millis(100));
    let seen = fs::read_to_string(&held);
    out.note("a: what the old open handle reads afterwards", seen.is_ok(), &format!("{seen:?}"));
    let _ = h.kill();
    let _ = h.wait();

    // (b) readers in a tight loop during >= 1000 replacements over >= 3 s.
    for use_raw in [false, true] {
        let how = if use_raw { "MoveFileExW REPLACE_EXISTING" } else { "std::fs::rename" };
        let dir = out.fresh("p5b");
        let cur = dir.join("current");
        fs::write(&cur, "v0\n").unwrap();
        let stop = dir.join("stop");
        let mut readers = Vec::new();
        for n in 0..2 {
            let res = dir.join(format!("reader-{n}.out"));
            let ready = dir.join(format!("reader-{n}.ready"));
            let c = Command::new(&out.self_exe)
                .arg("read-loop")
                .args([&cur, &stop, &res, &ready])
                .spawn()
                .unwrap();
            wait_for(&ready, 15);
            readers.push((c, res));
        }
        let (mut wr, mut rn) = (BTreeMap::new(), BTreeMap::new());
        let start = Instant::now();
        let mut i = 0u64;
        while i < 1000 || start.elapsed() < Duration::from_secs(3) {
            i += 1;
            let (w, r) = replace(&cur, &format!("v{i}\n"), use_raw);
            tally(&mut wr, &w);
            if w.is_ok() {
                tally(&mut rn, &r);
            }
        }
        let writer_ok = rn.len() == 1 && rn.contains_key("ok") && wr.len() == 1;
        out.note(
            &format!("b [{how}]: writer, {i} replacements in {} ms, no retries", start.elapsed().as_millis()),
            writer_ok,
            &format!("temp-write results {wr:?}; rename results {rn:?}"),
        );
        fs::write(&stop, "").unwrap();
        for (n, (mut c, res)) in readers.into_iter().enumerate() {
            let _ = c.wait();
            let s = fs::read_to_string(&res).unwrap_or_else(|e| format!("no result: {e}"));
            let clean = s.contains("failures=0 ");
            out.note(&format!("b [{how}]: reader {n}"), clean, s.trim());
        }
    }
}

fn read_loop(cur: &Path, stop: &Path, res: &Path, ready: &Path) {
    let mut m: BTreeMap<String, u64> = BTreeMap::new();
    let (mut total, mut ok, mut max) = (0u64, 0u64, 0u64);
    fs::write(ready, "").unwrap();
    while !(total % 64 == 0 && stop.exists()) {
        total += 1;
        match fs::read_to_string(cur) {
            Ok(s) => {
                let v = s
                    .strip_prefix('v')
                    .and_then(|r| r.strip_suffix('\n'))
                    .and_then(|d| d.parse::<u64>().ok());
                match v {
                    Some(v) => {
                        ok += 1;
                        max = max.max(v);
                    }
                    None if s.is_empty() => *m.entry("empty".into()).or_default() += 1,
                    None => *m.entry(format!("torn:{s:?}")).or_default() += 1,
                }
            }
            Err(e) => {
                *m.entry(format!("os_error_{}", e.raw_os_error().unwrap_or(-1))).or_default() += 1
            }
        }
    }
    let _ = fs::write(
        res,
        format!("reads={total} ok={ok} failures={} max_version_seen={max} by_kind={m:?}\n", total - ok),
    );
}

// ---------------------------------------------------- probe 6: lock file

fn try_lock_once(path: &Path) -> String {
    match OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path) {
        Err(e) => format!("open-error {:?} {e}", e.raw_os_error()),
        Ok(f) => match f.try_lock() {
            Ok(()) => "locked".into(),
            Err(TryLockError::WouldBlock) => "WouldBlock".into(),
            Err(TryLockError::Error(e)) => format!("lock-error {:?} {e}", e.raw_os_error()),
        },
    }
}

fn second_process(out: &mut Out, step: &str, lock: &Path, expect: &str) {
    let o = Command::new(&out.self_exe).arg("lock-try").arg(lock).output();
    match o {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            out.note(step, s == expect, &s);
        }
        Err(e) => out.line(step, false, e.raw_os_error(), &e.to_string()),
    }
}

fn start_holder(out: &mut Out, lock: &Path, ready: &Path) -> Child {
    let _ = fs::remove_file(ready);
    let c = Command::new(&out.self_exe)
        .arg("lock-hold")
        .args([lock, ready])
        .spawn()
        .unwrap();
    wait_for(ready, 15);
    sleep(Duration::from_millis(50));
    let s = fs::read_to_string(ready).unwrap_or_default();
    out.note("holder process takes the lock", s.trim() == "locked", s.trim());
    c
}

fn p6(out: &mut Out) {
    out.probe = "6".into();
    let root = out.fresh("p6");
    let home = root.join("home");
    fs::create_dir_all(&home).unwrap();
    let lock = home.join("install.lock");
    let ready = root.join("ready");
    let mut h = start_holder(out, &lock, &ready);
    second_process(out, "second process try_lock while held", &lock, "WouldBlock");
    let own = try_lock_once(&lock);
    out.note("second handle in a third process try_lock while held", own == "WouldBlock", &own);

    let cur = home.join("current");
    let mut rn = BTreeMap::new();
    for i in 0..1000 {
        let (_, r) = replace(&cur, &format!("v{i}\n"), false);
        tally(&mut rn, &r);
    }
    out.note(
        "1000 rename-over replacements of another file in the lock's folder while held",
        rn.len() == 1 && rn.contains_key("ok"),
        &format!("{rn:?}"),
    );
    second_process(out, "second process try_lock after the renames", &lock, "WouldBlock");
    let aside = root.join("home.old-1");
    if out.ren("rename the lock's parent folder aside while held", &home, &aside) {
        out.ren("rename the parent folder back", &aside, &home);
    }

    let pid = h.id().to_string();
    let start = Instant::now();
    let tk = Command::new("taskkill").args(["/F", "/PID", &pid]).output();
    let tk_s = match &tk {
        Ok(o) => format!("exit={:?} {}", o.status.code(), String::from_utf8_lossy(&o.stdout).trim()),
        Err(e) => format!("taskkill failed to start: {e}"),
    };
    out.note("taskkill /F the holder", tk.is_ok(), &tk_s);
    let mut attempts = 0;
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(10) {
        attempts += 1;
        last = try_lock_once(&lock);
        if last == "locked" {
            break;
        }
    }
    out.note(
        "lock is free after the holder was killed",
        last == "locked",
        &format!("result={last} attempts={attempts} ms_since_taskkill_started={}", start.elapsed().as_millis()),
    );
    let _ = h.kill();
    let _ = h.wait();

    // What can be done to the lock file itself while it is locked.
    let mut h = start_holder(out, &lock, &ready);
    let lock_aside = home.join("install.lock.old-1");
    if out.ren("rename the locked lock file aside", &lock, &lock_aside) {
        second_process(out, "try_lock on the original path after rename-aside (a new file)", &lock, "locked");
        second_process(out, "try_lock on the aside name (the held file)", &lock_aside, "WouldBlock");
        out.del("delete the aside (still locked) lock file", &lock_aside);
    } else {
        out.del("delete the locked lock file", &lock);
        second_process(out, "try_lock on the path after delete attempt", &lock, "WouldBlock");
    }
    let _ = h.kill();
    let _ = h.wait();
}

// -------------------------------------------------- probe 9: hard links

fn p9(out: &mut Out) {
    out.probe = "9".into();
    let dir = out.fresh("p9");
    let (a, b) = (dir.join("a.exe"), dir.join("b.exe"));
    fs::copy(&out.self_exe, &a).unwrap();
    let r = fs::hard_link(&a, &b);
    out.rec("std::fs::hard_link (CreateHardLinkW) in this folder", &r);
    let same = fs::read(&a).ok() == fs::read(&b).ok();
    out.note("link content equals original", same && r.is_ok(), &dir.display().to_string());
    let r = fs::write(dir.join("ads.txt:probe"), "x");
    out.rec("create an alternate data stream on this volume", &r);
}

// ----------------------------------------------------------------- main

fn run(base: &Path, out_path: &Path) {
    let self_exe = std::env::current_exe().unwrap();
    let bin = self_exe.parent().unwrap().to_path_buf();
    let _ = fs::remove_dir_all(base);
    fs::create_dir_all(base).unwrap();
    let mut out = Out {
        file: File::create(out_path).unwrap(),
        probe: "setup".into(),
        raw_only: false,
        counter: 0,
        base: base.to_path_buf(),
        v2_exe: bin.join("v2.exe"),
        dll: bin.join("probedll.dll"),
        self_exe,
    };
    out.note("base folder", true, &base.display().to_string());
    let probes: [(&str, fn(&mut Out), bool); 11] = [
        ("1", p1, false),
        ("2", p2, false),
        ("3", p3, false),
        ("4", p4, false),
        ("1-raw", p1, true),
        ("2-raw", p2, true),
        ("3-raw", p3, true),
        ("4-raw", p4, true),
        ("5", p5, false),
        ("6", p6, false),
        ("9", p9, false),
    ];
    for (name, f, raw_only) in probes {
        out.raw_only = raw_only;
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut out)));
        if let Err(e) = r {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            out.probe = name.into();
            out.note("PROBE PANICKED, later steps not run", false, &msg);
        }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let p = |i: usize| PathBuf::from(&a[i]);
    match a.get(1).map(String::as_str) {
        Some("run") => run(&p(2), &p(3)),
        Some("serve") => {
            if a.len() > 3 {
                if let Err(e) = raw::load_library(&p(3)) {
                    eprintln!("LoadLibraryW failed: {e}");
                    std::process::exit(3);
                }
            }
            let hb = p(2);
            for n in 1..12_000u64 {
                let _ = fs::write(&hb, n.to_string());
                sleep(Duration::from_millis(50));
            }
        }
        Some("hold-open") => {
            let mut f = File::open(p(2)).unwrap();
            fs::write(p(3), "").unwrap();
            if wait_for(&p(4), 600) {
                let mut s = String::new();
                let r = f.read_to_string(&mut s);
                let _ = fs::write(p(5), format!("{r:?} {s:?}"));
            }
            sleep(Duration::from_secs(600));
        }
        Some("read-loop") => read_loop(&p(2), &p(3), &p(4), &p(5)),
        Some("lock-hold") => {
            let f = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(p(2)).unwrap();
            let s = match f.try_lock() {
                Ok(()) => "locked".to_string(),
                Err(e) => format!("{e:?}"),
            };
            fs::write(p(3), s).unwrap();
            sleep(Duration::from_secs(600));
        }
        Some("lock-try") => println!("{}", try_lock_once(&p(2))),
        _ => {
            eprintln!("usage: winprobe run <base-dir> <out.jsonl>");
            std::process::exit(2);
        }
    }
}
