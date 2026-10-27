#[cfg(not(unix))]
compile_error!("climon2 launcher currently supports Unix only");

pub fn run() -> std::process::ExitCode {
    eprintln!("climon2: error: launcher is not implemented");
    std::process::ExitCode::from(125)
}
