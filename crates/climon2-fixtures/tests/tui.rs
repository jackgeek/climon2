use climon2_test_term::{Isolation, Terminal, bin_path};
use std::{fs, time::Duration};

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
