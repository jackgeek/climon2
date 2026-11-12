#[cfg(not(any(target_os = "macos", target_os = "linux")))]
compile_error!("climon2-client requires macOS or Linux");

fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    match climon2_client::run(std::env::args_os().skip(1)) {
        Ok(code) => std::process::ExitCode::from(code as u8),
        Err(error) => {
            eprintln!("climon2: error: {error}");
            std::process::ExitCode::from(error.code as u8)
        }
    }
}
