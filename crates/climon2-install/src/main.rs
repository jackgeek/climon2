fn main() -> std::process::ExitCode {
    climon2_core::test_hook_marker!();
    eprintln!("climon2: error: installer is not implemented");
    std::process::ExitCode::from(125)
}
