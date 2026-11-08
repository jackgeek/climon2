use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use monitor_selector_prototype::{
    Outcome, Session, State, Status,
    input::{self, Prefix},
    ui,
};
use ratatui::{
    Terminal,
    backend::{Backend, TestBackend},
    style::Color,
};

fn session(id: u128, title: &str, status: Status, changed: u64) -> Session {
    Session {
        id,
        title: title.into(),
        command: "cargo test --workspace".into(),
        cwd: "/work/project".into(),
        status,
        changed,
        connected: true,
        controlled: id == 1,
        cache: vec!["retained output".into()],
        outcome: None,
    }
}

fn state() -> State {
    State::new(vec![
        session(1, "Shell", Status::Idle, 10),
        session(2, "Build", Status::Running, 20),
        session(3, "Review", Status::NeedsAttention, 30),
    ])
}

fn frame(state: &State, width: u16, height: u16) -> (Terminal<TestBackend>, ui::Hits) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut hits = ui::Hits::default();
    terminal
        .draw(|frame| hits = ui::paint(frame, state))
        .unwrap();
    (terminal, hits)
}

fn text(terminal: &Terminal<TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect()
}

fn key(state: &mut State, code: KeyCode, modifiers: KeyModifiers, prefix: Prefix) {
    let (_, hits) = frame(state, 100, 32);
    input::handle(
        state,
        Event::Key(KeyEvent::new(code, modifiers)),
        &hits,
        prefix,
        true,
    );
}

#[test]
fn opening_navigation_and_cancel_never_select_or_take_control() {
    let mut state = state();
    state.open();
    assert_eq!(state.highlight, Some(1));
    state.move_highlight(-1);
    assert_eq!(state.highlight, Some(2));
    state.application_input("menu key");
    assert!(state.input.is_empty());
    state.cancel();
    assert_eq!(state.selected, Some(1));
    assert!(!state.sessions[1].controlled);
    state.application_input("live key");
    assert_eq!(state.input, ["live key"]);
}

#[test]
fn literal_filtering_and_paste_never_execute_monitor_actions() {
    let mut state = state();
    state.open();
    state.insert_search("bUiLd");
    assert_eq!(state.visible_ids(), [2]);
    assert_eq!(state.highlight, Some(2));
    state.query.clear();
    state.cursor = 0;
    state.insert_search("cargo");
    assert_eq!(state.visible_ids(), [3, 2, 1]);
    state.query.clear();
    state.cursor = 0;
    state.insert_search("A\tB\n\u{1d}x\u{1b}");
    assert_eq!(state.query.iter().collect::<String>(), "A B x");
    assert!(!state.exiting);
    assert_eq!(state.selected, Some(1));
    assert!(state.input.is_empty());
}

#[test]
fn status_search_matches_displayed_status_and_failure_details_case_insensitively() {
    for (query, expected) in [("SUCCEEDED", 1), ("signal 9", 2), ("NEEDS ATTENTION", 3)] {
        let mut state = state();
        state.sessions[0].status = Status::Completed;
        state.sessions[0].outcome = Some(Outcome::Exit(0));
        state.sessions[1].status = Status::Failed;
        state.sessions[1].outcome = Some(Outcome::Signal(9));
        state.open();
        state.insert_search(query);
        assert_eq!(state.visible_ids(), [expected], "{query}");
        assert_eq!(state.highlight, Some(expected));
    }
}

#[test]
fn status_search_uses_effective_disconnection_and_invalidates_changed_matches() {
    let mut state = state();
    state.sessions[1].connected = false;
    state.open();
    state.insert_search("running");
    assert!(state.visible_ids().is_empty());
    state.query.clear();
    state.cursor = 0;
    state.insert_search("disconnected");
    assert_eq!(state.visible_ids(), [2]);
    assert_eq!(state.highlight, Some(2));
    state.sessions[1].connected = true;
    state.reconcile();
    assert_eq!(state.highlight, None);
    state.confirm();
    assert_eq!(state.selected, Some(1));
    assert!(state.menu);
}

#[test]
fn asynchronous_changes_preserve_identity_and_cannot_redirect_enter() {
    let mut state = state();
    state.open();
    state.highlight = Some(2);
    state.sessions[1].status = Status::NeedsAttention;
    state.sessions[1].changed = 40;
    state.reconcile();
    assert_eq!(state.visible_ids(), [2, 3, 1]);
    assert_eq!(state.highlight, Some(2));
    state.sessions.retain(|session| session.id != 2);
    state.reconcile();
    assert_eq!(state.highlight, None);
    state.confirm();
    assert_eq!(state.selected, Some(1));
    assert!(state.menu);
    state.move_highlight(1);
    state.confirm();
    assert_eq!(state.selected, Some(3));
    assert!(!state.menu);
}

#[test]
fn title_losing_search_match_requires_new_explicit_highlight() {
    let mut state = state();
    state.open();
    state.insert_search("build");
    state.sessions[1].title = "Other".into();
    state.reconcile();
    state.confirm();
    assert_eq!(state.highlight, None);
    assert_eq!(state.selected, Some(1));
    assert!(state.menu);
}

#[test]
fn always_focused_search_ignores_tab_and_arrows_navigate_without_wrapping() {
    let mut state = state();
    state.open();
    key(
        &mut state,
        KeyCode::Tab,
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Char('b'),
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    assert_eq!(state.query, ['b']);
    key(
        &mut state,
        KeyCode::Down,
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    assert_eq!(state.highlight, Some(2));
    state.query.clear();
    state.cursor = 0;
    state.search_changed();
    state.move_highlight(100);
    assert_eq!(state.highlight, Some(1));
    state.move_highlight(-100);
    assert_eq!(state.highlight, Some(3));
    state.insert_search("界e");
    state.cursor = 1;
    state.insert_search("\u{301}");
    assert_eq!(state.query.iter().collect::<String>(), "界\u{301}e");
}

#[test]
fn enter_opens_disconnected_or_final_history_and_chrome_is_truthful() {
    for (status, connected, expected) in [
        (Status::Running, false, "Disconnected"),
        (Status::Failed, false, "Failed - signal 9"),
        (Status::Completed, false, "Succeeded"),
    ] {
        let mut state = state();
        state.sessions[1].status = status;
        state.sessions[1].connected = connected;
        state.sessions[1].outcome = Some(Outcome::Signal(9));
        state.open();
        state.highlight = Some(2);
        state.confirm();
        assert_eq!(state.selected, Some(2));
        assert!(state.inspecting);
        assert!(!state.sessions[1].controlled);
        state.application_input("blocked");
        assert!(state.input.is_empty());
        let (terminal, _) = frame(&state, 100, 32);
        let text = text(&terminal);
        assert!(text.contains(expected));
        assert!(text.contains("retained output"));
        assert!(text.contains("READ-ONLY"));
    }
}

#[test]
fn missing_disconnected_cache_is_explicit_and_connected_enter_takes_control() {
    let mut state = state();
    state.sessions[1].connected = false;
    state.sessions[1].cache.clear();
    state.open();
    state.highlight = Some(2);
    state.confirm();
    let (terminal, _) = frame(&state, 100, 32);
    assert!(text(&terminal).contains("No cached terminal content"));
    state.sessions[1].connected = true;
    state.open();
    state.confirm();
    assert!(state.sessions[1].controlled);
    assert!(!state.inspecting);
}

#[test]
fn keyboard_prefix_paste_and_menu_editing_are_isolated() {
    let mut state = state();
    key(
        &mut state,
        KeyCode::Char(']'),
        KeyModifiers::CONTROL,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Char('s'),
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Char('b'),
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Char('u'),
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Home,
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Delete,
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    assert_eq!(state.query.iter().collect::<String>(), "u");
    key(
        &mut state,
        KeyCode::Char('t'),
        KeyModifiers::CONTROL,
        Prefix::CtrlBracket,
    );
    assert!(!state.sessions[1].controlled);
    input::handle(
        &mut state,
        Event::Paste("\u{1d}x\ntext".into()),
        &ui::Hits::default(),
        Prefix::CtrlBracket,
        true,
    );
    assert!(!state.exiting);
    assert!(state.input.is_empty());
    key(
        &mut state,
        KeyCode::Esc,
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Char(']'),
        KeyModifiers::CONTROL,
        Prefix::CtrlBracket,
    );
    key(
        &mut state,
        KeyCode::Char('d'),
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    assert_eq!(state.input, ["\u{1d}d"]);
}

#[test]
fn single_click_only_highlights_and_enter_acquires() {
    let mut state = state();
    state.open();
    let (_, hits) = frame(&state, 100, 32);
    let row = hits.rows.iter().find(|(id, _)| *id == 2).unwrap().1;
    input::handle(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: row.x,
            row: row.y,
            modifiers: KeyModifiers::NONE,
        }),
        &hits,
        Prefix::CtrlBracket,
        true,
    );
    assert_eq!(state.highlight, Some(2));
    assert_eq!(state.selected, Some(1));
    assert!(!state.sessions[1].controlled);
    key(
        &mut state,
        KeyCode::Enter,
        KeyModifiers::NONE,
        Prefix::CtrlBracket,
    );
    assert_eq!(state.selected, Some(2));
    assert!(state.sessions[1].controlled);
}

#[test]
fn double_selector_border_mixed_search_junctions_and_status_title_columns() {
    let mut state = state();
    state.open();
    let (terminal, hits) = frame(&state, 100, 32);
    let buffer = terminal.backend().buffer();
    assert_eq!(buffer[(hits.overlay.x, hits.overlay.y)].symbol(), "╔");
    assert_eq!(buffer[(hits.overlay.x, hits.search.y + 1)].symbol(), "╟");
    assert_eq!(
        buffer[(hits.overlay.right() - 1, hits.search.y + 1)].symbol(),
        "╢"
    );
    let text = text(&terminal);
    assert!(text.contains("Needs attention"));
    assert!(text.contains("Review"));
    assert!(!text.contains("Highlighted session"));
    assert!(!text.contains("control elsewhere"));
    assert!(!text.contains("[Select]"));
    assert!(!text.contains("Enter selects"));
    assert!(!text.contains("* current"));
    assert!(!text.contains("Directory:"));
}

#[test]
fn toast_size_hint_row_animation_and_clockwise_erasure() {
    let mut state = state();
    state.open();
    state.tick_toast(200);
    let (terminal, hits) = frame(&state, 100, 32);
    let toast = hits.toast.unwrap();
    assert!(toast.width <= 40);
    assert_eq!(toast.height, 4);
    assert_eq!(
        terminal.backend().buffer()[(toast.x, toast.y)].symbol(),
        "┌"
    );
    assert_eq!(
        terminal.backend().buffer()[(toast.x, toast.y)].fg,
        Color::Yellow
    );
    assert_eq!(
        terminal.backend().buffer()[(toast.right() - 1, toast.y)].symbol(),
        "┐"
    );
    assert_eq!(
        terminal.backend().buffer()[(toast.x, toast.bottom() - 1)].symbol(),
        " "
    );
    assert!(!text(&terminal).contains("Ctrl+]+j"));
    state.tick_toast(500);
    let (terminal, _) = frame(&state, 100, 32);
    assert!(text(&terminal).contains("Ctrl+]+j"));
    state.tick_toast(2500);
    let (terminal, _) = frame(&state, 100, 32);
    let title_color = terminal.backend().buffer()[(toast.x + 1, toast.y + 1)].fg;
    assert!(matches!(title_color, Color::Rgb(value, _, _) if value > 0 && value < 245));
    state.tick_toast(2800);
    let (terminal, _) = frame(&state, 100, 32);
    assert_eq!(
        terminal.backend().buffer()[(toast.x, toast.y)].symbol(),
        " "
    );
    assert_eq!(
        terminal.backend().buffer()[(toast.x, toast.bottom() - 1)].symbol(),
        "└"
    );
    state.hints = false;
    let (_, hits) = frame(&state, 100, 32);
    assert_eq!(hits.toast.unwrap().height, 3);
    state.tick_toast(3200);
    assert!(frame(&state, 100, 32).1.toast.is_none());
}

#[test]
fn toast_click_dismisses_without_takeover_and_episodes_do_not_duplicate() {
    let mut state = state();
    state.open();
    state.highlight = Some(3);
    state.scenario(4);
    assert_eq!(state.toast_queue.len(), 0);
    state.highlight = Some(2);
    state.scenario(4);
    assert_eq!(state.toast_queue.len(), 1);
    state.scenario(4);
    assert_eq!(state.toast_queue.len(), 1);
    state.tick_toast(100);
    let (_, hits) = frame(&state, 100, 32);
    let toast = hits.toast.unwrap();
    input::handle(
        &mut state,
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: toast.x + 1,
            row: toast.y + 1,
            modifiers: KeyModifiers::NONE,
        }),
        &hits,
        Prefix::CtrlBracket,
        true,
    );
    assert_eq!(state.selected, Some(1));
    assert!(!state.sessions[1].controlled);
    assert_eq!(state.sessions[2].status, Status::NeedsAttention);
    state.tick_toast(600);
    assert_eq!(state.toast.as_ref().unwrap().title, "Build");
}

#[test]
fn attention_jump_uses_current_connected_state_not_old_highlight() {
    let mut state = state();
    state.sessions[1].status = Status::NeedsAttention;
    state.sessions[1].changed = 40;
    state.sessions[1].connected = false;
    state.open();
    state.highlight = Some(1);
    state.jump_attention();
    assert_eq!(state.selected, Some(3));
    assert_eq!(state.sessions[2].status, Status::Acknowledged);
    state.jump_attention();
    assert_eq!(state.message, "No sessions need attention");
}

#[test]
fn no_matches_small_terminal_and_unicode_search_cursor_remain_usable() {
    let mut state = state();
    state.open();
    state.insert_search("no match");
    assert!(text(&frame(&state, 26, 14).0).contains("No matching sessions"));
    assert!(text(&frame(&state, 12, 3).0).contains("too small"));
    state.query.clear();
    state.cursor = 0;
    state.insert_search(&"界".repeat(80));
    let (mut terminal, hits) = frame(&state, 26, 14);
    let cursor = terminal.backend_mut().get_cursor_position().unwrap();
    assert!(hits.search.contains(cursor));
}

#[test]
fn monitor_program_status_tracks_connected_pending_attention_not_toast_visibility() {
    let mut state = state();
    assert!(
        state
            .program_status()
            .contains("state=blocked:kind=question:app=climon2-monitor")
    );
    state.dismiss_toast();
    state.tick_toast(4000);
    assert!(state.program_status().contains("state=blocked"));
    state.sessions[2].connected = false;
    assert!(state.program_status().contains("state=working"));
    state.sessions[2].connected = true;
    state.sessions[2].status = Status::Acknowledged;
    assert!(state.program_status().contains("state=working"));
    state.sessions[2].status = Status::Failed;
    assert!(state.program_status().contains("state=working"));
    state.sessions[2].status = Status::NeedsAttention;
    state.jump_attention();
    assert!(state.program_status().contains("state=working"));
}
