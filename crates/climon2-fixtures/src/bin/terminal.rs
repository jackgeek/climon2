use std::{
    env, fs,
    io::{self, Write},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("climon2-fixture: error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.is_empty() {
        return Err("expected fixture script".into());
    }
    let mut script = args.iter();
    while let Some(action) = script.next() {
        if action == "await-resize" {
            let initial = climon2_fixtures::size()?;
            print!("resize-ready");
            io::stdout().flush()?;
            let deadline = Instant::now() + Duration::from_secs(2);
            while climon2_fixtures::size()? == initial {
                if Instant::now() >= deadline {
                    return Err("resize deadline exceeded".into());
                }
                thread::sleep(Duration::from_millis(5));
            }
            continue;
        }
        if action == "ignore-sighup" {
            // This fixture is single-threaded; no other code installs signal handlers.
            unsafe {
                nix::sys::signal::signal(
                    nix::sys::signal::Signal::SIGHUP,
                    nix::sys::signal::SigHandler::SigIgn,
                )?;
            }
            continue;
        }
        let value = script.next().ok_or("missing script argument")?;
        match action.as_str() {
            "report" => report(value, &args)?,
            "print" => {
                print!("{value}");
                io::stdout().flush()?;
            }
            "exit" => return Ok(value.parse()?),
            "kill-self" => {
                let name = format!("SIG{}", value.trim_start_matches("SIG"));
                let signal: nix::sys::signal::Signal = name.parse()?;
                nix::sys::signal::raise(signal)?;
            }
            "sleep" => {
                let millis: u64 = value.parse()?;
                if millis > 5000 {
                    return Err("sleep exceeds 5 s fixture bound".into());
                }
                thread::sleep(Duration::from_millis(millis));
            }
            _ => return Err(format!("unknown fixture action: {action}").into()),
        }
    }
    Ok(0)
}

fn report(path: &str, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let value = climon2_fixtures::report(args)?;
    fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}
