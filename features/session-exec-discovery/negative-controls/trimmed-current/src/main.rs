use std::{env, fs, path::Path, process::{self, Command}};
fn fail(message: &str, path: &Path) -> ! {
    eprintln!("climon2: {}: {}", message, path.display()); process::exit(125)
}
fn main() {
    let exe = env::current_exe().unwrap();
    if exe.file_stem().unwrap() != "climon2" {
        println!("No climon2 sessions running."); return;
    }
    let root = exe.parent().unwrap();
    let current = root.join("current");
    let raw = fs::read_to_string(&current).unwrap_or_else(|_| fail("current is missing", &current));
    // Deliberate defect: surrounding whitespace is accepted.
    let version = raw.trim();
    if semver::Version::parse(version).is_err() { fail("current is invalid", &current); }
    let runtime = root.join("versions").join(version);
    if !runtime.is_dir() { fail("runtime directory is missing", &runtime); }
    let cli = runtime.join(format!("climon2-cli{}", env::consts::EXE_SUFFIX));
    let mut command = Command::new(&cli); command.args(env::args_os().skip(1));
    #[cfg(unix)] { use std::os::unix::process::CommandExt; let _ = command.exec(); fail("cannot launch", &cli); }
    #[cfg(windows)] { process::exit(command.status().unwrap_or_else(|_| fail("cannot launch", &cli)).code().unwrap_or(125)); }
}
