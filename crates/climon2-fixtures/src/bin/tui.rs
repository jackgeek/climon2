use std::{
    env, fs,
    io::{self, Read, Write},
    process::ExitCode,
    time::{Duration, Instant},
};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("climon2-tui-fixture: error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.len() != 1 {
        return Err("expected report file".into());
    }
    let _raw = climon2_fixtures::RawTerminal::enter()?;
    print!("\x1b[?1049h\x1b[?1000h\x1b[?1006h\x1b[?2004h\x1b[2J\x1b[3;5H\x1b[6n");
    io::stdout().flush()?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut input = Vec::new();
    let mut previous = String::new();
    loop {
        if Instant::now() >= deadline {
            return Err("TUI fixture deadline exceeded".into());
        }
        let mut buffer = [0; 1024];
        let count = io::stdin().read(&mut buffer)?;
        input.extend_from_slice(&buffer[..count]);
        let text = String::from_utf8_lossy(&input);
        let cursor = sequence(&text, "\x1b[", "R");
        let mouse = sequence(&text, "\x1b[<", "M");
        let paste = text
            .split_once("\x1b[200~")
            .and_then(|(_, rest)| rest.split_once("\x1b[201~"))
            .map(|(paste, _)| paste);
        let [rows, cols] = climon2_fixtures::size()?;
        let screen = format!(
            "size={rows}x{cols} {} {} {}",
            if cursor.is_some() {
                "cursor-ok"
            } else {
                "cursor-wait"
            },
            if mouse.is_some() {
                "mouse-ok"
            } else {
                "mouse-wait"
            },
            if paste.is_some() {
                "paste-ok"
            } else {
                "paste-wait"
            }
        );
        if screen != previous {
            print!("\x1b[H\x1b[2K{screen}");
            io::stdout().flush()?;
            previous = screen;
        }
        if input.contains(&4) {
            let mut report = climon2_fixtures::report(&args)?;
            report["cursor"] = serde_json::json!(cursor);
            report["mouse"] = serde_json::json!(mouse);
            report["paste"] = serde_json::json!(paste);
            fs::write(&args[0], serde_json::to_vec_pretty(&report)?)?;
            print!("\x1b[?2004l\x1b[?1006l\x1b[?1000l\x1b[?1049ltui-done");
            io::stdout().flush()?;
            return Ok(());
        }
    }
}

fn sequence<'a>(input: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let start = input.find(prefix)?;
    let end = input[start..].find(suffix)? + start + suffix.len();
    Some(&input[start..end])
}
