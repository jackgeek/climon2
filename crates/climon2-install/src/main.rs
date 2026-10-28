fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    match climon2_install::run(std::env::args_os().skip(1)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            let message = error.to_string().replace('\r', "\\r").replace('\n', "\\n");
            eprintln!("climon2: error: {message}");
            std::process::ExitCode::from(125)
        }
    }
}
