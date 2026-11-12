use std::{
    env, fs,
    io::{self, Read, Write},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

/// The longest this fixture sleeps or awaits input: longer than any test waits
/// on it under load, and short enough for a fixture a killed test left to end.
const BOUND: Duration = Duration::from_secs(60);

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
    let mut raw = None;
    while let Some(action) = script.next() {
        if action == "raw" {
            raw = Some(climon2_fixtures::RawTerminal::enter()?);
            continue;
        }
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
            "await-input" if raw.is_some() => await_input(value)?,
            "await-input" => return Err("await-input needs raw mode".into()),
            "exit" => return Ok(value.parse()?),
            "write-pid" => fs::write(value, std::process::id().to_string())?,
            "detach" => {
                let count: usize = value.parse()?;
                let detached: Vec<_> = script.by_ref().take(count).collect();
                if detached.len() != count {
                    return Err("detach exceeds the script".into());
                }
                detach(&detached)?;
            }
            "kill-self" => {
                let name = format!("SIG{}", value.trim_start_matches("SIG"));
                let signal: nix::sys::signal::Signal = name.parse()?;
                nix::sys::signal::raise(signal)?;
            }
            "sleep" => {
                let duration = Duration::from_millis(value.parse()?);
                if duration > BOUND {
                    return Err("sleep exceeds the fixture bound".into());
                }
                thread::sleep(duration);
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

/// Reads raw input until `expected` arrives, so a test can hold this fixture's
/// next output until the host has read everything before it.
fn await_input(expected: &str) -> Result<(), Box<dyn std::error::Error>> {
    let deadline = Instant::now() + BOUND;
    let mut input = Vec::new();
    while !String::from_utf8_lossy(&input).contains(expected) {
        if Instant::now() >= deadline {
            return Err("input deadline exceeded".into());
        }
        // Raw mode returns from a read after at most 0.1 s without input.
        let mut buffer = [0; 64];
        let count = io::stdin().read(&mut buffer)?;
        input.extend_from_slice(&buffer[..count]);
    }
    Ok(())
}

/// Starts a copy of this fixture running `script` like a daemon would: in its own session,
/// detached from the terminal, and with an empty environment.
fn detach(script: &[&String]) -> Result<(), Box<dyn std::error::Error>> {
    use nix::unistd::{
        ForkResult, dup2_stderr, dup2_stdin, dup2_stdout, execve, fork, getsid, setsid,
    };
    use std::{ffi::CString, os::unix::ffi::OsStringExt};
    let program = CString::new(env::current_exe()?.into_os_string().into_vec())?;
    let mut argv = vec![program.clone()];
    for argument in script {
        argv.push(CString::new(argument.as_str())?);
    }
    let null = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/null")?;
    // The fixture is single-threaded, so the child may run arbitrary code before exec.
    let child = match unsafe { fork() }? {
        ForkResult::Parent { child } => child,
        ForkResult::Child => {
            let error = setsid()
                .and_then(|_| dup2_stdin(&null))
                .and_then(|()| dup2_stdout(&null))
                .and_then(|()| dup2_stderr(&null))
                .and_then(|()| execve::<CString, CString>(&program, &argv, &[]))
                .unwrap_err();
            eprintln!("climon2-fixture: error: detach: {error}");
            std::process::exit(127);
        }
    };
    // Until the child leads its own session, this fixture exiting would hang it up.
    let deadline = Instant::now() + Duration::from_secs(2);
    while getsid(Some(child)) != Ok(child) {
        if Instant::now() >= deadline {
            return Err("detached child never started its session".into());
        }
        thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}
