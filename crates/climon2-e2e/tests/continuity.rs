//! Plan section 6.1: session continuity through the installed product, from
//! controlled bash 5 outer terminals with isolated homes. Every scenario ends
//! with `assert_clean()`.
#![cfg(any(target_os = "macos", target_os = "linux"))]

#[allow(dead_code)]
mod support;

use climon2_test_term::{Modes, Options, Terminal, controlled_bash};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};
use support::{
    DETACH, Package, TIMEOUT, assert_released, detach, kill_session, outer, quote, wait_file,
};

const TAKE_CONTROL: &[u8] = b"\x14";
const PLACEHOLDER: &str = "This session is being viewed elsewhere.";
const NOTICE: &str = "[climon2: older session scrollback was discarded]";
const ROWS: u16 = 30;
const COLS: u16 = 160;

/// An executable script run by the controlled bash.
fn script(package: &Package, bash: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = package.isolation.home().join(name);
    fs::write(&path, format!("#!{}\nset -u\n{body}", bash.display())).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn climon2(package: &Package, args: &[&str], rows: u16, cols: u16) -> Terminal {
    climon2_with(package, args, rows, cols, Options::default())
}

fn climon2_with(
    package: &Package,
    args: &[&str],
    rows: u16,
    cols: u16,
    options: Options,
) -> Terminal {
    let mut command = package.command("climon2");
    command.args(args);
    Terminal::spawn_with(command, rows, cols, options).unwrap()
}

fn exec_bash(package: &Package, bash: &Path) -> Terminal {
    let mut command = package.command("climon2");
    command.arg("exec");
    command.arg(bash);
    command.args(["--noprofile", "--norc", "-i"]);
    command.env("PS1", "INNER> ");
    let mut terminal = Terminal::spawn(command, ROWS, COLS).unwrap();
    prompt(&mut terminal);
    terminal
}

fn prompt(terminal: &mut Terminal) {
    terminal.wait_for_text("INNER> ", TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
}

/// Waits for `marker`, then for a prompt printed after it.
fn prompt_after(terminal: &mut Terminal, marker: &str) {
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .rsplit_once(marker)
                .is_some_and(|(_, after)| after.contains("INNER> "))
        })
        .unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
}

/// Runs a foreground command that never reads the terminal, so a kill ends the
/// shell with the hang-up signal rather than a prompt read failure (see
/// `RECORDER`).
fn hold_without_reading(terminal: &mut Terminal) {
    terminal
        .send("printf 'HOLD%s\\n' ING; sleep 1000\n")
        .unwrap();
    terminal.wait_for_text("HOLDING", TIMEOUT).unwrap();
}

/// An outer controlled bash that fills its host with sentinels, then becomes
/// `climon2 <climon2>`, so the host is occupied when the client starts.
fn occupied(package: &Package, bash: &Path, climon2: &str) -> Terminal {
    let mut command = package.command(bash);
    command.args(["--noprofile", "--norc", "-c"]);
    command.arg(format!(
        "for i in 1 2 3; do echo OUTER-SENTINEL-$i; done; PS1='INNER> ' exec climon2 {climon2}"
    ));
    Terminal::spawn(command, ROWS, COLS).unwrap()
}

fn session_id(package: &Package) -> String {
    package.metadata().session_id.to_string()
}

fn wait_contents(path: &Path, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        let contents = fs::read(path).unwrap_or_default();
        if done(&contents) {
            return contents;
        }
        assert!(
            Instant::now() < deadline,
            "{} stayed {:?}",
            path.display(),
            String::from_utf8_lossy(&contents)
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Native scrollback then the visible screen, one trimmed line per row.
fn lines(terminal: &Terminal) -> Vec<String> {
    let mut lines = terminal.history();
    lines.extend(
        terminal
            .screen()
            .lines()
            .map(|line| line.trim_end().to_owned()),
    );
    lines
}

/// The rows that are exactly `<prefix><n>`, as their numbers in host order.
fn numbered(terminal: &Terminal, prefix: &str) -> Vec<u32> {
    lines(terminal)
        .iter()
        .filter_map(|line| line.strip_prefix(prefix)?.parse().ok())
        .collect()
}

fn client_logs(package: &Package, id: &str) -> Vec<String> {
    let directory = package.isolation.climon2_home().join("logs").join(id);
    let mut logs: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("client-"))
        .collect();
    logs.sort();
    logs
}

fn assert_client_logs(package: &Package, id: &str, count: usize) {
    let logs = client_logs(package, id);
    assert_eq!(logs.len(), count, "{logs:?}");
    let directory = package.isolation.climon2_home().join("logs").join(id);
    let mut pids: Vec<_> = logs
        .iter()
        .map(|name| {
            let log = fs::read_to_string(directory.join(name)).unwrap();
            let header = log.lines().next().unwrap_or_default();
            assert!(header.contains("role=client"), "{name}: {header}");
            name.rsplit_once('-').unwrap().1.to_owned()
        })
        .collect();
    pids.sort();
    pids.dedup();
    assert_eq!(
        pids.len(),
        count,
        "each invocation has its own log: {logs:?}"
    );
}

fn configure(package: &Package, config: &str) {
    fs::create_dir_all(package.isolation.climon2_home()).unwrap();
    fs::write(
        package.isolation.climon2_home().join("config.jsonc"),
        config,
    )
    .unwrap();
}

#[test]
fn installed_session_after_vim_keeps_outer_shell_input_below_restored_output() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let path = package.isolation.home().join("vim.txt");
    fs::write(&path, "vim-cursor-regression\n").unwrap();
    let mut command = package.bash(&bash);
    command.env("PS1", "OUTER> ");
    let mut terminal = Terminal::spawn(command, ROWS, COLS).unwrap();
    terminal.wait_for_text("OUTER> ", TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    terminal
        .send(format!(
            "printf 'OUTER-HISTORY\\n'; PS1='INNER> ' climon2 exec {} --noprofile --norc -i; printf 'OUTER-%s\\n' RETURN\n",
            quote(&bash)
        ))
        .unwrap();
    prompt(&mut terminal);
    terminal
        .send(format!(
            "printf 'SESSION-%s\\n' HISTORY; /usr/bin/vim -Nu NONE -i NONE -n {}\n",
            quote(&path)
        ))
        .unwrap();
    terminal
        .wait_for_text("vim-cursor-regression", TIMEOUT)
        .unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    terminal.send(":q!\r").unwrap();
    prompt(&mut terminal);
    terminal.send("exit\n").unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen
                .rsplit_once("OUTER-RETURN")
                .is_some_and(|(_, after)| after.contains("OUTER> "))
        })
        .unwrap();
    terminal.wait_for_text("OUTER> ", TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    terminal.send("echo OUTER-INPUT\n").unwrap();
    terminal.wait_for_text("OUTER-INPUT", TIMEOUT).unwrap();
    let screen = terminal.screen();
    let history = screen
        .lines()
        .position(|line| line.trim_end() == "SESSION-HISTORY")
        .unwrap_or_else(|| panic!("restored output was overwritten:\n{screen}"));
    let input = screen
        .lines()
        .position(|line| line.starts_with("OUTER> echo OUTER-INPUT"))
        .expect("outer shell input remains visible");
    assert!(
        input > history,
        "outer input overwrote restored output:\n{screen}"
    );
    terminal.send("exit\n").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    package.assert_clean();
}

/// Section 6.1.1: history and output produced while detached are restored on
/// an occupied host after attaching by an 8-hex prefix; typing works; a
/// second detach/reattach works; each client invocation has its own log and
/// the attached client returns the command's status.
#[test]
fn installed_session_detaches_and_reattaches_on_an_occupied_host() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let exec = format!("exec {} --noprofile --norc -i", quote(&bash));
    let mut first = occupied(&package, &bash, &exec);
    prompt(&mut first);
    first
        .send("for i in $(seq 1 60); do echo history-$i; done\n")
        .unwrap();
    first.wait_for_text("history-60", TIMEOUT).unwrap();
    first
        .send("until [ -e ~/go ]; do sleep 0.05; done; echo detached-output; : > ~/done\n")
        .unwrap();
    // Detaching discards input not yet written to the PTY; wait for the echo.
    first.wait_for_text(": > ~/done", TIMEOUT).unwrap();
    let id = session_id(&package);
    detach(&mut first, &id);
    drop(first);

    fs::write(package.isolation.home().join("go"), "").unwrap();
    wait_file(&package.isolation.home().join("done"));
    let sessions = package.list();
    assert_eq!(
        sessions[0]["state"],
        json!({"lifecycle": "detached", "attachments": 0, "controlled": false})
    );

    let mut second = occupied(&package, &bash, &format!("attach {}", &id[..8]));
    second.wait_for_text("detached-output", TIMEOUT).unwrap();
    prompt(&mut second);
    assert_eq!(numbered(&second, "history-"), (1..=60).collect::<Vec<_>>());
    let history = second.history();
    let sentinels: Vec<_> = history
        .iter()
        .filter(|line| line.starts_with("OUTER-SENTINEL-"))
        .collect();
    assert_eq!(
        sentinels,
        ["OUTER-SENTINEL-1", "OUTER-SENTINEL-2", "OUTER-SENTINEL-3"],
        "outer content stays in the host's scrollback"
    );
    second.send("echo typed-$((2+3))\n").unwrap();
    second.wait_for_text("typed-5", TIMEOUT).unwrap();
    detach(&mut second, &id);
    drop(second);

    let mut third = climon2(&package, &["attach", &id], ROWS, COLS);
    third.wait_for_text("typed-5", TIMEOUT).unwrap();
    prompt(&mut third);
    third.send("exit 7\n").unwrap();
    assert_eq!(third.wait_exit(TIMEOUT).unwrap(), 7);
    drop(third);
    assert_client_logs(&package, &id, 3);
    assert!(package.list().is_empty());
    package.assert_clean();
}

/// Section 6.1.2: an alternate-screen application is shown again after
/// reattaching; quitting it shows the restored normal screen, and restored
/// history is in the host's normal scrollback, not lost to the alternate one.
#[test]
fn installed_alternate_screen_is_reattached_and_quitting_shows_normal_content() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let tui = script(
        &package,
        &bash,
        "alternate.sh",
        "printf '\\e[?1049h\\e[H\\e[2JALT-DISPLAY'\nIFS= read -rsn1 _\nprintf '\\e[?1049l'\necho after-alt\n",
    );
    let mut first = exec_bash(&package, &bash);
    first
        .send("for i in $(seq 1 50); do echo hist-$i; done; echo NORMAL-$((1+1))\n")
        .unwrap();
    first.wait_for_text("NORMAL-2", TIMEOUT).unwrap();
    first.send(format!("{}\n", quote(&tui))).unwrap();
    first.wait_for_text("ALT-DISPLAY", TIMEOUT).unwrap();
    let id = session_id(&package);
    detach(&mut first, &id);
    drop(first);

    let mut second = climon2(&package, &["attach", &id], ROWS, COLS);
    second.wait_for_text("ALT-DISPLAY", TIMEOUT).unwrap();
    second.wait_for_input_ready(TIMEOUT).unwrap();
    assert!(second.modes().alternate_screen);
    assert!(!second.screen().contains("NORMAL-2"));
    second.send("q").unwrap();
    second.wait_for_text("after-alt", TIMEOUT).unwrap();
    prompt(&mut second);
    assert!(!second.modes().alternate_screen);
    assert!(second.screen().contains("NORMAL-2"), "{}", second.screen());
    assert_eq!(numbered(&second, "hist-"), (1..=50).collect::<Vec<_>>());
    second.send("exit 4\n").unwrap();
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 4);
    drop(second);
    package.assert_clean();
}

/// Section 6.1.3: an attach takes control and the previous client shows the
/// placeholder; Ctrl+T takes control back; the non-controller's input is
/// discarded; the controller's size drives the session; a detach leaves the
/// other client running; command exit reaches every attached client.
#[test]
fn installed_clients_share_a_session_with_exactly_one_controller() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let mut a = exec_bash(&package, &bash);
    let id = session_id(&package);
    let mut b = climon2(&package, &["attach", &id], 40, 100);
    prompt(&mut b);
    a.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    a.wait_for_text("Press Ctrl+T to take control", TIMEOUT)
        .unwrap();
    b.send("stty size\n").unwrap();
    b.wait_for_text("40 100", TIMEOUT).unwrap();

    a.send(TAKE_CONTROL).unwrap();
    b.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    a.wait_for_text("40 100", TIMEOUT).unwrap();
    a.wait_for_input_ready(TIMEOUT).unwrap();
    b.send("echo B-STALE\n").unwrap();
    a.send("stty size; echo A-$((6*7))\n").unwrap();
    a.wait_for_text("A-42", TIMEOUT).unwrap();
    assert!(a.screen().contains("30 160"), "{}", a.screen());
    assert!(!a.screen().contains("B-STALE"), "{}", a.screen());

    detach(&mut b, &id);
    drop(b);
    a.send("echo A-CONTINUES-$((1+1))\n").unwrap();
    a.wait_for_text("A-CONTINUES-2", TIMEOUT).unwrap();
    let mut c = climon2(&package, &["attach", &id], ROWS, COLS);
    prompt(&mut c);
    a.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    c.send("exit 5\n").unwrap();
    assert_eq!(c.wait_exit(TIMEOUT).unwrap(), 5);
    assert_eq!(a.wait_exit(TIMEOUT).unwrap(), 5);
    drop((a, c));
    assert_client_logs(&package, &id, 3);
    package.assert_clean();
}

/// Section 6.1.5: kill ends a detached session silently with 0, and an
/// attached one while clients are attached, which receive the signal-derived
/// status. Unknown selectors are 125 and malformed ones 2. Every client
/// invocation (exec, attach, reattach and the kill helper) has its own log.
#[test]
fn installed_kill_ends_detached_and_attached_sessions() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let mut first = exec_bash(&package, &bash);
    let id = session_id(&package);
    detach(&mut first, &id);
    drop(first);
    let mut kill = climon2(&package, &["kill", &id], ROWS, COLS);
    assert_eq!(kill.wait_exit(TIMEOUT).unwrap(), 0);
    assert_eq!(kill.screen().trim(), "", "kill is silent");
    drop(kill);
    assert!(package.list().is_empty());

    for (args, status) in [
        (["kill", "deadbeef"], 125),
        (["attach", "deadbeef"], 125),
        (["attach", "1234567"], 125),
        (["kill", "not-hex"], 2),
        (["attach", "---"], 2),
    ] {
        let mut refused = climon2(&package, &args, ROWS, COLS);
        assert_eq!(refused.wait_exit(TIMEOUT).unwrap(), status, "{args:?}");
        assert!(refused.screen().contains("climon2:"), "{args:?}");
    }

    let mut a = exec_bash(&package, &bash);
    let id = session_id(&package);
    let mut b = climon2(&package, &["attach", &id], ROWS, COLS);
    prompt(&mut b);
    detach(&mut b, &id);
    drop(b);
    let mut c = climon2(&package, &["attach", &id[..8]], ROWS, COLS);
    prompt(&mut c);
    hold_without_reading(&mut c);
    let mut kill = climon2(&package, &["kill", &id[..8]], ROWS, COLS);
    assert_eq!(kill.wait_exit(TIMEOUT).unwrap(), 0);
    assert_eq!(kill.screen().trim(), "", "kill is silent");
    assert_eq!(c.wait_exit(TIMEOUT).unwrap(), 129);
    assert_eq!(a.wait_exit(TIMEOUT).unwrap(), 129);
    drop((a, c, kill));
    assert_client_logs(&package, &id, 4);
    assert!(package.list().is_empty());
    package.assert_clean();
}

/// Section 6.1.6: attaching to the enclosing session is refused with 125 and
/// leaves the session and its controller undisturbed.
#[test]
fn installed_self_attach_is_refused_without_disturbing_the_session() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let mut terminal = exec_bash(&package, &bash);
    terminal
        .send("climon2 attach \"$CLIMON2_SESSION_ID\"; echo SELF-STATUS=$?\n")
        .unwrap();
    terminal.wait_for_text("SELF-STATUS=125", TIMEOUT).unwrap();
    assert!(terminal.screen().contains("climon2: error:"));
    prompt(&mut terminal);
    assert_eq!(
        package.list()[0]["state"],
        json!({"lifecycle": "attached", "attachments": 1, "controlled": true})
    );
    terminal.send("echo still-$((1+1))\n").unwrap();
    terminal.wait_for_text("still-2", TIMEOUT).unwrap();
    terminal.send("exit 0\n").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    package.assert_clean();
}

/// Section 6.1.7: an invalid config is refused before any session exists.
#[test]
fn installed_invalid_config_is_refused_before_a_session_exists() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    configure(&package, r#"{"client": {"take_control_key": "Ctrl+["}}"#);
    let mut command = package.command("climon2");
    command.arg("exec");
    command.arg(&bash);
    command.args(["-c", "echo SHOULD-NOT-RUN"]);
    let mut terminal = Terminal::spawn(command, ROWS, COLS).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    terminal
        .wait_for_text("take_control_key may not be Ctrl+[", TIMEOUT)
        .unwrap();
    assert!(!terminal.screen().contains("SHOULD-NOT-RUN"));
    drop(terminal);
    assert!(package.list().is_empty());
    let logs = package.isolation.climon2_home().join("logs");
    let directories: Vec<_> = fs::read_dir(logs)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(directories, ["general"], "no session logs");
    package.assert_clean();
}

/// Section 6.1.7: a custom escape prefix detaches and a null takeover key
/// hides the instruction and leaves Ctrl+T as ordinary input.
#[test]
fn installed_config_rebinds_the_client_keys() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    configure(
        &package,
        "{\n  // rebinding\n  \"client\": {\"escape_prefix\": \"Ctrl+A\", \"take_control_key\": null,},\n}\n",
    );
    let mut a = exec_bash(&package, &bash);
    let id = session_id(&package);
    let mut b = climon2(&package, &["attach", &id], ROWS, COLS);
    prompt(&mut b);
    a.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    a.send(TAKE_CONTROL).unwrap();
    b.send("echo B-CONTROLS-$((1+1))\n").unwrap();
    b.wait_for_text("B-CONTROLS-2", TIMEOUT).unwrap();
    assert!(!a.screen().contains("Press"), "{}", a.screen());
    assert!(a.screen().contains(PLACEHOLDER));
    b.wait_for_input_ready(TIMEOUT).unwrap();
    b.send(DETACH).unwrap();
    b.send("\x15echo LITERAL-$((2+2))\n").unwrap();
    b.wait_for_text("LITERAL-4", TIMEOUT).unwrap();
    b.wait_for_input_ready(TIMEOUT).unwrap();
    b.send("\x01d").unwrap();
    assert_eq!(b.wait_exit(TIMEOUT).unwrap(), 0);
    b.wait_for_bytes(format!("detached from session {id}").as_bytes(), TIMEOUT)
        .unwrap();
    drop(b);
    a.send("\x01d").unwrap();
    assert_eq!(a.wait_exit(TIMEOUT).unwrap(), 0);
    drop(a);
    let mut kill = climon2(&package, &["kill", &id], ROWS, COLS);
    assert_eq!(kill.wait_exit(TIMEOUT).unwrap(), 0);
    drop(kill);
    package.assert_clean();
}

fn retention_session(package: &Package, bash: &Path) -> Terminal {
    let mut first = exec_bash(package, bash);
    first
        .send("clear; for i in $(seq 1 40); do echo row-$i; done\n")
        .unwrap();
    first.wait_for_text("row-40", TIMEOUT).unwrap();
    prompt(&mut first);
    let id = session_id(package);
    detach(&mut first, &id);
    drop(first);
    let mut second = climon2(package, &["attach", &id], ROWS, COLS);
    second.wait_for_text("row-40", TIMEOUT).unwrap();
    prompt(&mut second);
    second
}

/// Section 6.1.7: a small `max_rows` keeps only the newest history rows,
/// preceded by the truncation notice; `max_rows: 0` restores the screen
/// without history or notice.
#[test]
fn installed_scrollback_max_rows_bounds_restored_history() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    configure(&package, r#"{"scrollback": {"max_rows": 5}}"#);
    let mut small = retention_session(&package, &bash);
    let on_screen: Vec<u32> = (40 - (u32::from(ROWS) - 2)..=40).collect();
    let first_shown = on_screen[0];
    let mut expected: Vec<u32> = (first_shown - 5..first_shown).collect();
    expected.extend(&on_screen);
    assert_eq!(numbered(&small, "row-"), expected, "{:?}", lines(&small));
    let all = lines(&small);
    let notice = all.iter().position(|line| line == NOTICE).unwrap();
    assert_eq!(all[notice + 1], format!("row-{}", first_shown - 5));
    small.send("exit 0\n").unwrap();
    assert_eq!(small.wait_exit(TIMEOUT).unwrap(), 0);
    drop(small);

    configure(&package, r#"{"scrollback": {"max_rows": 0}}"#);
    let mut none = retention_session(&package, &bash);
    assert_eq!(numbered(&none, "row-"), on_screen, "{:?}", lines(&none));
    assert!(!lines(&none).iter().any(|line| line == NOTICE));
    none.send("exit 0\n").unwrap();
    assert_eq!(none.wait_exit(TIMEOUT).unwrap(), 0);
    drop(none);
    package.assert_clean();
}

/// Asks DA1 then DSR 5, the first split across writes, and stores every
/// reply byte up to the DSR answer in `$1`, then prints `$2`.
const QUERY: &str = r#"stty raw -echo
until [ -e "$1.go" ]; do sleep 0.05; done
printf '\e['; sleep 0.2; printf 'c'; sleep 0.2; printf '\e[5n'
IFS= read -rd n reply
printf '%s' "$reply" > "$1.tmp"; mv "$1.tmp" "$1"
printf '%s\r\n' "$2"
stty sane
"#;

/// Section 6.1.8: with harness replies disabled, the daemon alone answers
/// DA1/DSR exactly once whether attached or detached, including a query
/// split across reads; host-dependent requests and OSC 52 never reach the
/// host; a CPR-shaped key typed by the user reaches the application.
#[test]
fn installed_session_answers_queries_without_the_host() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    script(&package, &bash, "query.sh", QUERY);
    let home = package.isolation.home().to_owned();
    let silent = Options {
        replies: false,
        ..Options::default()
    };
    let mut command = package.command("climon2");
    command.arg("exec");
    command.arg(&bash);
    command.args(["--noprofile", "--norc", "-i"]);
    command.env("PS1", "INNER> ");
    let mut first = Terminal::spawn_with(command, ROWS, COLS, silent).unwrap();
    prompt(&mut first);
    let id = session_id(&package);

    let attached = home.join("attached");
    first.send("~/query.sh ~/attached ATTACHED-DONE\n").unwrap();
    fs::write(home.join("attached.go"), "").unwrap();
    prompt_after(&mut first, "\nATTACHED-DONE");
    assert_eq!(fs::read(&attached).unwrap(), b"\x1b[?6c\x1b[0");

    let detached = home.join("detached");
    first.send("~/query.sh ~/detached DETACHED-DONE\n").unwrap();
    first.wait_for_text("DETACHED-DONE", TIMEOUT).unwrap();
    detach(&mut first, &id);
    let first_requests = first.host_requests();
    drop(first);
    fs::write(home.join("detached.go"), "").unwrap();
    assert_eq!(
        wait_contents(&detached, |reply| !reply.is_empty()),
        b"\x1b[?6c\x1b[0"
    );

    let mut second = climon2_with(&package, &["attach", &id], ROWS, COLS, silent);
    prompt_after(&mut second, "\nDETACHED-DONE");
    second
        .send(concat!(
            "printf '\\e[?6n\\e]11;?\\a\\e[14t\\e]52;c;aGVsbG8=\\a\\e]52;c;?\\a'; ",
            "echo HOST-$((3*3))\n"
        ))
        .unwrap();
    prompt_after(&mut second, "HOST-9");

    second
        .send("stty raw -echo; printf KEY-READY; IFS= read -rd R key; stty sane; printf '\\nKEY=%q\\n' \"$key\"\n")
        .unwrap();
    second.wait_for_text("KEY-READY", TIMEOUT).unwrap();
    second.wait_for_input_ready(TIMEOUT).unwrap();
    second.send("\x1b[3;5R").unwrap();
    prompt_after(&mut second, "KEY=$'\\E[3;5'");
    assert_eq!(first_requests, Vec::<String>::new());
    assert_eq!(second.host_requests(), Vec::<String>::new());
    second.send("exit 0\n").unwrap();
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 0);
    drop(second);
    package.assert_clean();
}

/// Section 6.1.9: the default title is visible after attach; a title holding
/// a control character arrives percent-encoded and an OSC 8 link whose target
/// holds a control byte arrives as plain text, both live and restored.
#[test]
fn installed_titles_are_encoded_and_links_are_plain_text() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let mut first = exec_bash(&package, &bash);
    let default = format!("climon2 exec {} --noprofile --norc -i", bash.display());
    assert_eq!(first.title().as_deref(), Some(default.as_str()));
    first
        .send(concat!(
            "printf '\\e]2;A\\xc2\\x85B\\a\\e]8;;http://x/\\x01y\\aLINK-TEXT\\e]8;;\\a\\n'; ",
            "echo SHOWN-$((2*4))\n"
        ))
        .unwrap();
    prompt_after(&mut first, "SHOWN-8");
    let encoded = "A%C2%85B";
    assert_eq!(first.title().as_deref(), Some(encoded));
    assert!(first.screen().contains("\nLINK-TEXT"), "{}", first.screen());
    let live = first.received();
    assert!(!contains(&live, b"\x1b]8;"), "raw OSC 8 reached the host");
    let id = session_id(&package);
    detach(&mut first, &id);
    drop(first);

    let mut second = climon2(&package, &["attach", &id], ROWS, COLS);
    prompt_after(&mut second, "SHOWN-8");
    assert_eq!(second.title().as_deref(), Some(encoded));
    assert!(
        second.screen().contains("\nLINK-TEXT"),
        "{}",
        second.screen()
    );
    assert!(!contains(&second.received(), b"\x1b]8;"));
    second.send("exit 0\n").unwrap();
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 0);
    drop(second);

    let mut third = exec_bash(&package, &bash);
    let id = session_id(&package);
    detach(&mut third, &id);
    drop(third);
    let mut fourth = climon2(&package, &["attach", &id], ROWS, COLS);
    prompt(&mut fourth);
    assert_eq!(fourth.title().as_deref(), Some(default.as_str()));
    fourth.send("exit 0\n").unwrap();
    assert_eq!(fourth.wait_exit(TIMEOUT).unwrap(), 0);
    drop(fourth);
    package.assert_clean();
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// Section 6.1.10: a large write completes with no client attached, and the
/// session stays live and attachable.
#[test]
fn installed_detached_session_absorbs_a_large_write() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let mut first = exec_bash(&package, &bash);
    first
        .send(concat!(
            "until [ -e ~/go ]; do sleep 0.05; done; ",
            "head -c 8388608 /dev/zero | tr '\\0' x | fold -w 128; ",
            "echo WRITTEN-$((2*4)); : > ~/done\n"
        ))
        .unwrap();
    first.wait_for_text(": > ~/done", TIMEOUT).unwrap();
    let id = session_id(&package);
    detach(&mut first, &id);
    drop(first);
    fs::write(package.isolation.home().join("go"), "").unwrap();
    wait_file(&package.isolation.home().join("done"));
    assert_eq!(package.list()[0]["state"]["lifecycle"], "detached");

    let mut second = climon2(&package, &["attach", &id], ROWS, COLS);
    prompt_after(&mut second, "WRITTEN-8");
    second.send("echo LIVE-$((5*5))\n").unwrap();
    prompt_after(&mut second, "LIVE-25");
    second.send("exit 0\n").unwrap();
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 0);
    drop(second);
    package.assert_clean();
}

/// A TUI that sets mouse reporting, focus, bracketed paste, a hidden cursor,
/// application keys and the alternate screen, then exits on any key without
/// undoing any of them.
/// Ends after one key; if its terminal closes first, it waits for the hang-up
/// signal like `RECORDER`. `dd` reads the key because `read -s`/`-n` install
/// bash's terminating-signal handlers, which can consume that signal.
const TUI: &str = r#"printf '\e[?1049h\e[?1000h\e[?1006h\e[?1004h\e[?2004h\e[?25l\e[?1h\e=\e[H\e[2JTUI-READY'
stty raw -echo
dd bs=1 count=1 >/dev/null 2>&1 || exec sleep 1000
"#;

/// Section 6.1.11: detach, natural exit with modes left set, kill, and a
/// non-controlling departure each return the host to the documented baseline
/// with exact termios and ordinary shell input.
#[test]
fn installed_client_releases_the_host_on_every_ending() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let tui = script(&package, &bash, "tui.sh", TUI);
    let exec = format!("climon2 exec {}", quote(&tui));
    let ready = |terminal: &mut Terminal| {
        terminal.wait_for_text("TUI-READY", TIMEOUT).unwrap();
        terminal.wait_for_input_ready(TIMEOUT).unwrap();
        assert!(terminal.modes().alternate_screen && terminal.modes().mouse);
    };

    let mut detached = outer(&package, &bash, &exec, ROWS, COLS);
    ready(&mut detached);
    let id = session_id(&package);
    detached.send(DETACH).unwrap();
    assert_released(&package, &mut detached, 0);
    drop(detached);
    kill_session(&package, &id);

    let mut natural = outer(&package, &bash, &exec, ROWS, COLS);
    ready(&mut natural);
    natural.send("q").unwrap();
    assert_released(&package, &mut natural, 0);
    drop(natural);

    let mut killed = outer(&package, &bash, &exec, ROWS, COLS);
    ready(&mut killed);
    let id = session_id(&package);
    kill_session(&package, &id);
    assert_released(&package, &mut killed, 129);
    drop(killed);

    let mut observer = outer(&package, &bash, &exec, ROWS, COLS);
    ready(&mut observer);
    let id = session_id(&package);
    let mut controller = climon2(&package, &["attach", &id], ROWS, COLS);
    ready(&mut controller);
    observer.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    observer.send(DETACH).unwrap();
    assert_released(&package, &mut observer, 0);
    drop(observer);
    controller.send("q").unwrap();
    assert_eq!(controller.wait_exit(TIMEOUT).unwrap(), 0);
    assert_eq!(controller.modes(), Modes::BASELINE);
    drop(controller);
    package.assert_clean();
}

/// Records every input byte in `$1` with bracketed paste enabled.
/// Linux wakes a terminal reader before it signals the session leader when a
/// killed session's terminal closes, so a leader still reading the terminal
/// can end with its own status (ADR 0003). The shell therefore leaves reading
/// to `cat` and waits for the hang-up signal once `cat` ends.
const RECORDER: &str = r#"stty raw -echo
printf '\e[?2004hREC-READY'
cat > "$1"
exec sleep 1000
"#;

fn recorder(package: &Package, bash: &Path, name: &str) -> (Terminal, PathBuf) {
    let script = script(package, bash, "recorder.sh", RECORDER);
    let path = package.isolation.home().join(name);
    let mut terminal = climon2(
        package,
        &["exec", script.to_str().unwrap(), path.to_str().unwrap()],
        ROWS,
        COLS,
    );
    terminal.wait_for_text("REC-READY", TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    (terminal, path)
}

fn attach_recorder(package: &Package, id: &str) -> Terminal {
    let mut terminal = climon2(package, &["attach", id], ROWS, COLS);
    terminal.wait_for_text("REC-READY", TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    terminal
}

fn wait_controlling(terminal: &mut Terminal) {
    terminal
        .wait_for_screen(TIMEOUT, |screen| {
            screen.contains("REC-READY") && !screen.contains(PLACEHOLDER)
        })
        .unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
}

fn is_client(pid: i32) -> bool {
    let output = std::process::Command::new("/bin/ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .unwrap();
    Path::new(String::from_utf8_lossy(&output.stdout).trim())
        .file_name()
        .is_some_and(|name| name == "climon2-client")
}

const PASTE_START: &str = "\x1b[200~first";
/// The rest of the paste, holding both client-action bindings.
const PASTE_REST: &str = "\x1dd\x14rest\x1b[201~";

/// Section 6.1.12: a fragmented paste interrupted by takeover, by the
/// controller's socket loss, and by a readiness change while the local paste
/// is still streaming. The application sees exactly one closing marker, no
/// stale payload and no client action, then the next controller's input.
#[test]
fn installed_interrupted_paste_is_closed_once_without_stale_payload() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let closed = |after: &str| format!("\x1b[200~first\x1b[201~{after}").into_bytes();

    let (mut a, rec) = recorder(&package, &bash, "takeover");
    let id = session_id(&package);
    a.send(PASTE_START).unwrap();
    wait_contents(&rec, |bytes| bytes.ends_with(b"first"));
    let mut b = attach_recorder(&package, &id);
    a.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    a.send(format!("{PASTE_REST}A-STALE")).unwrap();
    b.send("B-AFTER").unwrap();
    assert_eq!(
        wait_contents(&rec, |bytes| bytes.ends_with(b"B-AFTER")),
        closed("B-AFTER")
    );
    assert_eq!(
        package.list()[0]["state"],
        json!({"lifecycle": "attached", "attachments": 2, "controlled": true}),
        "the remaining payload took no client action"
    );
    assert!(a.screen().contains(PLACEHOLDER));
    kill_session(&package, &id);
    assert_eq!(a.wait_exit(TIMEOUT).unwrap(), 129);
    assert_eq!(b.wait_exit(TIMEOUT).unwrap(), 129);
    drop((a, b));

    let (mut b, rec) = recorder(&package, &bash, "loss");
    let id = session_id(&package);
    let mut a = attach_recorder(&package, &id);
    assert!(is_client(a.pid()), "the launcher becomes the client");
    a.send(PASTE_START).unwrap();
    wait_contents(&rec, |bytes| bytes.ends_with(b"first"));
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(a.pid()),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
    wait_controlling(&mut b);
    b.send("B-AFTER").unwrap();
    assert_eq!(
        wait_contents(&rec, |bytes| bytes.ends_with(b"B-AFTER")),
        closed("B-AFTER")
    );
    kill_session(&package, &id);
    assert_eq!(b.wait_exit(TIMEOUT).unwrap(), 129);
    drop((a, b));

    let (mut a, rec) = recorder(&package, &bash, "readiness");
    let id = session_id(&package);
    a.send(PASTE_START).unwrap();
    wait_contents(&rec, |bytes| bytes.ends_with(b"first"));
    let mut b = attach_recorder(&package, &id);
    a.wait_for_text(PLACEHOLDER, TIMEOUT).unwrap();
    detach(&mut b, &id);
    drop(b);
    wait_controlling(&mut a);
    a.send(format!("{PASTE_REST}A-AFTER")).unwrap();
    assert_eq!(
        wait_contents(&rec, |bytes| bytes.ends_with(b"A-AFTER")),
        closed("A-AFTER")
    );
    assert_eq!(package.list()[0]["state"]["attachments"], 1);
    kill_session(&package, &id);
    assert_eq!(a.wait_exit(TIMEOUT).unwrap(), 129);
    drop(a);
    package.assert_clean();
}

/// Section 6.1.13: a restoration prefix larger than 16 MiB reaches a healthy
/// host completely and in order, and numbered output produced during the
/// restoration is neither lost nor duplicated.
#[test]
fn installed_large_restoration_is_complete_and_ordered() {
    const ROWS_PRODUCED: u32 = 80_000;
    const LIVE: u32 = 3_000;
    const SLOW: Duration = Duration::from_secs(120);
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    configure(&package, r#"{"scrollback": {"max_rows": 100000}}"#);
    let mut first = exec_bash(&package, &bash);
    first
        .send(format!(
            "echo $((6*7))-producer-ready; until [ -e ~/go ]; do sleep 0.05; done; \
             awk 'BEGIN {{ for (i = 1; i <= {ROWS_PRODUCED}; i++) {{ printf \"%06d\", i; \
             for (j = 0; j < 73; j++) printf \"€\"; printf \"\\n\" }} }}'; : > ~/done; \
             until [ -e ~/live ]; do sleep 0.01; done; \
             for i in $(seq 1 {LIVE}); do echo live-$i; done; echo LIVE-$((2+2))\n"
        ))
        .unwrap();
    first.wait_for_text("42-producer-ready", TIMEOUT).unwrap();
    let id = session_id(&package);
    detach(&mut first, &id);
    drop(first);
    fs::write(package.isolation.home().join("go"), "").unwrap();
    let done = package.isolation.home().join("done");
    let deadline = Instant::now() + SLOW;
    while !done.exists() {
        assert!(Instant::now() < deadline, "production timed out");
        thread::sleep(Duration::from_millis(50));
    }

    let options = Options {
        history: 100_000,
        ..Options::default()
    };
    let mut command = package.command("climon2");
    command.args(["attach", &id]);
    let mut second = Terminal::spawn_with(command, ROWS, 80, options).unwrap();
    fs::write(package.isolation.home().join("live"), "").unwrap();
    second.wait_for_text("\nLIVE-4", SLOW).unwrap();
    second.wait_for_input_ready(TIMEOUT).unwrap();

    let lines = lines(&second);
    let produced: Vec<u32> = lines
        .iter()
        .filter(|line| line.chars().nth(6) == Some('€'))
        .map(|line| {
            assert_eq!(line.chars().count(), 79, "{line}");
            line[..6].parse().unwrap()
        })
        .collect();
    assert!(
        produced.iter().copied().eq(1..=ROWS_PRODUCED),
        "restored rows are complete and ordered: {} rows, first {:?}",
        produced.len(),
        produced.first()
    );
    assert_eq!(numbered(&second, "live-"), (1..=LIVE).collect::<Vec<_>>());
    second.send("exit 0\n").unwrap();
    assert_eq!(second.wait_exit(TIMEOUT).unwrap(), 0);
    drop(second);
    package.assert_clean();
}

/// Releases a script waiting with `WAITING` once the client is ready.
fn after_readiness(terminal: &mut Terminal) {
    terminal.wait_for_text("WAITING", TIMEOUT).unwrap();
    terminal.wait_for_input_ready(TIMEOUT).unwrap();
    terminal.send("g").unwrap();
}

/// Section 6.1.14: a 24x501 host shows a 24x500 session that wraps at its own
/// width and clamps cursor positioning to its own rectangle.
#[test]
fn installed_wide_host_presents_the_session_rectangle() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let wide = script(
        &package,
        &bash,
        "wide.sh",
        "stty raw -echo\nprintf WAITING; IFS= read -rsn1 _\nprintf '\\e[H\\e[2J'; printf '%500s' '' | tr ' ' a; printf b\nprintf '\\e[5;999HX\\e[12;1HWIDE-DONE'\nIFS= read -rsn1 _\n",
    );
    let mut terminal = climon2(&package, &["exec", wide.to_str().unwrap()], 24, 501);
    after_readiness(&mut terminal);
    terminal.wait_for_text("WIDE-DONE", TIMEOUT).unwrap();
    let first = terminal.row(0);
    assert_eq!(first.chars().filter(|&c| c == 'a').count(), 500);
    assert_eq!(
        first.chars().nth(500),
        Some(' '),
        "host column 501 is unused"
    );
    assert!(terminal.row(1).starts_with("b "), "{:?}", terminal.row(1));
    let fifth: Vec<char> = terminal.row(4).chars().collect();
    assert_eq!((fifth[499], fifth[500]), ('X', ' '));
    terminal.send("q").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    package.assert_clean();
}

/// Section 6.1.14: on a 201x80 host, a line feed on session row 200 scrolls
/// the session only: new text lands on host row 200, host row 201 stays blank
/// and the scrolled-off row is in the host's native scrollback.
#[test]
fn installed_tall_host_scrolls_the_session_rectangle() {
    let Some(bash) = controlled_bash() else {
        return;
    };
    let package = Package::new();
    let tall = script(
        &package,
        &bash,
        "tall.sh",
        "stty raw -echo\nprintf WAITING; IFS= read -rsn1 _\nprintf '\\e[H\\e[2JTOP-ROW\\e[200;1HBOTTOM\\nNEXT-TEXT\\e[300;20HY'\nIFS= read -rsn1 _\n",
    );
    let mut terminal = climon2(&package, &["exec", tall.to_str().unwrap()], 201, 80);
    after_readiness(&mut terminal);
    terminal.wait_for_text("NEXT-TEXT", TIMEOUT).unwrap();
    terminal
        .wait_for_screen(TIMEOUT, |screen| !screen.contains("TOP-ROW"))
        .unwrap();
    assert_eq!(terminal.row(198).trim_end(), "BOTTOM");
    // Raw mode: LF moves down without a carriage return.
    assert_eq!(terminal.row(199).trim_end(), "      NEXT-TEXT    Y");
    assert_eq!(terminal.row(200).trim_end(), "", "host row 201 is unused");
    assert!(
        terminal.history().iter().any(|line| line == "TOP-ROW"),
        "{:?}",
        terminal.history()
    );
    terminal.send("q").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    package.assert_clean();
}
