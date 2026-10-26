use std::{env, fs, path::Path, process::{self, Command}};
fn fail(message: &str, path: &Path) -> ! {
    eprintln!("climon2: {}: {}", message, path.display()); process::exit(125)
}
fn main() {
    let exe = env::current_exe().unwrap();
    let root = exe.parent().unwrap();
    let current = root.join("current");
    let raw = fs::read_to_string(&current).unwrap_or_else(|_| fail("current is missing", &current));
    let version = raw.strip_suffix("\r\n").or_else(|| raw.strip_suffix('\n')).unwrap_or(&raw);
    if semver::Version::parse(version).is_err() { fail("current is invalid", &current); }
    let runtime = root.join("versions").join(version);
    if !runtime.is_dir() { fail("runtime directory is missing", &runtime); }
    let cli = runtime.join(format!("climon2-cli{}", env::consts::EXE_SUFFIX));
    let mut command = Command::new(&cli);
    // Deliberate defect: empty argument values are discarded.
    command.args(env::args_os().skip(1).filter(|arg| !arg.is_empty()));
    #[cfg(unix)] { use std::os::unix::process::CommandExt; let _ = command.exec(); fail("cannot launch", &cli); }
    #[cfg(windows)] { process::exit(command.status().unwrap_or_else(|_| fail("cannot launch", &cli)).code().unwrap_or(125)); }
}
