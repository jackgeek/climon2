mod discovery;
mod protocol;
mod session;
#[cfg(unix)]
#[path = "unix.rs"]
mod platform;
#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

use std::{env, path::{Path, PathBuf}, process::Command};
type Result<T> = std::result::Result<T, String>;

fn executable(name: &str) -> String { format!("{}{}", name, env::consts::EXE_SUFFIX) }
fn real_exe() -> Result<PathBuf> { env::current_exe().and_then(std::fs::canonicalize).map_err(|e| format!("cannot locate executable: {e}")) }
fn launch_error(path: impl AsRef<Path>, e: impl std::fmt::Display) -> String { format!("cannot launch {}: {e}", path.as_ref().display()) }
fn absolute(path: impl AsRef<Path>) -> Result<PathBuf> {
    let p = path.as_ref();
    if p.is_absolute() { Ok(p.to_owned()) } else { Ok(env::current_dir().map_err(|e| e.to_string())?.join(p)) }
}
fn nonempty(key: &str) -> Option<std::ffi::OsString> { env::var_os(key).filter(|s| !s.is_empty()) }
fn home() -> Result<PathBuf> {
    if let Some(p) = nonempty("CLIMON2_HOME") { return absolute(p); }
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    let p = nonempty(key).ok_or_else(|| format!("cannot determine home directory: {key} is unset or empty"))?;
    absolute(PathBuf::from(p).join(".climon2"))
}
fn launcher() -> Result<i32> {
    let exe = real_exe()?;
    let root = exe.parent().unwrap();
    let current = root.join("current");
    let value = std::fs::read_to_string(&current).map_err(|e| {
        format!("current is {}: {}: {e}", if e.kind() == std::io::ErrorKind::NotFound { "missing" } else { "invalid" }, current.display())
    })?;
    let version = value.strip_suffix("\r\n").or_else(|| value.strip_suffix('\n')).unwrap_or(&value);
    semver::Version::parse(version).map_err(|_| format!("current is invalid: {}", current.display()))?;
    let runtime = root.join("versions").join(version);
    if !runtime.is_dir() { return Err(format!("runtime directory is missing: {}", runtime.display())); }
    let cli = runtime.join(executable("climon2-cli"));
    let mut command = Command::new(&cli);
    command.args(env::args_os().skip(1));
    platform::launch_cli(command, &cli)
}
fn cli(args: &[String]) -> Result<i32> {
    match args {
        [sub] if sub == "list" => discovery::list(false),
        [sub, flag] if sub == "list" && flag == "--json" => discovery::list(true),
        [sub, command, rest @ ..] if sub == "exec" => session::exec(command, rest),
        _ => Err("usage: climon2 exec <command> [args...] | climon2 list [--json]".into())
    }
}
fn main() {
    let result = (|| {
        let args: Vec<String> = env::args().skip(1).collect();
        #[cfg(windows)]
        if env!("CARGO_BIN_NAME") == "climon2-daemon" && args.first().map(String::as_str) == Some("--run-command") { return platform::run_command(&args); }
        match env!("CARGO_BIN_NAME") {
            "climon2" => launcher(),
            "climon2-cli" => cli(&args),
            "climon2-daemon" => session::daemon(),
            "climon2-client" => session::client(&args),
            _ => cli(&args),
        }
    })();
    match result {
        Ok(code) => std::process::exit(code),
        Err(error) => { eprintln!("climon2: {error}"); std::process::exit(125); }
    }
}
