fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    match climon2_relay::run(std::env::args_os().skip(1)) {
        Ok(status) => std::process::ExitCode::from(status as u8),
        Err(error) => {
            eprintln!("climon2: error: {error}");
            std::process::ExitCode::from(error.code as u8)
        }
    }
}
