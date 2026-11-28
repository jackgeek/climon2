//! `c2 monitor pair <host:port>` (spec #488, CM10-15): reads the PIN of the
//! relay's link invitation in this process, then pairs through the monitor
//! library. The PIN comes from an echoing prompt on a terminal, or from one
//! line of stdin, and from nowhere else (P13).
use climon2_core::exit::CLIMON2_ERROR;
use climon2_monitor::{
    Error, Startup,
    links::{self, Paired, Pin},
};
use std::io::{self, BufRead, IsTerminal, Write};

const PROMPT: &[u8] = b"PIN: ";
const NOT_A_PIN: &str = "a PIN is 8 digits; hyphens and spaces are ignored";

/// Pairs with the relay at `address`, as the user typed it. Success prints
/// one line to stdout naming the relay and its key fingerprint, to compare
/// with the relay's screen, and exits 0. Every failure is the climon2 error
/// code, with its reason.
pub(crate) fn run(address: &str) -> Result<u32, Error> {
    let startup = Startup::read()?;
    let pin = typed_pin()?;
    let largest_record = startup.config.settings().limits.library().wire_frame_bytes;
    let paired = links::pair(&startup.home, address, pin, largest_record)
        .map_err(|unpaired| Error::new(CLIMON2_ERROR, unpaired))?;
    println!("{}", reported(&paired));
    Ok(0)
}

/// The PIN the user types: at an echoing prompt on a terminal, or as one
/// line of a pipe.
fn typed_pin() -> Result<Pin, Error> {
    let stdin = io::stdin();
    let typed = read_pin(stdin.is_terminal(), &mut stdin.lock(), &mut io::stderr())?;
    Pin::parse(&typed).ok_or_else(|| Error::new(CLIMON2_ERROR, NOT_A_PIN))
}

/// The PIN as typed: one line of `input`, without its line ending. On a
/// terminal it is asked for first with `PIN: ` on `prompt`; the terminal
/// echoes what is typed.
fn read_pin(
    terminal: bool,
    input: &mut impl BufRead,
    prompt: &mut impl Write,
) -> io::Result<String> {
    if terminal {
        prompt.write_all(PROMPT)?;
        prompt.flush()?;
    }
    let mut line = String::new();
    input.read_line(&mut line)?;
    let typed = line.strip_suffix('\n').unwrap_or(&line);
    Ok(typed.strip_suffix('\r').unwrap_or(typed).to_owned())
}

/// The line that reports a pairing.
fn reported(paired: &Paired) -> String {
    format!(
        "paired with {} (key fingerprint {})",
        paired.name, paired.fingerprint
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(terminal: bool, input: &str) -> (String, String) {
        let mut prompt = Vec::new();
        let typed = read_pin(terminal, &mut input.as_bytes(), &mut prompt).unwrap();
        (typed, String::from_utf8(prompt).unwrap())
    }

    /// CM10 (PAIR-10): a terminal is prompted on stderr; a pipe is not.
    #[test]
    fn a_terminal_is_prompted_and_a_pipe_gives_one_line() {
        assert_eq!(
            read(true, "1234-5678\n"),
            ("1234-5678".into(), "PIN: ".into())
        );
        assert_eq!(
            read(false, "1234 5678\nmore\n"),
            ("1234 5678".into(), String::new())
        );
    }

    #[test]
    fn the_line_ending_is_dropped_whatever_it_is() {
        assert_eq!(read(false, "12345678\r\n").0, "12345678");
        assert_eq!(read(false, "12345678").0, "12345678");
        assert_eq!(read(false, "").0, "");
        assert_eq!(read(false, "1234\r5678\n").0, "1234\r5678");
    }

    /// CM15: the success line names the relay and its fingerprint.
    #[test]
    fn a_pairing_is_reported_with_the_relays_name_and_fingerprint() {
        let paired = Paired {
            name: "buildbox".into(),
            fingerprint: "630d:cd29:66c4:3366:9112:5448:bbb2:5b4f".into(),
        };
        assert_eq!(
            reported(&paired),
            "paired with buildbox (key fingerprint 630d:cd29:66c4:3366:9112:5448:bbb2:5b4f)"
        );
    }
}
