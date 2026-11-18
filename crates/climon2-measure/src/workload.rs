//! The measured sessions' application: it titles its session, reports
//! itself working (OSC 7501, so its status holds at Running whatever the
//! clock), writes lowercase lines at a fixed rate, discards what is typed
//! (the terminal's own echo shows it), and on request reports attention.
//! Its output never holds an uppercase letter, so a probe token typed in
//! uppercase is never mistaken for it.

use std::{
    fs,
    io::{self, BufRead, Read, Write},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

/// One output line: 71 lowercase characters and a newline.
pub const LINE: &[u8; 72] =
    b"measured output abcdefghijklmnopqrstuvwxyz 0123456789 abcdefghijklmnopq\n";

/// What the application does, from its arguments.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workload {
    pub title: String,
    /// Output bytes per second.
    pub rate: u32,
    /// A FIFO whose lines ask for a status report ([`answer`]).
    pub attention: Option<PathBuf>,
    /// Report nothing, so the daemon's own heuristic sets the status.
    pub heuristic: bool,
}

/// The bytes due `elapsed` into writing at `rate` bytes a second.
pub fn due(rate: u32, elapsed: Duration) -> u64 {
    u64::try_from(u128::from(rate) * elapsed.as_micros() / 1_000_000).unwrap_or(u64::MAX)
}

/// The OSC 7501 report of `state`.
pub fn report(state: &str) -> Vec<u8> {
    let kind = if state == "blocked" {
        ":kind=question"
    } else {
        ""
    };
    format!("\x1b]7501;state={state}{kind}:app=climon2-measure\x1b\\").into_bytes()
}

/// What the application writes for a line read from its FIFO: `attention`
/// and `working` report that status.
pub fn answer(line: &str) -> Option<Vec<u8>> {
    match line.trim() {
        "attention" => Some(report("blocked")),
        "working" => Some(report("working")),
        _ => None,
    }
}

/// What the application writes first: its title, the line `started
/// <title>`, and unless it is heuristic, that it is working.
pub fn opening(workload: &Workload) -> Vec<u8> {
    let mut bytes = format!("\x1b]2;{0}\x07started {0}\n", workload.title).into_bytes();
    if !workload.heuristic {
        bytes.extend(report("working"));
    }
    bytes
}

/// Runs the application until its terminal goes away.
pub fn run(workload: &Workload) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(&opening(workload))?;
    out.flush()?;
    if workload.rate > 0 {
        let rate = workload.rate;
        thread::spawn(move || write_at(rate));
    }
    if let Some(fifo) = workload.attention.clone() {
        thread::spawn(move || attend(&fifo));
    }
    let mut discarded = [0; 4096];
    while io::stdin().read(&mut discarded)? > 0 {}
    Ok(())
}

/// Writes [`LINE`] repeatedly, as many bytes as are [`due`] every 10 ms.
fn write_at(rate: u32) -> io::Result<()> {
    let start = Instant::now();
    let mut written = 0u64;
    let mut out = io::stdout();
    loop {
        thread::sleep(Duration::from_millis(10));
        let target = due(rate, start.elapsed());
        while written < target {
            let offset = (written % LINE.len() as u64) as usize;
            let count = (LINE.len() - offset).min((target - written) as usize);
            out.write_all(&LINE[offset..offset + count])?;
            written += count as u64;
        }
        out.flush()?;
    }
}

/// Writes what each line of the FIFO asks for, reopening it after each
/// writer closes it.
fn attend(fifo: &PathBuf) -> io::Result<()> {
    loop {
        let reader = io::BufReader::new(fs::File::open(fifo)?);
        for line in reader.lines() {
            if let Some(bytes) = answer(&line?) {
                let mut out = io::stdout();
                out.write_all(&bytes)?;
                out.flush()?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At 720 bytes a second, ten lines a second: 72 bytes after 100 ms,
    /// 720 after a second; nothing at a zero rate.
    #[test]
    fn output_is_due_at_the_fixed_rate() {
        assert_eq!(due(720, Duration::from_millis(100)), 72);
        assert_eq!(due(720, Duration::from_secs(1)), 720);
        assert_eq!(due(262_144, Duration::from_millis(10)), 2621);
        assert_eq!(due(0, Duration::from_secs(60)), 0);
    }

    /// The line is 72 bytes and holds no uppercase letter or escape.
    #[test]
    fn output_never_holds_what_a_probe_types() {
        assert_eq!(LINE.len(), 72);
        assert!(
            LINE[..71]
                .iter()
                .all(|&byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b' ')
        );
    }

    /// Attention is a blocked question; working clears it.
    #[test]
    fn the_fifo_asks_for_attention_or_working() {
        assert_eq!(
            answer("attention\n"),
            Some(b"\x1b]7501;state=blocked:kind=question:app=climon2-measure\x1b\\".to_vec())
        );
        assert_eq!(
            answer("working"),
            Some(b"\x1b]7501;state=working:app=climon2-measure\x1b\\".to_vec())
        );
        assert_eq!(answer("title"), None);
    }

    /// A session opens with its title and start line, and reports working
    /// unless the daemon's heuristic is to set its status.
    #[test]
    fn a_session_opens_titled_and_working_unless_heuristic() {
        let mut workload = Workload {
            title: "s001".to_owned(),
            rate: 0,
            attention: None,
            heuristic: false,
        };
        assert_eq!(
            opening(&workload),
            b"\x1b]2;s001\x07started s001\n\x1b]7501;state=working:app=climon2-measure\x1b\\"
        );
        workload.heuristic = true;
        assert_eq!(opening(&workload), b"\x1b]2;s001\x07started s001\n");
    }
}
