fn main() -> std::process::ExitCode {
    println!(
        "{:?} {:?}",
        [0, 1, 9, 10].map(isolation_core::mutation_probe),
        [None, Some(0), Some(1)].map(isolation_core::question_probe)
    );
    std::process::ExitCode::from(125)
}
