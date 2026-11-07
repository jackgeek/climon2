use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use monitor_selector_prototype::{
    Focus,
    input::{self, Prefix},
};
use monitor_selector_prototype::{Outcome, Session, State, Status, Variant, ui};
use ratatui::{Terminal, backend::TestBackend};

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
fn filtering_is_literal_case_insensitive_and_paste_is_not_a_command() {
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
fn asynchronous_reordering_preserves_identity_but_removal_cannot_redirect_enter() {
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
    assert_eq!(state.highlight, Some(3));
    state.confirm();
    assert_eq!(state.selected, Some(3));
    assert!(!state.menu);
}

#[test]
fn updated_title_losing_match_requires_new_explicit_highlight() {
    let mut state = state();
    state.open();
    state.insert_search("build");
    state.sessions[1].title = "Other".into();
    state.reconcile();
    assert_eq!(state.highlight, None);
    state.confirm();
    assert_eq!(state.selected, Some(1));
    assert!(state.menu);
}

#[test]
fn navigation_does_not_wrap_and_search_editing_preserves_unicode() {
    let mut state = state();
    state.open();
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
fn inspection_never_acquires_and_disconnected_confirmation_never_falls_back() {
    let mut state = state();
    state.open();
    state.highlight = Some(2);
    state.inspect();
    assert_eq!(state.selected, Some(2));
    assert!(state.inspecting);
    assert!(!state.sessions[1].controlled);
    state.application_input("blocked");
    assert!(state.input.is_empty());
    state.cancel();
    assert!(!state.inspecting);
    assert!(!state.sessions[1].controlled);
    state.open();
    state.sessions[1].connected = false;
    state.confirm();
    assert_eq!(state.selected, Some(2));
    assert!(!state.sessions[1].controlled);
    assert!(!state.menu);
    assert!(state.message.contains("disconnected"));
}

#[test]
fn passive_session_without_cache_cannot_be_inspected() {
    let mut state = state();
    state.sessions[1].cache.clear();
    state.open();
    state.highlight = Some(2);
    state.inspect();
    assert!(state.menu);
    assert_eq!(state.selected, Some(1));
    assert!(state.message.contains("No cached"));
}

#[test]
fn attention_jump_uses_current_connected_state_not_old_menu_highlight() {
    let mut state = state();
    state.sessions[1].status = Status::NeedsAttention;
    state.sessions[1].changed = 40;
    state.sessions[1].connected = false;
    state.open();
    state.highlight = Some(1);
    state.jump_attention();
    assert_eq!(state.selected, Some(3));
    assert!(!state.menu);
    assert_eq!(state.sessions[2].status, Status::Acknowledged);
    state.jump_attention();
    assert_eq!(state.selected, Some(3));
    assert_eq!(state.message, "No sessions need attention");
}

#[test]
fn selector_variants_render_minimal_details_and_truthful_signal_failure() {
    for variant in [Variant::A, Variant::B, Variant::C] {
        let mut state = state();
        state.variant = variant;
        state.sessions[1].status = Status::Failed;
        state.sessions[1].outcome = Some(Outcome::Signal(9));
        state.open();
        state.highlight = Some(2);
        let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
        terminal.draw(|frame| ui::render(frame, &state)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("Build"));
        assert!(text.contains("/work/project"));
        assert!(text.contains("signal 9"));
        assert!(!text.contains("00000000-"));
    }
}

#[test]
fn compact_details_no_matches_too_small_and_hidden_hints_are_visible_behaviors() {
    let mut state = state();
    state.open();
    state.highlight = Some(2);
    state.details = true;
    let mut terminal = Terminal::new(TestBackend::new(35, 10)).unwrap();
    terminal.draw(|frame| ui::render(frame, &state)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("/work/project"));
    state.details = false;
    state.insert_search("no match");
    terminal.draw(|frame| ui::render(frame, &state)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("No matching sessions"));
    state.hints = false;
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
    terminal.draw(|frame| ui::render(frame, &state)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(!text.contains("Enter selects"));
    assert!(text.contains("Cancel"));
    let mut terminal = Terminal::new(TestBackend::new(12, 3)).unwrap();
    terminal.draw(|frame| ui::render(frame, &state)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("too small"));
}

fn key(state: &mut State, code: KeyCode, modifiers: KeyModifiers, prefix: Prefix) {
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut hits = ui::Hits::default();
    terminal
        .draw(|frame| hits = ui::paint(frame, state))
        .unwrap();
    input::handle(
        state,
        Event::Key(KeyEvent::new(code, modifiers)),
        &hits,
        prefix,
        true,
    );
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
fn single_click_only_highlights_and_explicit_select_button_acquires() {
    let mut state = state();
    state.open();
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut hits = ui::Hits::default();
    terminal
        .draw(|frame| hits = ui::paint(frame, &state))
        .unwrap();
    let row = hits.rows.iter().find(|(id, _)| *id == 2).unwrap().1;
    let click = |rect: ratatui::layout::Rect| {
        Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x,
            row: rect.y,
            modifiers: KeyModifiers::NONE,
        })
    };
    input::handle(&mut state, click(row), &hits, Prefix::CtrlBracket, true);
    assert_eq!(state.highlight, Some(2));
    assert_eq!(state.selected, Some(1));
    assert!(!state.sessions[1].controlled);
    let button = hits
        .buttons
        .iter()
        .find(|(focus, _)| *focus == Focus::Select)
        .unwrap()
        .1;
    input::handle(&mut state, click(button), &hits, Prefix::CtrlBracket, true);
    assert_eq!(state.selected, Some(2));
    assert!(state.sessions[1].controlled);
}

#[test]
fn compact_actions_and_escape_prefix_details_have_an_accessible_cancel() {
    let mut state = state();
    state.open();
    let mut terminal = Terminal::new(TestBackend::new(26, 14)).unwrap();
    let mut hits = ui::Hits::default();
    terminal
        .draw(|frame| hits = ui::paint(frame, &state))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("[Cancel]"), "{text}");
    assert!(text.contains("[Details]"), "{text}");
    state.details = true;
    key(&mut state, KeyCode::Tab, KeyModifiers::NONE, Prefix::Escape);
    key(
        &mut state,
        KeyCode::Enter,
        KeyModifiers::NONE,
        Prefix::Escape,
    );
    assert!(!state.details);
    assert!(state.menu);
}

#[test]
fn hidden_hints_do_not_leak_takeover_shortcuts_in_placeholder() {
    let mut state = state();
    state.hints = false;
    state.selected = Some(2);
    let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
    terminal.draw(|frame| ui::render(frame, &state)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("being viewed elsewhere"));
    assert!(!text.contains("Ctrl+T"));
    assert!(!text.contains("Ctrl+]+s"));
}
