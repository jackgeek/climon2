use std::{env, fs, io::{self, BufRead, IsTerminal, Write}, path::PathBuf, process::{self, Command}};

fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""), '\\' => out.push_str("\\\\"),
            c if c <= '\u{1f}' || ('\u{7f}'..='\u{9f}').contains(&c) => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"'); out
}
fn strings(xs: impl Iterator<Item=String>) -> String {
    format!("[{}]", xs.map(|s| quote(&s)).collect::<Vec<_>>().join(","))
}
#[cfg(windows)]
fn platform() -> String {
    use std::ffi::c_void;
    #[link(name="kernel32")]
    extern "system" {
        fn GetConsoleProcessList(ids: *mut u32, n: u32) -> u32;
        fn QueryInformationJobObject(job: *mut c_void, class: u32, data: *mut c_void, size: u32, used: *mut u32) -> i32;
    }
    let mut ids = [0u32; 256];
    let n = unsafe { GetConsoleProcessList(ids.as_mut_ptr(), 256) } as usize;
    let console = &ids[..n.min(256)];
    let mut job = [0usize; 258];
    let ok = unsafe { QueryInformationJobObject(std::ptr::null_mut(), 3, job.as_mut_ptr().cast(), std::mem::size_of_val(&job) as u32, std::ptr::null_mut()) };
    let count = if ok != 0 { ((job[0] >> 32) as usize).min(256) } else { 0 };
    format!("\"console\":{:?},\"job\":{:?}", console, &job[1..1+count])
}
#[cfg(not(windows))]
fn platform() -> String { String::from("\"console\":[],\"job\":[]") }
fn report() -> String {
    let env = env::vars().map(|(k,v)| format!("{}:{}", quote(&k), quote(&v))).collect::<Vec<_>>().join(",");
    format!("{{\"pid\":{},\"executable\":{},\"argv\":{},\"cwd\":{},\"env\":{{{}}},\"tty\":[{},{},{}],{}}}",
        process::id(), quote(&env::current_exe().unwrap().to_string_lossy()), strings(env::args().skip(1)), quote(&env::current_dir().unwrap().to_string_lossy()), env,
        io::stdin().is_terminal(), io::stdout().is_terminal(), io::stderr().is_terminal(), platform())
}
#[cfg(windows)]
unsafe extern "system" fn interrupt(kind: u32) -> i32 {
    if kind == 0 { println!("PROBE_CTRL_C_{}", process::id()); 1 } else { 0 }
}
fn main() {
    #[cfg(windows)] unsafe {
        #[link(name="kernel32")]
        extern "system" { fn SetConsoleCtrlHandler(handler: unsafe extern "system" fn(u32) -> i32, add: i32) -> i32; }
        SetConsoleCtrlHandler(interrupt, 1);
    }
    let exe = env::current_exe().unwrap();
    let name = exe.file_stem().unwrap().to_string_lossy();
    let real = exe.with_file_name(format!("{}.real{}", name, env::consts::EXE_SUFFIX));
    if real.exists() {
        let log = PathBuf::from(env::var_os("PROBE_LOG").unwrap()).join(format!("{}-{}.json", name, process::id()));
        fs::write(log, report()).unwrap();
        let switcher = if cfg!(windows) { "climon2-cli" } else { "climon2-daemon" };
        if name == switcher {
            if let Some(current) = env::var_os("PROBE_CURRENT") { fs::write(current, "9.9.9\n").unwrap(); }
        }
        let mut cmd = Command::new(real); cmd.args(env::args_os().skip(1));
        #[cfg(unix)] { use std::os::unix::process::CommandExt; panic!("exec: {}", cmd.exec()); }
        #[cfg(windows)] { process::exit(cmd.status().unwrap().code().unwrap_or(126)); }
    }
    if let Some(path) = env::var_os("PROBE_PID_FILE") { fs::write(path, process::id().to_string()).unwrap(); }
    println!("PROBE {} PROBE_END", report()); io::stdout().flush().unwrap();
    if env::var_os("PROBE_STARTUP_STDERR").is_some() { eprintln!("PROBE_STARTUP_ERROR_STREAM"); }
    if env::var("PROBE_MODE").as_deref() == Ok("immediate") {
        println!("FINAL_DRAIN_{}", env::var("PROBE_MARKER").unwrap_or_default());
        process::exit(env::var("PROBE_EXIT").unwrap_or("37".into()).parse().unwrap());
    }
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        if let Some(code) = line.strip_prefix("exit ") { process::exit(code.parse().unwrap()); }
        println!("PROBE_OUT {}", quote(&line));
        eprintln!("PROBE_ERR {}", quote(&line));
        io::stdout().flush().unwrap();
    }
    process::exit(env::var("PROBE_EXIT").unwrap_or("37".into()).parse().unwrap());
}
