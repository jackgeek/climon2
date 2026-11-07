use crate::{State, ui::Hits};
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    CtrlBracket,
    Escape,
}

impl Prefix {
    pub fn label(self) -> &'static str {
        match self {
            Self::CtrlBracket => "Ctrl+]",
            Self::Escape => "Ctrl+[",
        }
    }

    fn matches(self, key: KeyEvent) -> bool {
        match self {
            Self::CtrlBracket => control_byte(key) == Some(0x1d),
            Self::Escape => key.code == KeyCode::Esc,
        }
    }
}

pub fn handle(state: &mut State, event: Event, hits: &Hits, prefix: Prefix, usable: bool) {
    match event {
        Event::Paste(text) => {
            if state.pending_prefix {
                state.pending_prefix = false;
                if usable {
                    literal_input(
                        state,
                        if prefix == Prefix::Escape {
                            "\u{1b}"
                        } else {
                            "\u{1d}"
                        },
                    );
                }
            }
            if usable {
                literal_input(state, &text);
            }
        }
        Event::Key(key) if key.kind != KeyEventKind::Release => {
            key_input(state, key, hits, prefix, usable)
        }
        Event::Mouse(mouse) => {
            let point = ratatui::layout::Position::new(mouse.column, mouse.row);
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && hits.toast.is_some_and(|rect| rect.contains(point))
            {
                state.dismiss_toast();
                return;
            }
            if state.menu {
                if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                    if let Some((id, _)) = hits.rows.iter().find(|(_, rect)| rect.contains(point)) {
                        state.highlight = Some(*id);
                        state.manual_scroll = false;
                    }
                } else if hits.list.contains(point) {
                    let maximum = state.visible_ids().len().saturating_sub(hits.page_size);
                    match mouse.kind {
                        MouseEventKind::ScrollDown => {
                            state.list_offset = (hits.start + 3).min(maximum)
                        }
                        MouseEventKind::ScrollUp => {
                            state.list_offset = hits.start.saturating_sub(3)
                        }
                        _ => return,
                    }
                    state.manual_scroll = true;
                }
            } else if usable {
                let maximum = state
                    .selected
                    .and_then(|id| state.session(id))
                    .map_or(0, |s| s.cache.len().saturating_sub(1));
                match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        state.history_offset = (state.history_offset + 3).min(maximum)
                    }
                    MouseEventKind::ScrollDown => {
                        state.history_offset = state.history_offset.saturating_sub(3)
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

fn key_input(state: &mut State, key: KeyEvent, hits: &Hits, prefix: Prefix, usable: bool) {
    if prefix == Prefix::Escape && key.modifiers == KeyModifiers::ALT {
        state.pending_prefix = true;
        key_input(
            state,
            KeyEvent::new(key.code, KeyModifiers::NONE),
            hits,
            prefix,
            usable,
        );
        return;
    }
    if state.pending_prefix {
        state.pending_prefix = false;
        if prefix.matches(key) {
            if usable {
                literal_input(
                    state,
                    if prefix == Prefix::Escape {
                        "\u{1b}"
                    } else {
                        "\u{1d}"
                    },
                );
            }
            return;
        }
        match key.code {
            KeyCode::Char('s') if key.modifiers.is_empty() => state.open(),
            KeyCode::Char('x') if key.modifiers.is_empty() => state.exiting = true,
            KeyCode::Char('j') if key.modifiers.is_empty() => state.jump_attention(),
            _ if usable => {
                let mut text = if prefix == Prefix::Escape {
                    "\u{1b}"
                } else {
                    "\u{1d}"
                }
                .to_owned();
                text.push_str(&key_text(key));
                literal_input(state, &text);
            }
            _ => {}
        }
        return;
    }
    if prefix.matches(key) {
        state.pending_prefix = true;
        return;
    }
    match key.code {
        KeyCode::F(3) => state.hints = !state.hints,
        KeyCode::F(number @ 4..=8) => state.scenario(number),
        KeyCode::Esc if state.menu => state.cancel(),
        _ if !usable => {}
        _ if state.menu => menu_key(state, key, hits),
        KeyCode::Char('t')
            if key.modifiers.contains(KeyModifiers::CONTROL) && !state.inspecting =>
        {
            if let Some(session) = state.selected.and_then(|id| state.session(id)) {
                if session.connected && !session.controlled && !session.final_outcome() {
                    state.highlight = Some(session.id);
                    state.confirm();
                } else {
                    state.application_input("\u{14}");
                }
            }
        }
        _ => state.application_input(&key_text(key)),
    }
}

fn menu_key(state: &mut State, key: KeyEvent, hits: &Hits) {
    match key.code {
        KeyCode::Up => state.move_highlight(-1),
        KeyCode::Down => state.move_highlight(1),
        KeyCode::PageUp => state.move_highlight(-(hits.page_size as isize)),
        KeyCode::PageDown => state.move_highlight(hits.page_size as isize),
        KeyCode::Enter => state.confirm(),
        KeyCode::Backspace if state.cursor > 0 => {
            state.cursor -= 1;
            state.query.remove(state.cursor);
            state.search_changed();
        }
        KeyCode::Delete if state.cursor < state.query.len() => {
            state.query.remove(state.cursor);
            state.search_changed();
        }
        KeyCode::Left => state.cursor = state.cursor.saturating_sub(1),
        KeyCode::Right => state.cursor = (state.cursor + 1).min(state.query.len()),
        KeyCode::Home => state.cursor = 0,
        KeyCode::End => state.cursor = state.query.len(),
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            state.insert_search(&ch.to_string())
        }
        _ => {}
    }
}

fn literal_input(state: &mut State, text: &str) {
    if state.menu {
        state.insert_search(text);
    } else {
        state.application_input(text);
    }
}

fn key_text(key: KeyEvent) -> String {
    if let Some(byte) = control_byte(key) {
        return char::from(byte).to_string();
    }
    match key.code {
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Enter => "\r".into(),
        KeyCode::Backspace => "\u{7f}".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::Esc => "\u{1b}".into(),
        KeyCode::Up => "\u{1b}[A".into(),
        KeyCode::Down => "\u{1b}[B".into(),
        KeyCode::Right => "\u{1b}[C".into(),
        KeyCode::Left => "\u{1b}[D".into(),
        _ => String::new(),
    }
}

fn control_byte(key: KeyEvent) -> Option<u8> {
    match key.code {
        KeyCode::Char(ch) if key.modifiers.contains(KeyModifiers::CONTROL) && ch.is_ascii() => {
            // Crossterm decodes legacy bytes 0x1c..0x1f as Ctrl+4..Ctrl+7.
            Some(match ch {
                '4'..='7' => ch as u8 - b'4' + 0x1c,
                _ => ch as u8 & 0x1f,
            })
        }
        _ => None,
    }
}
