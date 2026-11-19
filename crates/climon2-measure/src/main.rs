//! `climon2-measure`: the PERF-01 measurement harness (#121), and the
//! measured sessions' application.

use climon2_measure::{
    args::{self, Command},
    harness, workload,
};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args::parse(&args) {
        Ok(Command::Workload(workload)) => workload::run(&workload).map(|()| true),
        Ok(Command::Run(run)) => harness::measure(&run),
        Err(error) => {
            eprintln!("climon2-measure: {error}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => {
            eprintln!("climon2-measure: a harness error left cases unmeasured; see the report");
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("climon2-measure: {error}");
            ExitCode::FAILURE
        }
    }
}
