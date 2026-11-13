use std::{
    env, fs,
    io::{self, Write},
    process::ExitCode,
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
    print!(
        "\x1b[?1049h\x1b[?1000h\x1b[?1002h\x1b[?1004h\x1b[?1006h\x1b[?2004h\x1b[2J\x1b[3;5H\x1b[6n"
    );
    io::stdout().flush()?;
    let mut previous = String::new();
    let input = climon2_fixtures::read_until(|input| {
        let screen = Seen::of(&String::from_utf8_lossy(input)).status(climon2_fixtures::size()?);
        if screen != previous {
            print!("\x1b[H\x1b[2K{screen}");
            io::stdout().flush()?;
            previous = screen;
        }
        Ok(input.contains(&4))
    })?;
    let text = String::from_utf8_lossy(&input);
    let seen = Seen::of(&text);
    let mut report = climon2_fixtures::report(&args)?;
    report["cursor"] = serde_json::json!(seen.cursor);
    report["mouse"] = serde_json::json!(seen.mouse);
    report["paste"] = serde_json::json!(seen.paste);
    report["input"] = serde_json::json!(input);
    fs::write(&args[0], serde_json::to_vec_pretty(&report)?)?;
    print!("\x1b[?2004l\x1b[?1006l\x1b[?1004l\x1b[?1002l\x1b[?1000l\x1b[?1049ltui-done");
    io::stdout().flush()?;
    Ok(())
}

/// The cursor reply, mouse report and paste found in the input so far.
struct Seen<'a> {
    cursor: Option<&'a str>,
    mouse: Option<&'a str>,
    paste: Option<&'a str>,
}

impl<'a> Seen<'a> {
    fn of(text: &'a str) -> Self {
        Self {
            cursor: sequence(text, "\x1b[", "R"),
            mouse: sequence(text, "\x1b[<", "M"),
            paste: text
                .split_once("\x1b[200~")
                .and_then(|(_, rest)| rest.split_once("\x1b[201~"))
                .map(|(paste, _)| paste),
        }
    }

    /// The status line: the terminal size and whether each has arrived.
    fn status(&self, [rows, cols]: [u16; 2]) -> String {
        let state = |name: &str, seen: Option<&str>| {
            format!("{name}-{}", if seen.is_some() { "ok" } else { "wait" })
        };
        format!(
            "size={rows}x{cols} {} {} {}",
            state("cursor", self.cursor),
            state("mouse", self.mouse),
            state("paste", self.paste)
        )
    }
}

fn sequence<'a>(input: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let start = input.find(prefix)?;
    let end = input[start..].find(suffix)? + start + suffix.len();
    Some(&input[start..end])
}
