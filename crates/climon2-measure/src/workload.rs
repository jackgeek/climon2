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
    /// A FIFO whose lines `attention` and `working` set the reported status.
    pub attention: Option<PathBuf>,
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

/// The status a line read from the attention FIFO asks for.
pub fn requested(line: &str) -> Option<&'static str> {
    match line.trim() {
        "attention" => Some("blocked"),
        "working" => Some("working"),
        _ => None,
    }
}

/// Runs the application until its terminal goes away.
pub fn run(workload: &Workload) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(format!("\x1b]2;{}\x07", workload.title).as_bytes())?;
    out.write_all(&report("working"))?;
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

/// Reports what each line of the FIFO asks for, reopening it after each
/// writer closes it.
fn attend(fifo: &PathBuf) -> io::Result<()> {
    loop {
        let reader = io::BufReader::new(fs::File::open(fifo)?);
        for line in reader.lines() {
            if let Some(state) = requested(&line?) {
                let mut out = io::stdout();
                out.write_all(&report(state))?;
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
    fn the_fifo_switches_between_attention_and_working() {
        assert_eq!(requested("attention\n"), Some("blocked"));
        assert_eq!(requested("working"), Some("working"));
        assert_eq!(requested("other"), None);
        assert_eq!(
            report("blocked"),
            b"\x1b]7501;state=blocked:kind=question:app=climon2-measure\x1b\\"
        );
        assert_eq!(
            report("working"),
            b"\x1b]7501;state=working:app=climon2-measure\x1b\\"
        );
    }
}
