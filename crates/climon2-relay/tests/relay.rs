//! #499 (spec #488): the relay executable itself, started in a real terminal
//! with no CLI in front of it. RLY-01, RLY-02 and RLY-03, as far as a relay
//! with no link yet can show them: what it binds, what it shows, how it
//! refuses to start and how it stops. Expectations come from the relay
//! executable resolution (#194) and the command-surface resolution (#478).
#![cfg(unix)]

use climon2_test_term::{InputTerminal, Isolation, Modes, Terminal, process_command};
use nix::{
    sys::signal::{Signal, kill},
    unistd::Pid,
};
use std::{
    fs,
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

const RELAY: &str = env!("CARGO_BIN_EXE_climon2-relay");
const TIMEOUT: Duration = Duration::from_secs(20);
const ROWS: u16 = 10;
const COLS: u16 = 60;
const USAGE: &str = "climon2: error: usage: c2 exec|x [--headless|-h] [--nested|-n] <command> [args...] | attach|a <session-id> | kill|k <session-id> | list|l [--json] | monitor|m [pair|p <host:port>] | relay|r [--listen <ip>:<port>]\n";

/// The relay in a terminal of its own, in `home`'s climon2 home.
fn start(home: &Isolation, args: &[&str]) -> Terminal {
    let mut relay = home.command(RELAY);
    relay.args(args);
    Terminal::spawn(relay, ROWS, COLS).unwrap()
}

/// The relay with no terminal: stdin from /dev/null, the rest into pipes.
fn scripted(home: &Isolation, args: &[&str]) -> Output {
    let isolated = home.command(RELAY);
    let mut relay = process_command(RELAY);
    relay
        .args(args)
        .env_clear()
        .envs(isolated.iter_full_env_as_str())
        .current_dir(home.home())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    climon2_test_term::output_within(&mut relay, TIMEOUT)
}

/// Whether `screen` is the whole listening screen: the status bar on the
/// first row and the key bar, drawn last, on the last.
fn drawn(screen: &str) -> bool {
    let rows: Vec<_> = screen.lines().map(str::trim_end).collect();
    rows.first() == Some(&" Listening") && rows.last() == Some(&" p Pair  s Stop")
}

/// Waits for the listening screen and returns the address its detail row
/// shows.
fn listening(terminal: &mut Terminal) -> SocketAddr {
    terminal.wait_for_screen(TIMEOUT, drawn).unwrap();
    terminal.row(1).trim().parse().unwrap_or_else(|_| {
        panic!(
            "the detail row is no listen address:\n{}",
            terminal.screen()
        )
    })
}

/// The one general log a relay run left in `home`.
fn relay_log(home: &Isolation) -> PathBuf {
    let general = home
        .climon2_home()
        .canonicalize()
        .unwrap()
        .join("logs/general");
    let mut logs: Vec<_> = fs::read_dir(&general)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(logs.len(), 1, "{logs:?}");
    let log = logs.pop().unwrap();
    let name = log.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("relay-"), "{name}");
    log
}

/// A start that failed: status 125, nothing on stdout, and on stderr the
/// reason and the name of the log that recorded it.
fn assert_start_failure(output: &Output, home: &Isolation, reason: &str) {
    assert_eq!(output.status.code(), Some(125), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let log = relay_log(home);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        format!("climon2: error: {reason} (logs: {})\n", log.display())
    );
    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains(" role=relay "), "{recorded}");
    assert!(recorded.contains(reason), "{recorded}");
}

/// A free loopback port, held by nothing once this returns.
fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// After a clean stop: the status, the shell's screen and modes back, and no
/// log named.
fn assert_stopped(terminal: &mut Terminal, status: i32) {
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), status);
    assert_eq!(terminal.modes(), Modes::BASELINE, "{}", terminal.screen());
    let screen = terminal.screen();
    assert!(!screen.contains("Listening"), "{screen}");
    assert!(!screen.contains("logs:"), "{screen}");
    assert!(!screen.contains("error"), "{screen}");
}

/// RLY-01, RLY-03 (X11, CM4): the listener is bound at start, to the port the
/// system picked when asked for port 0, and the screen shows that port. The
/// key bar names stop, and stop from it exits 0 and closes the listener.
#[test]
fn the_relay_listens_shows_the_bound_address_and_stops_from_its_menu() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    let address = listening(&mut terminal);
    assert_eq!(terminal.row(0).trim_end(), " Listening");
    assert_eq!(address.ip().to_string(), "127.0.0.1");
    assert_ne!(
        address.port(),
        0,
        "the screen shows the port actually bound"
    );
    assert_eq!(
        terminal.row(ROWS as usize - 1).trim_end(),
        " p Pair  s Stop"
    );
    assert!(terminal.modes().alternate_screen);
    // The cursor is hidden just after the frame is drawn.
    let deadline = std::time::Instant::now() + TIMEOUT;
    while terminal.modes().cursor_visible {
        assert!(
            std::time::Instant::now() < deadline,
            "the cursor stays shown"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    TcpStream::connect_timeout(&address, TIMEOUT).expect("the listener is bound");
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
    assert!(
        TcpStream::connect_timeout(&address, TIMEOUT).is_err(),
        "a stopped relay listens no more"
    );
    home.assert_clean();
}

/// #501 (X6, P4-5, ST5): the pair key mints a link invitation, whose PIN
/// and listen address the screen shows; the relay's identity was made at
/// its start and the PIN is written to no file.
#[test]
fn the_pair_key_shows_a_pin_and_the_address_to_pair_with() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    let address = listening(&mut terminal);
    let links = home.climon2_home().canonicalize().unwrap().join("links");
    assert!(links.join("relay-identity.json").is_file());
    terminal.send("p").unwrap();
    let hint = format!(" Pair with: c2 monitor pair {address}");
    terminal.wait_for_text(&hint, TIMEOUT).unwrap();
    let pin = terminal.row(3).trim().trim_start_matches("PIN ").to_owned();
    assert_eq!(pin.len(), 9, "{}", terminal.screen());
    assert!(
        pin[..4]
            .bytes()
            .chain(pin[5..].bytes())
            .all(|b| b.is_ascii_digit())
    );
    assert_eq!(&pin[4..5], "-");
    assert_eq!(terminal.row(4).trim_end(), hint);
    for entry in fs::read_dir(&links).unwrap() {
        let text = fs::read(entry.unwrap().path()).unwrap();
        let text = String::from_utf8_lossy(&text);
        assert!(!text.contains(&pin) && !text.contains(&pin.replace('-', "")));
    }
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
    home.assert_clean();
}

/// X10: an idle relay keeps running, and keys that are not its stop key,
/// Ctrl+C and an escape sequence holding an `s` among them, do nothing.
#[test]
fn an_idle_relay_keeps_running_whatever_else_is_typed() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    let address = listening(&mut terminal);
    terminal.send("x\x03q\r").unwrap();
    terminal.send("\x1bs").unwrap();
    std::thread::sleep(Duration::from_millis(300));
    kill(Pid::from_raw(terminal.pid()), None).expect("the relay still runs");
    assert_eq!(listening(&mut terminal), address);
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
}

/// RLY-02 (CM5): SIGTERM stops the relay cleanly with status 143.
#[test]
fn sigterm_stops_the_relay_with_143() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    listening(&mut terminal);
    kill(Pid::from_raw(terminal.pid()), Signal::SIGTERM).unwrap();
    assert_stopped(&mut terminal, 143);
    home.assert_clean();
}

/// RLY-02 (CM5): a closing terminal hangs the relay up, which stops it
/// cleanly with status 129.
#[test]
fn a_hangup_signal_stops_the_relay_with_129() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    listening(&mut terminal);
    kill(Pid::from_raw(terminal.pid()), Signal::SIGHUP).unwrap();
    assert_stopped(&mut terminal, 129);
}

/// RLY-02 (CM5): a terminal whose input has closed stops the relay cleanly
/// with status 129, whether or not a signal came with it.
#[test]
fn closed_terminal_input_stops_the_relay_with_129() {
    let home = Isolation::new();
    let input = InputTerminal::open(ROWS, COLS).unwrap();
    let mut shell = home.command("/bin/sh");
    shell.args([
        "-c",
        "trap '' HUP; \"$0\" --listen 127.0.0.1:0 < \"$1\"; echo \"STATUS=$?\"",
        RELAY,
    ]);
    shell.arg(input.path());
    let mut terminal = Terminal::spawn(shell, ROWS, COLS).unwrap();
    listening(&mut terminal);
    input.hang_up();
    terminal.wait_for_text("STATUS=129", TIMEOUT).unwrap();
    assert_eq!(terminal.modes(), Modes::BASELINE, "{}", terminal.screen());
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
}

/// A relay redraws for its terminal's new size: the key bar stays on the
/// last row.
#[test]
fn the_relay_redraws_when_its_terminal_is_resized() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    let address = listening(&mut terminal);
    terminal.resize(6, 40).unwrap();
    let redrawn = |screen: &str| {
        let rows: Vec<_> = screen.lines().collect();
        rows.len() == 6 && rows[5].trim_end() == " p Pair  s Stop" && rows[4].trim().is_empty()
    };
    terminal.wait_for_screen(TIMEOUT, redrawn).unwrap();
    assert_eq!(listening(&mut terminal), address);
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
}

/// RLY-01 (X4), RLY-02 (CM5): without a terminal the relay refuses to start:
/// 125, the reason and its log on stderr, and nothing bound.
#[test]
fn the_relay_refuses_to_start_without_a_terminal() {
    let home = Isolation::new();
    let port = free_port();
    let output = scripted(&home, &["--listen", &format!("127.0.0.1:{port}")]);
    assert_start_failure(&output, &home, "c2 relay needs a terminal");
    TcpListener::bind(("127.0.0.1", port)).expect("the relay bound nothing");
    assert!(!home.climon2_home().join("links").exists());
}

/// RLY-02 (CM5): an unknown flag or a malformed `--listen` is a usage error,
/// status 2, answered before the home is touched: no log is written or named.
#[test]
fn a_usage_error_is_2_and_touches_no_home() {
    let home = Isolation::new();
    for args in [
        &["--port", "7222"][..],
        &["--listen", "localhost:7222"],
        &["--listen", "127.0.0.1"],
        &["--listen"],
        &["extra"],
    ] {
        let output = scripted(&home, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty(), "{args:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr), USAGE, "{args:?}");
    }
    assert!(!home.climon2_home().join("logs").exists());
    // In a terminal too: the screen is never entered.
    let mut terminal = start(&home, &["--nope"]);
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 2);
    assert!(terminal.screen().contains("usage: c2 exec|x"));
    assert!(!terminal.screen().contains("Listening"));
}

/// RLY-01 (X9): a second start in the same home fails and names the running
/// relay, which carries on; a relay in another home starts beside it.
#[test]
fn a_second_relay_in_one_home_fails_naming_the_first_and_two_homes_run_two() {
    let home = Isolation::new();
    let mut first = start(&home, &["--listen", "127.0.0.1:0"]);
    let address = listening(&mut first);
    let mut second = start(&home, &["--listen", "127.0.0.1:0"]);
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 125);
    let refusal = second.screen().replace('\n', "");
    let named = format!(
        "climon2: error: a relay is already running in this climon2 home (pid {}, listening on {address}) (logs: ",
        first.pid()
    );
    assert!(refusal.contains(&named), "{}", second.screen());
    assert!(!refusal.contains("Listening"), "{}", second.screen());
    assert_eq!(second.modes(), Modes::BASELINE);

    let other = Isolation::new();
    let mut elsewhere = start(&other, &["--listen", "127.0.0.1:0"]);
    assert_ne!(listening(&mut elsewhere), address);
    TcpStream::connect_timeout(&address, TIMEOUT).expect("the first relay still listens");
    elsewhere.send("s").unwrap();
    assert_stopped(&mut elsewhere, 0);

    first.send("s").unwrap();
    assert_stopped(&mut first, 0);
    // The home is free again once its relay has stopped.
    let mut again = start(&home, &["--listen", "127.0.0.1:0"]);
    listening(&mut again);
    again.send("s").unwrap();
    assert_stopped(&mut again, 0);
}

/// RLY-01 (X11), RLY-02: a port already in use is a start failure, 125, that
/// names the address and the log; the home is not left locked.
#[test]
fn a_port_in_use_is_a_start_failure() {
    let home = Isolation::new();
    let taken = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = taken.local_addr().unwrap();
    let mut terminal = start(&home, &["--listen", &address.to_string()]);
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    let screen = terminal.screen().replace('\n', "");
    let reason = format!("climon2: error: cannot listen on {address}: ");
    assert!(screen.contains(&reason), "{}", terminal.screen());
    let log = relay_log(&home);
    assert!(
        screen.contains(&format!("(logs: {})", log.display())),
        "{}",
        terminal.screen()
    );
    assert!(!screen.contains("Listening"), "{}", terminal.screen());
    assert_eq!(terminal.modes(), Modes::BASELINE);
    fs::remove_file(log).unwrap();
    let mut free = start(&home, &["--listen", "127.0.0.1:0"]);
    listening(&mut free);
    free.send("s").unwrap();
    assert_stopped(&mut free, 0);
}

/// ADV-03 (U3): started inside a nested session, the relay refuses to
/// start, before it binds or touches the terminal; inside a session at
/// depth 1 it starts.
#[test]
fn adv_03_the_relay_refuses_to_start_in_a_nested_session() {
    let home = Isolation::new();
    let session = ("CLIMON2_SESSION_ID", "0d5c0000-0000-4000-8000-000000000001");
    let mut nested = home.command(RELAY);
    nested.args(["--listen", "127.0.0.1:0"]);
    nested.env(session.0, session.1);
    nested.env("CLIMON2_SESSION_DEPTH", "2");
    let mut terminal = Terminal::spawn(nested, ROWS, COLS).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    let screen = terminal.screen().replace('\n', "");
    let reason =
        "climon2: error: c2 relay cannot start in a nested session (this one is at depth 2)";
    assert!(screen.contains(reason), "{}", terminal.screen());
    assert!(!screen.contains("Listening"), "{}", terminal.screen());
    assert_eq!(terminal.modes(), Modes::BASELINE);
    fs::remove_file(relay_log(&home)).unwrap();

    let mut inside = home.command(RELAY);
    inside.args(["--listen", "127.0.0.1:0"]);
    inside.env(session.0, session.1);
    inside.env("CLIMON2_SESSION_DEPTH", "1");
    let mut terminal = Terminal::spawn(inside, ROWS, COLS).unwrap();
    listening(&mut terminal);
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
}

fn configure(home: &Isolation, text: &str) {
    fs::write(home.climon2_home().join("config.jsonc"), text).unwrap();
}

/// RLY-03 (CM2): `relay.listen` names the address when no flag does, and the
/// flag overrides it.
#[test]
fn relay_listen_is_used_and_the_flag_overrides_it() {
    let home = Isolation::new();
    let configured = free_port();
    configure(
        &home,
        &format!(r#"{{"relay": {{"listen": "127.0.0.1:{configured}"}}}}"#),
    );
    let mut terminal = start(&home, &[]);
    assert_eq!(
        listening(&mut terminal),
        SocketAddr::from(([127, 0, 0, 1], configured))
    );
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);

    let flagged = free_port();
    assert_ne!(flagged, configured);
    let mut terminal = start(&home, &["--listen", &format!("127.0.0.1:{flagged}")]);
    assert_eq!(
        listening(&mut terminal),
        SocketAddr::from(([127, 0, 0, 1], flagged))
    );
    TcpListener::bind(("127.0.0.1", configured)).expect("the setting's port stays free");
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
}

/// RLY-03 (CM3): with no flag and no setting the relay listens on
/// 127.0.0.1:7222. Another relay, or another run of this test, may hold that
/// port on this machine: then the start failure names the same address.
#[test]
fn the_default_listen_address_is_loopback_7222() {
    let home = Isolation::new();
    let mut terminal = start(&home, &[]);
    let listening_on_default = |screen: &str| {
        screen.contains("Listening")
            && screen
                .lines()
                .nth(1)
                .is_some_and(|row| row.trim() == "127.0.0.1:7222")
    };
    let refused = "cannot listen on 127.0.0.1:7222: ";
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            listening_on_default(screen) || screen.replace('\n', "").contains(refused)
        })
        .unwrap();
    if listening_on_default(&terminal.screen()) {
        terminal.send("s").unwrap();
        assert_stopped(&mut terminal, 0);
    } else {
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    }
}

/// RLY-03 (CM4): an IPv6 literal is a listen value, shown in brackets.
#[test]
fn an_ipv6_literal_is_listened_on() {
    if TcpListener::bind("[::1]:0").is_err() {
        eprintln!("skipped: this host has no IPv6 loopback");
        return;
    }
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "[::1]:0"]);
    let address = listening(&mut terminal);
    assert!(address.is_ipv6(), "{address}");
    assert!(terminal.row(1).trim().starts_with("[::1]:"));
    TcpStream::connect_timeout(&address, TIMEOUT).expect("the listener is bound");
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
}

/// RLY-02 (CM5): bad configuration is a start failure, 125, that names the
/// setting and the log, and binds nothing.
#[test]
fn bad_configuration_is_a_start_failure() {
    let home = Isolation::new();
    configure(&home, r#"{"relay": {"listen": "localhost:7222"}}"#);
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    let screen = terminal.screen().replace('\n', "");
    assert!(
        screen.contains(
            "climon2: error: config.jsonc is invalid: relay.listen must be an IP address and a port"
        ),
        "{}",
        terminal.screen()
    );
    assert!(screen.contains("(logs: "), "{}", terminal.screen());
    assert!(!screen.contains("Listening"));
    assert!(!home.climon2_home().join("links").exists());
}

/// RLY-02: a clean stop names no log, and the relay's log records how the
/// run went, under the relay role.
#[test]
fn a_clean_stop_names_no_log_and_the_log_records_the_run() {
    let home = Isolation::new();
    let mut terminal = start(&home, &["--listen", "127.0.0.1:0"]);
    let address = listening(&mut terminal);
    terminal.send("s").unwrap();
    assert_stopped(&mut terminal, 0);
    let recorded = fs::read_to_string(relay_log(&home)).unwrap();
    assert!(recorded.contains(" role=relay "), "{recorded}");
    assert!(recorded.contains("relay started"), "{recorded}");
    assert!(
        recorded.contains(&format!("listen={address}")),
        "{recorded}"
    );
    assert!(recorded.contains("relay stopped"), "{recorded}");
    assert!(recorded.contains("status=0"), "{recorded}");
    assert_links_are_private(&home.climon2_home());
}

/// The links folder and everything in it is owner-only.
fn assert_links_are_private(home: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let links = home.join("links");
    let mode = |path: &Path| fs::symlink_metadata(path).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode(&links), 0o700);
    let files: Vec<_> = fs::read_dir(&links)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!files.is_empty());
    for file in files {
        assert_eq!(mode(&file), 0o600, "{}", file.display());
    }
}
