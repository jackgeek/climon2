use climon2_test_term::{
    Cell, Color, Cursor, CursorShape, Isolation, Rendition, Terminal, Underline, Width, bin_path,
};
use std::{fs, time::Duration};

const SECOND: Duration = Duration::from_secs(1);

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
    assert_eq!(terminal.wait_exit(SECOND).unwrap(), 0);
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
        assert_eq!(terminal.wait_exit(SECOND).unwrap(), 0);
        assert_eq!(terminal.cursor_state(), expected, "{output:?}");
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
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("hello-pty", SECOND).unwrap();
    terminal.wait_for_bytes(b"hello-pty", SECOND).unwrap();
    assert_eq!(terminal.wait_exit(SECOND).unwrap(), 7);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(
        value["cwd"],
        isolation.home().canonicalize().unwrap().to_str().unwrap()
    );
    assert_eq!(value["env"]["CLIMON2_TEST_ID"], isolation.id());
    assert_eq!(value["env"]["FIXTURE_VALUE"], "spaces and unicode: \u{3bb}");
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
    terminal.wait_for_text("resize-ready", SECOND).unwrap();
    terminal.resize(31, 97).unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(2)).unwrap(), 143);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(value["size"], serde_json::json!([31, 97]));
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: writer stays alive, waits are bounded and failure output aids diagnosis.
#[test]
fn timed_waits_dump_screen_and_bytes_and_drop_cleans_ignored_sighup() {
    let isolation = Isolation::new();
    let mut command = isolation.command(bin_path("climon2-fixture"));
    command.args([
        "ignore-sighup",
        "kill-self",
        "HUP",
        "print",
        "still-alive",
        "sleep",
        "5000",
    ]);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    terminal.wait_for_text("still-alive", SECOND).unwrap();
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
    let mut digest = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
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
