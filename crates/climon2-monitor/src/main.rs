fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    match climon2_monitor::run() {
        Ok(status) => std::process::ExitCode::from(status as u8),
        Err(error) => {
            eprintln!("climon2: error: {error}");
            std::process::ExitCode::from(climon2_core::exit::CLIMON2_ERROR as u8)
        }
    }
}
