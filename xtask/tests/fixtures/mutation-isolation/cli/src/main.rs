fn main() -> std::process::ExitCode {
    if std::env::args().nth(1).as_deref() == Some("--staged-probe") {
        println!("{}", staged_probe(41));
        return std::process::ExitCode::SUCCESS;
    }
    println!(
        "{:?} {:?}",
        [0, 1, 9, 10].map(isolation_core::mutation_probe),
        [None, Some(0), Some(1)].map(isolation_core::question_probe)
    );
    std::process::ExitCode::from(125)
}

fn staged_probe(value: u32) -> u32 {
    value + 1
}
