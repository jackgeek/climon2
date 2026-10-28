fn main() -> std::process::ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("--staged-probe") => {
            println!("{}", staged_probe(41));
            return std::process::ExitCode::SUCCESS;
        }
        Some("--staged-exec") => {
            // Like the CLI's client handoff: count, then replace the process image.
            println!("{}", staged_probe(41));
            use std::os::unix::process::CommandExt;
            let error = std::process::Command::new("/usr/bin/true").exec();
            panic!("exec /usr/bin/true: {error}");
        }
        _ => {}
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
