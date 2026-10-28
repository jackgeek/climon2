fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(climon2_daemon::run(std::env::args_os().skip(1)))
}
