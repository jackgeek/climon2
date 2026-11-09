use climon2_test_term::{Isolation, Mouse, MouseButton, Terminal, bin_path};
use std::{fs, io::ErrorKind, time::Duration};

/// #31: the reader keeps the screen and cursor replies live even between API waits.
#[test]
fn terminal_answers_cursor_queries_without_an_active_wait() {
    let isolation = Isolation::new();
    let mut command = isolation.command(bin_path("climon2-tui-fixture"));
    command.arg(isolation.home().join("idle-query.json"));
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !terminal.screen().contains("cursor-ok") {
        assert!(
            std::time::Instant::now() < deadline,
            "background terminal reader did not answer the cursor query"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    terminal.send(b"\x04").unwrap();
    assert_eq!(terminal.wait_exit(Duration::from_secs(1)).unwrap(), 0);
    drop(terminal);
    isolation.assert_clean();
}

/// #21/#31: alternate screen, automatic cursor response, mouse, paste and resize redraw.
#[test]
fn tui_round_trips_terminal_protocols_and_restores_the_screen() {
    let isolation = Isolation::new();
    let report = isolation.home().join("tui.json");
    let mut command = isolation.command(bin_path("climon2-tui-fixture"));
    command.arg(&report);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    let second = Duration::from_secs(1);
    terminal.wait_for_bytes(b"\x1b[?1049h", second).unwrap();
    terminal.wait_for_text("cursor-ok", second).unwrap();
    terminal.send(b"\x1b[<0;7;9M").unwrap();
    terminal.wait_for_text("mouse-ok", second).unwrap();
    terminal.send(b"\x1b[200~pasted-text\x1b[201~").unwrap();
    terminal.wait_for_text("paste-ok", second).unwrap();
    terminal.resize(32, 99).unwrap();
    terminal
        .wait_for_text("size=32x99", Duration::from_secs(2))
        .unwrap();
    terminal.send(b"\x04").unwrap();
    assert_eq!(terminal.wait_exit(second).unwrap(), 0);
    terminal.wait_for_text("tui-done", second).unwrap();
    assert!(!terminal.screen().contains("cursor-ok"));
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    assert_eq!(value["cursor"], "\u{1b}[3;5R");
    assert_eq!(value["mouse"], "\u{1b}[<0;7;9M");
    assert_eq!(value["paste"], "pasted-text");
    assert_eq!(value["size"], serde_json::json!([32, 99]));
    drop(terminal);
    isolation.assert_clean();
}

/// #79 (Terminal oracle): injected host focus, SGR mouse press, drag and
/// release, wheel and bracketed paste reach the TUI fixture byte for byte, in
/// order, with xterm's documented encodings; positions outside the screen are
/// refused without sending anything.
#[test]
fn tui_receives_injected_focus_mouse_wheel_and_paste_encodings() {
    let isolation = Isolation::new();
    let report = isolation.home().join("injected.json");
    let mut command = isolation.command(bin_path("climon2-tui-fixture"));
    command.arg(&report);
    let mut terminal = Terminal::spawn(command, 24, 80).unwrap();
    let second = Duration::from_secs(1);
    terminal.wait_for_text("cursor-ok", second).unwrap();
    terminal.focus(true).unwrap();
    terminal.focus(false).unwrap();
    for (action, row, column) in [
        (Mouse::Press(MouseButton::Left), 8, 6),
        (Mouse::Drag(MouseButton::Left), 9, 7),
        (Mouse::Release(MouseButton::Left), 9, 7),
        (Mouse::Press(MouseButton::Middle), 0, 0),
        (Mouse::Release(MouseButton::Middle), 0, 0),
        (Mouse::Press(MouseButton::Right), 23, 79),
        (Mouse::Drag(MouseButton::Right), 22, 78),
        (Mouse::Release(MouseButton::Right), 22, 78),
        (Mouse::WheelUp, 3, 4),
        (Mouse::WheelDown, 3, 4),
    ] {
        terminal.mouse(action, row, column).unwrap();
    }
    for (row, column) in [(24, 0), (0, 80)] {
        let refused = terminal.mouse(Mouse::Press(MouseButton::Left), row, column);
        assert_eq!(refused.unwrap_err().kind(), ErrorKind::InvalidInput);
    }
    terminal.paste("multi\nline \x1b[A paste").unwrap();
    terminal.send(b"\x04").unwrap();
    assert_eq!(terminal.wait_exit(second).unwrap(), 0);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
    let input: Vec<u8> = serde_json::from_value(value["input"].clone()).unwrap();
    let expected = [
        b"\x1b[3;5R".as_slice(),
        b"\x1b[I",
        b"\x1b[O",
        b"\x1b[<0;7;9M",
        b"\x1b[<32;8;10M",
        b"\x1b[<0;8;10m",
        b"\x1b[<1;1;1M",
        b"\x1b[<1;1;1m",
        b"\x1b[<2;80;24M",
        b"\x1b[<34;79;23M",
        b"\x1b[<2;79;23m",
        b"\x1b[<64;5;4M",
        b"\x1b[<65;5;4M",
        b"\x1b[200~multi\nline \x1b[A paste\x1b[201~",
        b"\x04",
    ]
    .concat();
    assert_eq!(
        input.escape_ascii().to_string(),
        expected.escape_ascii().to_string()
    );
    drop(terminal);
    isolation.assert_clean();
}
