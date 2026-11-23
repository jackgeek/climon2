fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    match climon2_client::run(std::env::args_os().skip(1)) {
        Ok(code) => exit(code),
        Err(error) => {
            eprintln!("climon2: error: {error}");
            std::process::ExitCode::from(error.code as u8)
        }
    }
}

/// Exits with the command's status: all 32 bits on Windows (#322), the low 8
/// on Unix, as `ExitCode::from(code as u8)` would.
fn exit(code: u32) -> ! {
    std::process::exit(code as i32)
}
