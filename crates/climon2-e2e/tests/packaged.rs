#![cfg(target_os = "macos")]

mod support;

use climon2_test_term::{Terminal, bash_profiles};
use serde_json::{Value, json};
use std::{fs, path::Path};
use support::{Package, TIMEOUT, quote};

/// #21.6/#38: real installer → PATH launcher → CLI → daemon → client, from
/// controlled outer bash to controlled inner bash; a second real PTY lists one
/// session, the command's status reaches the outer shell, then nothing survives.
#[test]
fn installer_built_bash_smoke_lists_one_session_and_preserves_exit_seven() {
    for bash in bash_profiles() {
        let package = Package::new();
        let mut command = package.bash(&bash);
        command.env("PS1", "OUTER> ");
        let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
        terminal.wait_for_text("OUTER> ", TIMEOUT).unwrap();
        terminal
            .send(format!(
                "PS1='INNER> ' climon2 exec {} --noprofile --norc -i; status=$?; printf '\\nOUTER-STATUS=%s\\n' \"$status\"; exit \"$status\"\n",
                quote(&bash)
            ))
            .unwrap();
        terminal.send("echo $((6*7))-ok\n").unwrap();
        terminal.wait_for_text("42-ok", TIMEOUT).unwrap();
        let sessions = package.list();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["depth"], 1);
        terminal.send("exit 7\n").unwrap();
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 7);
        terminal.wait_for_text("OUTER-STATUS=7", TIMEOUT).unwrap();
        drop(terminal);
        assert!(package.list().is_empty());
        package.assert_clean();
        package.assert_archive_default_levels();
    }
}

fn launch(package: &Package, program: &str, args: &[&str]) -> Terminal {
    let mut command = package.command("climon2");
    command.args(["exec", program]);
    command.args(args);
    Terminal::spawn(command, 30, 120).unwrap()
}

fn report(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

/// #16/#29/#38: the generic fixture observes exact argv, cwd and complete
/// inherited environment through the installer-built launcher, not metadata's
/// lossy display projection; initial size, live resize and final status survive.
/// #25: TRACE diagnostics contain neither argument, input nor environment data.
#[test]
fn installed_generic_fixture_preserves_runtime_contract_and_log_privacy() {
    let package = Package::new();
    let cwd = package
        .isolation
        .home()
        .join("working directory with spaces");
    fs::create_dir(&cwd).unwrap();
    let before = package.isolation.home().join("baseline.json");
    let initial = package.isolation.home().join("initial.json");
    let resized = package.isolation.home().join("resized.json");
    let configure = |command: &mut portable_pty::CommandBuilder| {
        command.cwd(&cwd);
        command.env("CLIMON2_LOG", "trace");
        command.env("E2E_SECRET", "env-secret-not-for-logs-91b");
        command.env("E2E_EMPTY", "");
    };
    let mut direct = package.command("climon2-fixture");
    configure(&mut direct);
    direct.args(["report", before.to_str().unwrap(), "exit", "0"]);
    let mut direct = Terminal::spawn(direct, 30, 120).unwrap();
    assert_eq!(direct.wait_exit(TIMEOUT).unwrap(), 0);
    drop(direct);

    let args = [
        "report",
        initial.to_str().unwrap(),
        "print",
        "argv-secret ' literal spaces -- -78",
        "print",
        "",
        "print",
        "resize-ready",
        "sleep",
        "5000",
        "report",
        resized.to_str().unwrap(),
        "exit",
        "23",
    ];
    let mut command = package.command("climon2");
    configure(&mut command);
    command.args(["exec", "climon2-fixture"]);
    command.args(args);
    let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
    terminal.wait_for_text("resize-ready", TIMEOUT).unwrap();
    terminal.send("input-secret-not-for-logs-72c").unwrap();
    terminal.resize(43, 137).unwrap();
    assert_eq!(
        terminal.wait_exit(TIMEOUT).unwrap(),
        23,
        "{}",
        terminal.screen()
    );
    drop(terminal);
    let actual = report(&initial);
    assert_eq!(actual["argv"], json!(args));
    // getcwd(3) reports the physical directory, e.g. /private/var for /var.
    assert_eq!(actual["cwd"], json!(cwd.canonicalize().unwrap()));
    assert_eq!(actual["size"], json!([30, 120]));
    assert_eq!(actual["isatty"], json!([true, true, true]));
    assert_eq!(report(&resized)["size"], json!([43, 137]));
    let mut environment = actual["env"].as_object().unwrap().clone();
    let id = environment.remove("CLIMON2_SESSION_ID").unwrap();
    assert!(
        id.as_str()
            .unwrap()
            .parse::<climon2_core::SessionId>()
            .is_ok()
    );
    assert_eq!(
        environment.remove("CLIMON2_SESSION_DEPTH"),
        Some(json!("1"))
    );
    assert_eq!(json!(environment), report(&before)["env"]);
    for path in package.logs() {
        let log = fs::read_to_string(path).unwrap();
        for secret in [
            "env-secret-not-for-logs-91b",
            "argv-secret",
            "input-secret-not-for-logs-72c",
        ] {
            assert!(!log.contains(secret), "session content leaked: {log}");
        }
    }
    package.assert_clean();
}

/// #16/#29/#38: bash launched from the installed layout can launch another
/// session through PATH; both are live and have distinct GUIDs/depths, and each
/// command's status returns through its own version-matched client.
#[test]
fn installed_nested_bash_sessions_are_both_listed_and_route_exit_status() {
    for bash in bash_profiles() {
        let package = Package::new();
        let mut command = package.command("climon2");
        command.args(["exec"]);
        command.arg(&bash);
        command.args(["--noprofile", "--norc", "-i"]);
        command.env("PS1", "ENCLOSING> ");
        let mut terminal = Terminal::spawn(command, 30, 160).unwrap();
        terminal.wait_for_text("ENCLOSING> ", TIMEOUT).unwrap();
        terminal
            .send(format!(
                "climon2 exec {} --noprofile --norc -i; s=$?; printf '\\nNESTED-STATUS=%s\\n' \"$s\"\n",
                quote(&bash)
            ))
            .unwrap();
        terminal
            .wait_for_text("Starting climon2 session nested for the 1st time", TIMEOUT)
            .unwrap();
        terminal
            .send("echo depth:$CLIMON2_SESSION_DEPTH\n")
            .unwrap();
        terminal.wait_for_text("depth:2", TIMEOUT).unwrap();
        let sessions = package.list();
        assert_eq!(sessions.len(), 2);
        assert_ne!(sessions[0]["session_id"], sessions[1]["session_id"]);
        let mut depths: Vec<_> = sessions
            .iter()
            .map(|session| session["depth"].clone())
            .collect();
        depths.sort_by_key(|depth| depth.as_u64().unwrap());
        assert_eq!(depths, [json!(1), json!(2)]);
        terminal.send("exit 7\n").unwrap();
        terminal.wait_for_text("NESTED-STATUS=7", TIMEOUT).unwrap();
        assert_eq!(package.list().len(), 1);
        terminal.send("exit 9\n").unwrap();
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 9);
        drop(terminal);
        assert!(package.list().is_empty());
        package.assert_clean();
    }
}

/// #16/#20/#38: alternate screen, automatic cursor query replies, SGR mouse,
/// bracketed paste and resize travel through every installed process. The
/// fixture's own final screen survives client exit (the client must not reset).
#[test]
fn installed_tui_roundtrips_queries_mouse_paste_and_resize_without_reset() {
    let package = Package::new();
    let path = package.isolation.home().join("tui.json");
    let mut terminal = launch(&package, "climon2-tui-fixture", &[path.to_str().unwrap()]);
    terminal.wait_for_bytes(b"\x1b[?1049h", TIMEOUT).unwrap();
    terminal.wait_for_text("cursor-ok", TIMEOUT).unwrap();
    terminal
        .send("\x1b[<0;7;9M\x1b[200~paste-exact\x1b[201~")
        .unwrap();
    terminal
        .wait_for_text("mouse-ok paste-ok", TIMEOUT)
        .unwrap();
    terminal.resize(41, 132).unwrap();
    terminal.send("r").unwrap();
    terminal.wait_for_text("size=41x132", TIMEOUT).unwrap();
    terminal.send([4]).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    assert!(terminal.screen().contains("tui-done"));
    drop(terminal);
    let actual = report(&path);
    assert_eq!(actual["cursor"], "\x1b[3;5R");
    assert_eq!(actual["mouse"], "\x1b[<0;7;9M");
    assert_eq!(actual["paste"], "paste-exact");
    assert_eq!(actual["size"], json!([41, 132]));
    package.assert_clean();
}

/// #16/#21.10/#38: real macOS vim, not the TUI fixture, edits and saves through
/// the installed launcher and returns normally without client screen resets.
#[test]
fn installed_real_vim_edits_and_saves() {
    let package = Package::new();
    let path = package.isolation.home().join("vim.txt");
    fs::write(&path, "vim-original\n").unwrap();
    let mut terminal = launch(
        &package,
        "/usr/bin/vim",
        &["-Nu", "NONE", "-i", "NONE", "-n", path.to_str().unwrap()],
    );
    terminal.wait_for_text("vim-original", TIMEOUT).unwrap();
    terminal.send("Govim-through-session\x1b:wq\r").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    assert_eq!(
        fs::read_to_string(path).unwrap(),
        "vim-original\nvim-through-session\n"
    );
    package.assert_clean();
}

/// #16/#21.10/#38: real macOS less pages a multi-screen document, responds to
/// resize and navigation, and exits through the actual installed client.
#[test]
fn installed_real_less_navigates_and_resizes() {
    let package = Package::new();
    let path = package.isolation.home().join("less.txt");
    fs::write(
        &path,
        format!("less-top\n{}less-bottom\n", "a document line\n".repeat(90)),
    )
    .unwrap();
    let mut terminal = launch(&package, "/usr/bin/less", &["-R", path.to_str().unwrap()]);
    terminal.wait_for_text("less-top", TIMEOUT).unwrap();
    terminal.resize(38, 132).unwrap();
    terminal.send("G").unwrap();
    terminal.wait_for_text("less-bottom", TIMEOUT).unwrap();
    terminal.send("q").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    package.assert_clean();
}

fn kill(pid: u32) {
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap()),
        nix::sys::signal::Signal::SIGKILL,
    )
    .unwrap();
}

fn owned_bash(package: &Package, ignore_hangup: bool) -> Terminal {
    let script = format!(
        "{}printf '%s' \"$$\" > \"$HOME/command.pid\"; printf owned-command-ready; while :; do /bin/sleep 1; done",
        if ignore_hangup { "trap '' HUP; " } else { "" }
    );
    let mut terminal = launch(
        package,
        "/bin/bash",
        &["--noprofile", "--norc", "-c", &script],
    );
    terminal
        .wait_for_text("owned-command-ready", TIMEOUT)
        .unwrap();
    terminal
}

/// #19/#21.7/#38: killing the real installed client cannot request session
/// termination; EOF drives the daemon's one leave policy, including the full
/// five-second grace and group kill of a command that ignores hangup.
#[test]
fn killed_installed_client_uses_daemon_leave_policy_and_leaves_nothing() {
    let package = Package::new();
    let mut terminal = owned_bash(&package, true);
    let started = std::time::Instant::now();
    kill(package.client_pid().try_into().unwrap());
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 137);
    drop(terminal);
    package.assert_clean();
    assert!(started.elapsed() >= std::time::Duration::from_secs(5));
    assert!(package.list().is_empty());
}

/// #19/#20/#38: command SIGKILL is an exact 128+signal status, not portable-pty's
/// lossy wait result; descendants, metadata and sockets are still owned/cleaned.
#[test]
fn killed_installed_command_returns_137_and_cleans_descendants() {
    let package = Package::new();
    let mut terminal = owned_bash(&package, false);
    let pid = fs::read_to_string(package.isolation.home().join("command.pid")).unwrap();
    kill(pid.parse().unwrap());
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 137);
    drop(terminal);
    package.assert_clean();
    assert!(package.list().is_empty());
}

/// #19/#25/#26/#38: daemon SIGKILL yields the installed client's single
/// lost-connection diagnostic with the absolute session-log suffix; the command
/// receives PTY hangup and a subsequent real list removes only stale artifacts.
#[test]
fn killed_installed_daemon_reports_logs_and_list_reclaims_stale_session() {
    let package = Package::new();
    let mut terminal = owned_bash(&package, false);
    let metadata = package.metadata();
    kill(metadata.daemon_pid);
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    let screen = terminal.screen();
    assert_eq!(screen.matches("climon2: error:").count(), 1);
    assert!(screen.contains("lost connection to session"));
    let logs = package
        .isolation
        .climon2_home()
        .canonicalize()
        .unwrap()
        .join("logs")
        .join(metadata.session_id.to_string());
    terminal
        .wait_for_bytes(format!("(logs: {})", logs.display()).as_bytes(), TIMEOUT)
        .unwrap();
    drop(terminal);
    assert!(package.list().is_empty());
    package.assert_clean();
}

/// #19/#21.7/#25/#38: only hook-enabled debug siblings inject named pre-command
/// panic and never-ready startup faults. Both cross the actual launcher/CLI
/// readiness boundary, preserve production deadlines, report logs and clean up.
/// The no-hooks final archive runs all black-box tests, never these injections.
#[test]
fn debug_hook_startup_panic_and_never_ready_report_and_clean_up() {
    if std::env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
        return;
    }
    for fault in ["panic", "never-ready"] {
        let package = Package::new();
        let report = package.isolation.home().join("must-not-spawn.json");
        let mut command = package.command("climon2");
        command.args(["exec", "climon2-fixture", "report"]);
        command.arg(&report);
        command.env("CLIMON2_DAEMON_FAULT", fault);
        let started = std::time::Instant::now();
        let mut terminal = Terminal::spawn(command, 40, 240).unwrap();
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
        if fault == "never-ready" {
            assert!(started.elapsed() >= std::time::Duration::from_secs(5));
        }
        assert_eq!(terminal.screen().matches("climon2: error:").count(), 1);
        assert!(terminal.screen().contains("(logs: "));
        drop(terminal);
        assert!(!report.exists(), "startup fault must precede the command");
        if fault == "panic" {
            assert!(package.logs().iter().any(|path| {
                path.file_name().unwrap() == "daemon.log"
                    && fs::read_to_string(path).unwrap().contains("panic")
            }));
        }
        package.assert_clean();
    }
}

/// #25/#28/#38: each installed role writes a header, CLI→client exec retains
/// PID, daemon links to that PID, and the general/session link survives exec.
/// TRACE enables real lifecycle events; WARN suppresses those same events in
/// both daemon and client without changing the always-present headers.
#[test]
fn installed_log_headers_links_and_filters_survive_exec_handoffs() {
    for filter in ["trace", "warn"] {
        let package = Package::new();
        let mut command = package.command("climon2");
        command.args([
            "exec",
            "climon2-fixture",
            "print",
            "headers-ready",
            "sleep",
            "5000",
            "exit",
            "0",
        ]);
        command.env("CLIMON2_LOG", filter);
        let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
        terminal.wait_for_text("headers-ready", TIMEOUT).unwrap();
        let client_pid = package.client_pid();
        let metadata = package.metadata();
        terminal.resize(31, 121).unwrap();
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
        drop(terminal);
        let directory = package
            .isolation
            .climon2_home()
            .join("logs")
            .join(metadata.session_id.to_string());
        let cli = fs::read_to_string(directory.join("cli.log")).unwrap();
        let daemon = fs::read_to_string(directory.join("daemon.log")).unwrap();
        let client = fs::read_to_string(directory.join("client.log")).unwrap();
        for (role, pid, log) in [
            ("cli", client_pid.to_string(), &cli),
            ("client", client_pid.to_string(), &client),
            ("daemon", metadata.daemon_pid.to_string(), &daemon),
        ] {
            let header = log.lines().next().unwrap();
            assert!(header.contains(&format!("role={role} pid={pid}")));
            assert!(header.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))));
        }
        assert!(daemon.contains(&format!("cli_pid={client_pid}")));
        assert!(cli.contains("general_log="));
        let general: Vec<_> = package
            .logs()
            .into_iter()
            .filter(|path| path.parent().unwrap().file_name().unwrap() == "general")
            .collect();
        assert_eq!(general.len(), 1);
        assert!(cli.contains(general[0].to_str().unwrap()));
        let general = fs::read_to_string(&general[0]).unwrap();
        assert!(general.contains(&format!(
            "session {} started; continuing in",
            metadata.session_id
        )));
        assert!(general.contains(directory.join("cli.log").to_str().unwrap()));
        assert_eq!(client.contains("client attached"), filter == "trace");
        assert_eq!(daemon.contains("session ready"), filter == "trace");
        assert_eq!(
            cli.contains("handing off to version-matched client"),
            filter == "trace"
        );
        package.assert_clean();
    }
}

/// #19/#20/#38: a slow pre-hello peer plus invalid framing on unrelated raw
/// sockets cannot starve the actual installed client's input/output or the
/// installed list's concurrent probe. No raw peer becomes a controlling client.
#[test]
fn installed_session_survives_raw_socket_abuse_and_pending_hello() {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixStream;

    let package = Package::new();
    let mut terminal = launch(&package, "/bin/bash", &["--noprofile", "--norc", "-i"]);
    terminal.send("echo $((40+2))-raw\n").unwrap();
    terminal.wait_for_text("42-raw", TIMEOUT).unwrap();
    let metadata = package.metadata();
    let mut pending = UnixStream::connect(metadata.socket_path.to_string()).unwrap();
    pending
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    for bytes in [
        vec![0, 0, 0, 0],
        vec![0, 16, 0, 1],
        vec![0, 0, 0, 2, 1, b'!'],
    ] {
        let mut attacker = UnixStream::connect(metadata.socket_path.to_string()).unwrap();
        attacker
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        attacker.write_all(&bytes).unwrap();
        let mut response = Vec::new();
        attacker.read_to_end(&mut response).unwrap();
    }
    assert_eq!(package.list().len(), 1);
    terminal.send("echo $((6*9))-still-live\n").unwrap();
    terminal.wait_for_text("54-still-live", TIMEOUT).unwrap();
    let mut response = Vec::new();
    pending.read_to_end(&mut response).unwrap();
    let frame = climon2_core::protocol::read_frame(&mut response.as_slice()).unwrap();
    assert!(matches!(
        climon2_core::protocol::ServerMessage::from_frame(frame).unwrap(),
        climon2_core::protocol::ServerMessage::Error { .. }
    ));
    drop(pending);
    terminal.send("exit 0\n").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    package.assert_clean();
}

/// ADRs 0001/0002/0005/#26/#38/#39: from release two, the *actual* published
/// previous archive starts a held session, the current archive installs beside
/// it, and each real CLI lists both versions while their own real clients keep
/// returning exact statuses. First release has no invented previous binary.
#[test]
fn actual_previous_release_remains_live_across_current_archive_install() {
    let Some(package) = Package::previous_archive() else {
        eprintln!("first-release gate: no previous archive supplied; no compatibility fabricated");
        return;
    };
    let start = || {
        let mut terminal = launch(&package, "/bin/bash", &["--noprofile", "--norc", "-i"]);
        terminal.send("echo $((6*7))-upgrade-ready\n").unwrap();
        terminal.wait_for_text("42-upgrade-ready", TIMEOUT).unwrap();
        terminal
    };
    let mut previous = start();
    let old = package.metadata();
    let old_client = package.client_pid();
    assert_ne!(old.climon2_version.to_string(), env!("CARGO_PKG_VERSION"));
    package.install_current_release();
    assert_eq!(package.client_pid(), old_client);
    let sessions = package.list();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0]["session_id"], old.session_id.to_string());
    let mut current = start();
    let sessions = package.list();
    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .any(|session| session["climon2_version"] == old.climon2_version.to_string())
    );
    assert!(
        sessions
            .iter()
            .any(|session| session["climon2_version"] == env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(
        package
            .list_with_version(&old.climon2_version.to_string())
            .len(),
        2
    );
    previous.send("exit 7\n").unwrap();
    assert_eq!(previous.wait_exit(TIMEOUT).unwrap(), 7);
    drop(previous);
    assert_eq!(package.list().len(), 1);
    current.send("exit 9\n").unwrap();
    assert_eq!(current.wait_exit(TIMEOUT).unwrap(), 9);
    drop(current);
    assert!(package.list().is_empty());
    package.assert_clean();
    package.assert_archive_default_levels();
}

/// ADR 0001/#19/#25/#38: removing the installed version's client is detected
/// after genuine daemon readiness; the CLI's failed handoff names the version
/// and logs, and its armed startup owner does not leave a command or session.
#[test]
fn missing_installed_client_after_ready_is_diagnostic_and_cleans_command() {
    let package = Package::private();
    let client = package
        .executable_home()
        .join(env!("CARGO_PKG_VERSION"))
        .join("climon2-client");
    let saved = client.with_file_name("client-held-away");
    fs::rename(&client, &saved).unwrap();
    let script = "trap '' HUP; printf command-started > \"$HOME/ready-marker\"; while :; do /bin/sleep 1; done";
    let mut terminal = launch(
        &package,
        "/bin/bash",
        &["--noprofile", "--norc", "-c", script],
    );
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    assert!(package.isolation.home().join("ready-marker").is_file());
    let screen = terminal.screen();
    assert_eq!(screen.matches("climon2: error:").count(), 1);
    terminal
        .wait_for_bytes(env!("CARGO_PKG_VERSION").as_bytes(), TIMEOUT)
        .unwrap();
    terminal.wait_for_bytes(b"(logs: ", TIMEOUT).unwrap();
    drop(terminal);
    fs::rename(saved, client).unwrap();
    package.assert_clean();
    assert!(package.list().is_empty());
}

/// #21.17-20/#38: a hooked real installer crashes after publishing the launcher
/// while a real client owns a session; current still routes correctly, the
/// session remains usable, and the next real installer removes its staging.
/// This internal-only fault never substitutes binaries into final archives.
#[test]
fn debug_hook_mid_install_crash_recovers_without_stranding_live_client() {
    if std::env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
        return;
    }
    let package = Package::private();
    let installer = package.executable_home().join("install");
    fs::copy(climon2_test_term::bin_path("climon2-install"), &installer).unwrap();
    let mut terminal = launch(&package, "/bin/bash", &["--noprofile", "--norc", "-i"]);
    terminal.send("echo $((6*7))-install-ready\n").unwrap();
    terminal.wait_for_text("42-install-ready", TIMEOUT).unwrap();
    let session = package.metadata().session_id.to_string();
    let current = fs::read(package.executable_home().join("current")).unwrap();
    let invoke = |fault: Option<&str>| {
        let mut command = std::process::Command::new(&installer);
        command
            .env_clear()
            .env("HOME", package.isolation.home())
            .env("PATH", "/usr/bin:/bin")
            .args(["--dir"])
            .arg(package.executable_home());
        if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
            command.env("LLVM_PROFILE_FILE", profile);
        }
        if let Some(fault) = fault {
            command.env("CLIMON2_TEST_INSTALL_FAULT", fault);
        }
        command.output().unwrap()
    };
    let failed = invoke(Some("crash:launcher-published"));
    assert_eq!(failed.status.code(), Some(86), "{failed:?}");
    assert_eq!(
        fs::read(package.executable_home().join("current")).unwrap(),
        current
    );
    assert!(
        package
            .executable_home()
            .join(".climon2-install-stage")
            .exists()
    );
    assert_eq!(package.list()[0]["session_id"], session);
    let recovered = invoke(None);
    assert!(recovered.status.success(), "{recovered:?}");
    assert!(
        !package
            .executable_home()
            .join(".climon2-install-stage")
            .exists()
    );
    terminal.send("echo $((7*8))-after-recovery\n").unwrap();
    terminal
        .wait_for_text("56-after-recovery", TIMEOUT)
        .unwrap();
    terminal.send("exit 7\n").unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 7);
    drop(terminal);
    assert!(package.list().is_empty());
    package.assert_clean();
}

fn executable_script(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, script).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn frozen_client_fixture(package: &Package, real: &Path, version: &str) -> String {
    format!(
        "#!/bin/bash\nset -eu\n[ \"$#\" -eq 3 ]\n[ \"$1\" = attach ]\n[ \"$2\" = --metadata ]\ncase \"$3\" in /*) ;; *) exit 77 ;; esac\n[ \"$CLIMON2_HOME\" = {} ]\n[ \"$CLIMON2_LOG\" = warn ]\nprintf '%s\\n' \"$1\" \"$2\" \"$3\" > \"$HOME/frozen-client-{version}\"\nexec {} \"$@\"\n",
        quote(&package.isolation.climon2_home().canonicalize().unwrap()),
        quote(real),
    )
}

/// ADRs 0001/0002/#28/#38: controlled compatibility fixtures independently reject
/// any client argv beyond `attach --metadata ABSOLUTE`, require home/filter in
/// environment, and identify which version was selected. A current handoff
/// retains a running client's old version; new sessions use their own version.
/// These explicitly synthetic fixtures are not a previous-release gate and are
/// never substituted into the final no-hooks archive.
#[test]
fn controlled_versions_route_their_own_frozen_client_across_current_handoff() {
    if std::env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
        return;
    }
    let package = Package::private();
    let old_version = env!("CARGO_PKG_VERSION");
    let old_folder = package.executable_home().join(old_version);
    let real_client = old_folder.join("client-real");
    fs::rename(old_folder.join("climon2-client"), &real_client).unwrap();
    executable_script(
        &old_folder.join("climon2-client"),
        &frozen_client_fixture(&package, &real_client, old_version),
    );
    let start = || {
        let mut command = package.command("climon2");
        command.args(["exec", "/bin/bash", "--noprofile", "--norc", "-i"]);
        command.env("CLIMON2_LOG", "warn");
        let mut terminal = Terminal::spawn(command, 30, 120).unwrap();
        terminal.send("echo $((6*7))-version-ready\n").unwrap();
        terminal.wait_for_text("42-version-ready", TIMEOUT).unwrap();
        terminal
    };
    let mut old = start();
    let old_id = package.metadata().session_id.to_string();
    assert_eq!(
        fs::read_to_string(
            package
                .isolation
                .home()
                .join(format!("frozen-client-{old_version}"))
        )
        .unwrap(),
        format!(
            "attach\n--metadata\n{}\n",
            package
                .isolation
                .climon2_home()
                .canonicalize()
                .unwrap()
                .join("sessions")
                .join(format!("{old_id}.pty"))
                .display()
        )
    );
    let fixture_version = "9.9.9";
    let payload_root = package.isolation.home().join("controlled-payload");
    let fixture_folder = payload_root.join(fixture_version);
    fs::create_dir_all(&fixture_folder).unwrap();
    fs::copy(
        package.executable_home().join("climon2"),
        payload_root.join("climon2"),
    )
    .unwrap();
    for binary in ["climon2-cli", "climon2-daemon"] {
        fs::copy(old_folder.join(binary), fixture_folder.join(binary)).unwrap();
    }
    executable_script(
        &fixture_folder.join("climon2-client"),
        &frozen_client_fixture(&package, &real_client, fixture_version),
    );
    let payload = climon2_install::Payload::from_directory(&payload_root, fixture_version).unwrap();
    climon2_install::install(&payload, &package.executable_home()).unwrap();
    assert_eq!(package.list()[0]["session_id"], old_id);
    let mut new = start();
    let sessions = package.list();
    assert_eq!(sessions.len(), 2);
    assert!(
        sessions
            .iter()
            .any(|session| session["climon2_version"] == fixture_version)
    );
    assert_eq!(package.list_with_version(old_version).len(), 2);
    let proof = fs::read_to_string(package.isolation.home().join("frozen-client-9.9.9")).unwrap();
    let invocation: Vec<_> = proof.lines().collect();
    assert_eq!(invocation.len(), 3);
    assert_eq!(&invocation[..2], ["attach", "--metadata"]);
    assert!(Path::new(invocation[2]).is_absolute());
    let routed = climon2_core::metadata::SessionMetadata::from_json(
        Path::new(invocation[2]),
        &fs::read(invocation[2]).unwrap(),
    )
    .unwrap();
    assert_eq!(routed.climon2_version.to_string(), fixture_version);
    old.send("exit 7\n").unwrap();
    assert_eq!(old.wait_exit(TIMEOUT).unwrap(), 7);
    drop(old);
    assert_eq!(package.list().len(), 1);
    new.send("exit 9\n").unwrap();
    assert_eq!(new.wait_exit(TIMEOUT).unwrap(), 9);
    drop(new);
    assert!(package.list().is_empty());
    package.assert_clean();
}

/// #19/#25/#28/#38: a hooked client panic after the real exec handoff is a
/// single safe diagnostic with the session-log suffix; static panic capture
/// survives and the actual daemon applies leave policy without an orphan.
#[test]
fn debug_hook_client_panic_after_exec_logs_and_uses_daemon_leave_policy() {
    if std::env::var_os("CLIMON2_E2E_ARCHIVE").is_some() {
        return;
    }
    let package = Package::new();
    let mut command = package.command("climon2");
    command.args(["exec", "climon2-fixture", "await-resize", "exit", "0"]);
    command.env("CLIMON2_CLIENT_FAULT", "panic");
    let mut terminal = Terminal::spawn(command, 30, 240).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 125);
    assert_eq!(terminal.screen().matches("climon2: error:").count(), 1);
    terminal
        .wait_for_bytes(b"client panicked", TIMEOUT)
        .unwrap();
    terminal.wait_for_bytes(b"(logs: ", TIMEOUT).unwrap();
    drop(terminal);
    package.assert_clean();
    assert!(package.logs().iter().any(|path| {
        path.file_name().unwrap() == "client.log"
            && fs::read_to_string(path).unwrap().contains("client panic")
    }));
}
