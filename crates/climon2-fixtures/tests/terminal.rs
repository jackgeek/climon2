use climon2_test_term::{
    Cell, Color, Cursor, CursorShape, HostSignal, Isolation, Mouse, MouseButton, Rendition,
    Terminal, Underline, Width, bin_path,
};
use std::{fs, time::Duration};

mod support;
use support::TIMEOUT;

/// A default-coloured cell holding `text`.
fn cell(text: &str) -> Cell {
    Cell {
        text: text.to_owned(),
        ..Cell::default()
    }
}

/// #79 (Terminal oracle): each cell reports the rendition, colours, width and
/// combining characters that ECMA-48 SGR and Unicode width assign to what the
/// generic fixture wrote, not just its base character.
#[test]
fn generic_terminal_cells_report_rendition_colours_and_character_width() {
    let isolation = Isolation::new();
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "print",
        "\x1b[1;31mB\x1b[0;2;3;4mD\x1b[0;4:3;7;8;9mI\x1b[0;38;5;196;48;2;1;2;3mC\
         \x1b[0;94;103;4:2;58:2::10:20:30mU\x1b[0;4:4;58;5;9mO\x1b[0;4:5mS\
         \x1b[0;44m\u{754c}\x1b[0me\u{301}",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    let expected = [
        Cell {
            foreground: Color::Ansi(1),
            rendition: Rendition {
                bold: true,
                ..Rendition::default()
            },
            ..cell("B")
        },
        Cell {
            rendition: Rendition {
                dim: true,
                italic: true,
                underline: Some(Underline::Single),
                ..Rendition::default()
            },
            ..cell("D")
        },
        Cell {
            rendition: Rendition {
                underline: Some(Underline::Curly),
                inverse: true,
                hidden: true,
                strikeout: true,
                ..Rendition::default()
            },
            ..cell("I")
        },
        Cell {
            foreground: Color::Palette(196),
            background: Color::Rgb(1, 2, 3),
            ..cell("C")
        },
        Cell {
            foreground: Color::Ansi(12),
            background: Color::Ansi(11),
            underline_color: Color::Rgb(10, 20, 30),
            rendition: Rendition {
                underline: Some(Underline::Double),
                ..Rendition::default()
            },
            ..cell("U")
        },
        Cell {
            underline_color: Color::Palette(9),
            rendition: Rendition {
                underline: Some(Underline::Dotted),
                ..Rendition::default()
            },
            ..cell("O")
        },
        Cell {
            rendition: Rendition {
                underline: Some(Underline::Dashed),
                ..Rendition::default()
            },
            ..cell("S")
        },
        Cell {
            width: Width::Wide,
            background: Color::Ansi(4),
            ..cell("\u{754c}")
        },
        Cell {
            width: Width::Spacer,
            background: Color::Ansi(4),
            ..cell("")
        },
        cell("e\u{301}"),
        Cell::default(),
    ];
    assert_eq!(terminal.cells(0)[..expected.len()], expected);
    assert_eq!(terminal.cells(0).len(), 80);
    drop(terminal);
    isolation.assert_clean();
}

/// #79 (Terminal oracle): the cursor reports the position (CUP), visibility
/// (DECTCEM), shape and blink (DECSCUSR, private mode 12) that the generic
/// fixture last selected.
#[test]
fn generic_terminal_cursor_reports_position_visibility_shape_and_blink() {
    let isolation = Isolation::new();
    let home = Cursor {
        row: 0,
        column: 0,
        visible: true,
        shape: CursorShape::Block,
        blinking: false,
    };
    for (output, expected) in [
        ("x", Cursor { column: 1, ..home }),
        (
            "\x1b[5;10H\x1b[?25l\x1b[3 q",
            Cursor {
                row: 4,
                column: 9,
                visible: false,
                shape: CursorShape::Underline,
                blinking: true,
            },
        ),
        (
            "\x1b[2;3H\x1b[6 q",
            Cursor {
                row: 1,
                column: 2,
                shape: CursorShape::Bar,
                ..home
            },
        ),
        (
            "\x1b[5 q\x1b[?12l",
            Cursor {
                shape: CursorShape::Bar,
                ..home
            },
        ),
        (
            "\x1b[4 q\x1b[?12h",
            Cursor {
                shape: CursorShape::Underline,
                blinking: true,
                ..home
            },
        ),
        (
            "\x1b[1 q\x1b[?25l\x1b[?25h",
            Cursor {
                blinking: true,
                ..home
            },
        ),
        ("\x1b[6 q\x1b[0 q", home),
    ] {
        let mut command = isolation.command(bin_path("climon2-fixture"));
        command.args(["print", output]);
        let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
        assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
        assert_eq!(terminal.cursor_state(), expected, "{output:?}");
    }
    isolation.assert_clean();
}

/// Documented exact bytes of the monitor's host reports (`docs/protocol/monitor-host`).
const WORKING: &[u8] = b"\x1b]7501;state=working:app=climon2-monitor\x1b\\";
const CLEAR: &[u8] = b"\x1b]7501;state=clear\x1b\\";

fn report(bytes: &[u8]) -> HostSignal {
    HostSignal::Osc7501(bytes.to_vec())
}

/// #79 (Terminal oracle): OSC 7501 reports and bells reach the host signal log
/// in stream order with their exact bytes. A BEL that ends another OSC string,
/// or sits inside an APC string, rings no bell; a report keeps the terminator
/// the host parsed, or none when an escape sequence interrupted it.
#[test]
fn generic_terminal_records_program_status_reports_and_bells_in_order() {
    let isolation = Isolation::new();
    let blocked_by_bel = b"\x1b]7501;state=blocked:kind=question:app=climon2-monitor\x07";
    let interrupted = b"\x1b]7501;state=idle";
    let output = [
        b"a\x07".as_slice(),
        WORKING,
        b"\x1b]0;title\x07",
        b"\x1b_apc\x07\x1b\\",
        blocked_by_bel,
        b"\x1b]75010;x\x1b\\\x1b]750;x\x07",
        interrupted,
        b"\x1b[0m",
        CLEAR,
        b"\x07",
    ]
    .concat();
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args(["print", std::str::from_utf8(&output).unwrap()]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    assert_eq!(
        terminal.host_signals(),
        [
            HostSignal::Bell,
            report(WORKING),
            report(blocked_by_bel),
            report(interrupted),
            report(CLEAR),
            HostSignal::Bell,
        ]
    );
    drop(terminal);
    isolation.assert_clean();
}

/// #79 (Terminal oracle): a report split across host reads, even inside its
/// `ESC \` terminator, is recorded once and whole when the read that completes
/// it arrives, before the bell that follows it.
#[test]
fn generic_terminal_records_a_report_split_across_reads_once() {
    let isolation = Isolation::new();
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "raw",
        "print",
        "\x1b]7501;state=wor",
        "await-input",
        "1",
        "print",
        "king:app=climon2-monitor\x1b",
        "await-input",
        "2",
        "print",
        "\\\x07",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal
        .wait_for_bytes(b"\x1b]7501;state=wor", TIMEOUT)
        .unwrap();
    assert!(terminal.host_signals().is_empty());
    terminal.send("1").unwrap();
    terminal
        .wait_for_bytes(b"king:app=climon2-monitor\x1b", TIMEOUT)
        .unwrap();
    assert!(terminal.host_signals().is_empty());
    terminal.send("2").unwrap();
    terminal
        .wait_for_host_signals(TIMEOUT, |signals| signals.len() == 2)
        .unwrap();
    assert_eq!(terminal.host_signals(), [report(WORKING), HostSignal::Bell]);
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 0);
    drop(terminal);
    isolation.assert_clean();
}

/// Injects one host input event into a terminal.
type Inject = fn(&mut Terminal) -> std::io::Result<()>;

/// #79 (Terminal oracle): like a real terminal, the harness sends focus, mouse
/// and paste input only in the modes the generic fixture enabled; otherwise it
/// refuses and nothing reaches the fixture, whose line discipline would echo it.
#[test]
fn generic_terminal_refuses_input_the_application_has_not_enabled() {
    let isolation = Isolation::new();
    let cases: [(&str, Inject); 5] = [
        ("", |terminal| terminal.focus(true)),
        ("", |terminal| terminal.paste("text")),
        ("\x1b[?1000h", |terminal| {
            terminal.mouse(Mouse::Press(MouseButton::Left), 0, 0)
        }),
        ("\x1b[?1000;1006h", |terminal| {
            terminal.mouse(Mouse::Drag(MouseButton::Left), 0, 0)
        }),
        ("\x1b[?1006h", |terminal| {
            terminal.mouse(Mouse::WheelUp, 0, 0)
        }),
    ];
    for (modes, inject) in cases {
        let mut command = isolation.command(bin_path("climon2-fixture"));
        command.args(["print", &format!("{modes}ready"), "sleep", "60000"]);
        let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
        terminal.wait_for_text("ready", TIMEOUT).unwrap();
        let refused = inject(&mut terminal).unwrap_err();
        assert_eq!(
            refused.kind(),
            std::io::ErrorKind::InvalidInput,
            "{modes:?}"
        );
        terminal.send("done\n").unwrap();
        terminal.wait_for_bytes(b"done\r\n", TIMEOUT).unwrap();
        let received = terminal.received();
        assert!(
            received.ends_with(b"readydone\r\n"),
            "{modes:?}: {}",
            received.escape_ascii()
        );
    }
    isolation.assert_clean();
}

/// #21/#31: real PTY output, exact argv/cwd/env, terminal identity and exit status.
#[test]
fn generic_terminal_reports_its_contract_and_exits() {
    let isolation = Isolation::new();
    let report = isolation.home().join("report.json");
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "report",
        report.to_str().unwrap(),
        "print",
        "hello-pty",
        "exit",
        "7",
    ]);
    command.env("FIXTURE_VALUE", "spaces and unicode: \u{3bb}");
    command.env("PWD", "literal-not-the-working-directory");
    command.env("SHLVL", "41");
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("hello-pty", TIMEOUT).unwrap();
    terminal.wait_for_bytes(b"hello-pty", TIMEOUT).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 7);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(
        value["cwd"],
        isolation.home().canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(value["env"]["CLIMON2_TEST_ID"], isolation.id());
    assert_eq!(value["env"]["FIXTURE_VALUE"], "spaces and unicode: \u{3bb}");
    assert_eq!(value["env"]["PWD"], "literal-not-the-working-directory");
    assert_eq!(value["env"]["SHLVL"], "41");
    assert_eq!(value["size"], serde_json::json!([24, 80]));
    assert_eq!(value["isatty"], serde_json::json!([true, true, true]));
    assert_eq!(value["pid"], value["pgid"]);
    assert_eq!(value["argv"][3], "hello-pty");
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: kernel resize reaches the fixture; signal exits preserve 128+signal.
#[test]
fn generic_terminal_observes_resize_and_signal_exit() {
    let isolation = Isolation::new();
    let report = isolation.home().join("resized.json");
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "await-resize",
        "report",
        report.to_str().unwrap(),
        "kill-self",
        "TERM",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("resize-ready", TIMEOUT).unwrap();
    terminal.resize(31, 97).unwrap();
    assert_eq!(terminal.wait_exit(TIMEOUT).unwrap(), 143);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(value["size"], serde_json::json!([31, 97]));
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31/#79: writer stays alive, waits are bounded and failure output (the
/// screen, the host signals and the recent bytes) aids diagnosis.
#[test]
fn timed_waits_dump_screen_signals_and_bytes_and_drop_cleans_ignored_sighup() {
    let isolation = Isolation::new();
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "ignore-sighup",
        "kill-self",
        "HUP",
        "print",
        "\x07still-alive",
        "sleep",
        "60000",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("still-alive", TIMEOUT).unwrap();
    for result in [
        terminal.wait_for_text("missing", Duration::from_millis(100)),
        terminal.wait_for_bytes(b"absent", Duration::from_millis(100)),
        terminal.wait_exit(Duration::from_millis(100)).map(|_| ()),
    ] {
        let error = result.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        let message = error.to_string();
        assert!(
            message.contains("SCREEN:")
                && message.contains("HOST SIGNALS:\n[Bell]")
                && message.contains("RECENT BYTES:")
                && message.contains("still-alive"),
            "{message}"
        );
    }
    assert!(std::panic::catch_unwind(|| isolation.assert_clean()).is_err());
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: metadata and sockets are leaks; diagnostic logs may remain.
#[test]
fn leak_check_rejects_metadata_and_sockets_but_keeps_logs() {
    use std::{
        io::Write,
        os::unix::net::UnixListener,
        process::{Command, Stdio},
    };
    let isolation = Isolation::new();
    let sessions = isolation.climon2_home().join("sessions");
    fs::create_dir(&sessions).unwrap();
    fs::write(sessions.join("session.log"), "diagnostic").unwrap();
    isolation.assert_clean();
    let metadata = sessions.join("session.pty");
    fs::write(&metadata, "{}").unwrap();
    assert!(std::panic::catch_unwind(|| isolation.assert_clean()).is_err());
    fs::remove_file(metadata).unwrap();
    // Independent system SHA-256 implementation supplies the #18 runtime-path oracle.
    #[cfg(target_os = "macos")]
    let mut digest = {
        let mut command = Command::new("/usr/bin/shasum");
        command.args(["-a", "256"]);
        command
    };
    #[cfg(target_os = "linux")]
    let mut digest = Command::new("/usr/bin/sha256sum");
    let mut digest = digest
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    digest
        .stdin
        .take()
        .unwrap()
        .write_all(
            isolation
                .climon2_home()
                .canonicalize()
                .unwrap()
                .as_os_str()
                .as_encoded_bytes(),
        )
        .unwrap();
    let digest = digest.wait_with_output().unwrap();
    assert!(digest.status.success());
    let digest = String::from_utf8(digest.stdout).unwrap();
    let runtime = std::path::PathBuf::from(format!(
        "/tmp/climon2-{}-{}",
        nix::unistd::getuid(),
        &digest[..8]
    ));
    fs::create_dir(&runtime).unwrap();
    let socket = runtime.join("test.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    assert!(std::panic::catch_unwind(|| isolation.assert_clean()).is_err());
    drop(listener);
    fs::remove_file(socket).unwrap();
    fs::remove_dir(runtime).unwrap();
    isolation.assert_clean();
}
