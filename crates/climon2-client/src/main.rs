#[cfg(not(target_os = "macos"))]
compile_error!("climon2-client currently supports macOS only");

fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    eprintln!("climon2: error: client is not implemented");
    std::process::ExitCode::from(125)
}
