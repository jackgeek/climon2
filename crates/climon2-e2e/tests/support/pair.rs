//! The Pair layer's harness (spec #488; acceptance, Evidence policy): a
//! packaged relay and packaged hub monitors on one host, each in its own
//! climon2 home, joined over loopback TCP, with a real session daemon behind
//! the relay. Each home is a [`Package`] of the one installed release.

use super::{Package, TIMEOUT, kill_session, scripted};
use climon2_test_term::{Terminal, process_command};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    net::SocketAddr,
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

const ROWS: u16 = 12;
const COLS: u16 = 100;

/// A relay running in a terminal of its own, in a home of its own that has
/// one live session.
pub struct RelayHome {
    pub package: Package,
    terminal: Terminal,
    listen: SocketAddr,
    session: String,
}

impl RelayHome {
    /// Starts a headless session, then `c2 relay` on any loopback port, and
    /// waits until the relay shows the address it listens on.
    pub fn start() -> Self {
        let package = Package::new();
        let session = headless_session(&package);
        let mut relay = package.command("c2");
        relay.args(["relay", "--listen", "127.0.0.1:0"]);
        let mut terminal = Terminal::spawn(relay, ROWS, COLS).unwrap();
        terminal
            .wait_for_screen(TIMEOUT, |screen| {
                let rows: Vec<_> = screen.lines().map(str::trim_end).collect();
                rows.first() == Some(&" Listening") && rows.last() == Some(&" p Pair  s Stop")
            })
            .unwrap();
        let listen = terminal.row(1).trim().parse().unwrap();
        Self {
            package,
            terminal,
            listen,
            session,
        }
    }

    /// The address the relay listens on, as its screen shows it.
    pub fn listen(&self) -> SocketAddr {
        self.listen
    }

    /// Presses the pair key and gives the PIN the screen shows.
    pub fn mint(&mut self) -> String {
        self.terminal.send("p").unwrap();
        let hint = format!(" Pair with: c2 monitor pair {}", self.listen);
        self.terminal.wait_for_text(&hint, TIMEOUT).unwrap();
        let pin = self.row(3);
        pin.trim().strip_prefix("PIN ").unwrap().to_owned()
    }

    /// Row `row` of the relay's screen, without trailing blanks.
    pub fn row(&self, row: usize) -> String {
        self.terminal.row(row).trim_end().to_owned()
    }

    pub fn wait_for(&mut self, text: &str) {
        self.terminal.wait_for_text(text, TIMEOUT).unwrap();
    }

    pub fn screen(&self) -> String {
        self.terminal.screen()
    }

    /// Stops the relay from its menu, ends the session and checks that
    /// nothing the home ran is left.
    pub fn stop(mut self) {
        self.terminal.send("s").unwrap();
        assert_eq!(self.terminal.wait_exit(TIMEOUT).unwrap(), 0);
        kill_session(&self.package, &self.session);
        self.package.assert_clean();
    }
}

/// A headless session that runs for at most two minutes: a real session
/// daemon in `package`'s home.
fn headless_session(package: &Package) -> String {
    let script = r#"until [ "$SECONDS" -ge 120 ]; do sleep 0.1; done"#;
    let output = scripted(
        package,
        "c2",
        &["exec", "--headless", "/bin/bash", "-c", script],
    );
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// `c2 monitor pair <address>` in `hub`'s home with stdin a pipe holding
/// `stdin`, and `env` set over the isolated environment.
pub fn pair_piped(hub: &Package, address: &str, stdin: &str, env: &[(&str, &str)]) -> Output {
    let isolated = hub.command("c2");
    let mut command = process_command(hub.executable_home().join("c2"));
    command
        .args(["monitor", "pair", address])
        .env_clear()
        .envs(isolated.iter_full_env_as_str())
        .envs(env.iter().copied())
        .current_dir(hub.isolation.home())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    finished(command, stdin)
}

fn finished(mut command: Command, stdin: &str) -> Output {
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let deadline = Instant::now() + TIMEOUT * 2;
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            panic!("c2 monitor pair was still running after {:?}", TIMEOUT * 2);
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.wait_with_output().unwrap()
}

/// `c2 m p <address>` in `hub`'s home, in a terminal of its own.
pub fn pair_in_terminal(hub: &Package, address: &str) -> Terminal {
    let mut command = hub.command("c2");
    command.args(["m", "p", address]);
    Terminal::spawn(command, ROWS, COLS).unwrap()
}

/// The file `name` of `package`'s `links/` folder, parsed, if it exists.
pub fn links_file(package: &Package, name: &str) -> Option<Value> {
    let path = links(package).join(name);
    let text = fs::read(&path).ok()?;
    Some(serde_json::from_slice(&text).unwrap())
}

pub fn links(package: &Package) -> PathBuf {
    package
        .isolation
        .climon2_home()
        .canonicalize()
        .unwrap()
        .join("links")
}

/// The key fingerprint of the key written as 64 hexadecimal digits, as
/// the pairing resolution's display needs it: the first 16 bytes of its
/// SHA-256, as colon-joined groups of four lowercase hexadecimal digits.
pub fn fingerprint(hex: &str) -> String {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).unwrap())
        .collect();
    let digest = Sha256::digest(&bytes);
    let hex: String = digest[..16]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let groups: Vec<_> = hex
        .as_bytes()
        .chunks(4)
        .map(|group| std::str::from_utf8(group).unwrap())
        .collect();
    groups.join(":")
}

/// This machine's host name, which both sides suggest as their name.
pub fn host_name() -> String {
    let output = Command::new("uname").arg("-n").output().unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

/// The pairing messages in `bytes`: each a big-endian `u16` length and that
/// many bytes, as the pairing pages lay them out. `bytes` must hold whole
/// messages only.
pub fn messages(mut bytes: &[u8]) -> Vec<usize> {
    let mut lengths = Vec::new();
    while let [high, low, rest @ ..] = bytes {
        let length = usize::from(u16::from_be_bytes([*high, *low]));
        assert!(rest.len() >= length, "a partial message: {bytes:?}");
        lengths.push(length);
        bytes = &rest[length..];
    }
    assert!(bytes.is_empty(), "a stray byte");
    lengths
}
