/// Exits with the session's status at the width the platform keeps: all 32
/// bits on Windows (#323).
fn main() {
    climon2_core::test_hook_marker!();
    let code = match climon2_client::run(std::env::args_os().skip(1)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("climon2: error: {error}");
            error.code
        }
    };
    climon2_platform::process::exit(code)
}
