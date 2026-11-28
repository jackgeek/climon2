use crate::State;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

#[derive(Default)]
pub struct Hits {
    pub overlay: Rect,
    pub toast: Option<Rect>,
    pub rows: Vec<(u128, Rect)>,
    pub search: Rect,
    pub list: Rect,
    pub page_size: usize,
    pub start: usize,
}

pub fn literal(text: &str) -> String {
    text.chars()
        .flat_map(|ch| {
            if ch.is_control() {
                format!("\\u{{{:x}}}", u32::from(ch))
                    .chars()
                    .collect::<Vec<_>>()
            } else {
                vec![ch]
            }
        })
        .collect()
}

fn highlighted() -> Style {
    Style::default()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn render(frame: &mut Frame, state: &State) {
    paint(frame, state);
}

pub fn paint(frame: &mut Frame, state: &State) -> Hits {
    let area = frame.area();
    let mut hits = Hits::default();
    let selected = state.selected.and_then(|id| state.session(id));
    let chrome = match selected {
        Some(session) => format!("{} | {}", session.status_text(), literal(session.label())),
        None => "No selected session".into(),
    };
    frame.render_widget(
        Paragraph::new(chrome).style(Style::default().bg(Color::DarkGray)),
        Rect::new(0, 0, area.width, 1),
    );
    let content = Rect::new(0, 1, area.width, area.height.saturating_sub(1));
    if area.width < 24 || area.height < 13 {
        frame.render_widget(Paragraph::new("Terminal\ntoo small"), content);
        return hits;
    }
    let terminal_text = match selected {
        None => "Selected session is unavailable.".into(),
        Some(session) if state.inspecting || session.final_outcome() => {
            let end = session.cache.len().saturating_sub(state.history_offset);
            let history = if session.cache.is_empty() {
                "No cached terminal content is available".into()
            } else {
                session.cache[..end]
                    .iter()
                    .rev()
                    .take(usize::from(content.height.saturating_sub(4)))
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            };
            format!("READ-ONLY history inspector\n{history}")
        }
        Some(session) if !session.connected => {
            "Disconnected - select this session to inspect retained history.".into()
        }
        Some(session) if !session.controlled => "Session is being viewed elsewhere.".into(),
        Some(session) => {
            let end = session.cache.len().saturating_sub(state.history_offset);
            session.cache[..end]
                .iter()
                .rev()
                .take(usize::from(content.height.saturating_sub(4)))
                .rev()
                .cloned()
                .collect::<Vec<_>>()
                .join("\n")
        }
    };
    frame.render_widget(
        Paragraph::new(
            terminal_text
                .split('\n')
                .map(literal)
                .collect::<Vec<_>>()
                .join("\n"),
        ),
        content,
    );
    let footer = Rect::new(0, area.height - 3, area.width, 3);
    let selected_label = selected
        .map(|s| literal(s.label()))
        .unwrap_or_else(|| "(none)".into());
    let highlight_label = state
        .highlight
        .and_then(|id| state.session(id))
        .map(|s| literal(s.label()))
        .unwrap_or_else(|| "(none)".into());
    frame.render_widget(Paragraph::new(format!(
        "PROTOTYPE | selected: {} | highlight: {} | menu:{} history:{} input:{}\n{}\nF3 hints F4 attention/toast F5 title F6 disconnect F7 remove F8 reset",
        selected_label, highlight_label, state.menu, state.inspecting, state.input.len(), state.message,
    )).style(Style::default().fg(Color::Yellow).bg(Color::Black)), footer);
    if state.menu {
        hits = selector(frame, state);
    }
    hits.toast = toast(frame, state);
    hits
}

fn selector(frame: &mut Frame, state: &State) -> Hits {
    let area = frame.area();
    let width = area.width.saturating_sub(4).min(94);
    let height = area.height.saturating_sub(8).min(26);
    let overlay = Rect::new((area.width - width) / 2, 1, width, height);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double);
    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);
    let search = Rect::new(inner.x, inner.y, inner.width, 1);
    let search_width = usize::from(search.width).max(1);
    let mut first = 0;
    while first < state.cursor
        && Line::from(state.query[first..state.cursor].iter().collect::<String>()).width()
            >= search_width
    {
        first += 1;
    }
    frame.render_widget(
        Paragraph::new(state.query[first..].iter().collect::<String>()),
        search,
    );
    let y = search.y + 1;
    frame.render_widget(
        Paragraph::new("─".repeat(usize::from(inner.width))),
        Rect::new(inner.x, y, inner.width, 1),
    );
    frame.render_widget(Paragraph::new("╟"), Rect::new(overlay.x, y, 1, 1));
    frame.render_widget(Paragraph::new("╢"), Rect::new(overlay.right() - 1, y, 1, 1));
    let list = Rect::new(inner.x, y + 1, inner.width, inner.height.saturating_sub(2));
    let page_size = usize::from(list.height).max(1);
    let start = state.page_start(page_size);
    let mut hits = Hits {
        overlay,
        search,
        list,
        page_size,
        start,
        ..Hits::default()
    };
    let ids = state.visible_ids();
    if ids.is_empty() {
        frame.render_widget(Paragraph::new("No matching sessions"), list);
    }
    for (index, id) in ids.iter().skip(start).take(page_size).enumerate() {
        let session = state.session(*id).expect("visible ID belongs to session");
        let row = Rect::new(list.x, list.y + index as u16, list.width, 1);
        let columns = Layout::horizontal([
            Constraint::Length(20.min(list.width / 2)),
            Constraint::Length(2),
            Constraint::Min(1),
        ])
        .split(row);
        let style = if state.highlight == Some(*id) {
            highlighted()
        } else {
            Style::default()
        };
        frame.render_widget(
            Paragraph::new(" ".repeat(usize::from(row.width))).style(style),
            row,
        );
        frame.render_widget(
            Paragraph::new(session.status_text()).style(style),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(literal(session.label())).style(style),
            columns[2],
        );
        hits.rows.push((*id, row));
    }
    let offset =
        Line::from(state.query[first..state.cursor].iter().collect::<String>()).width() as u16;
    if offset < search.width {
        frame.set_cursor_position((search.x + offset, search.y));
    }
    hits
}

fn toast(frame: &mut Frame, state: &State) -> Option<Rect> {
    let toast = state.toast.as_ref()?;
    let progress = toast.frame(state.now_ms)?;
    let area = frame.area();
    let hint = format!("{}+j newest attention", state.prefix_label);
    let title = literal(&toast.title);
    let desired = Line::from(title.clone())
        .width()
        .max(if state.hints { hint.len() } else { 0 })
        + 2;
    let width = (desired.min(usize::from(area.width) * 2 / 5) as u16).max(3);
    let height = if state.hints { 4 } else { 3 };
    let rect = Rect::new(area.width - width, area.height - 3 - height, width, height);
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new("").style(Style::default().bg(Color::Black)),
        rect,
    );
    let mut border = vec![(rect.x, rect.y, "┌")];
    for x in rect.x + 1..rect.right() - 1 {
        border.push((x, rect.y, "─"));
    }
    border.push((rect.right() - 1, rect.y, "┐"));
    for y in rect.y + 1..rect.bottom() - 1 {
        border.push((rect.right() - 1, y, "│"));
    }
    border.push((rect.right() - 1, rect.bottom() - 1, "┘"));
    for x in (rect.x + 1..rect.right() - 1).rev() {
        border.push((x, rect.bottom() - 1, "─"));
    }
    border.push((rect.x, rect.bottom() - 1, "└"));
    for y in (rect.y + 1..rect.bottom() - 1).rev() {
        border.push((rect.x, y, "│"));
    }
    let drawn = (border.len() as f64 * progress.drawn).ceil() as usize;
    let erased = (drawn as f64 * progress.erased).floor() as usize;
    for (x, y, symbol) in border.into_iter().take(drawn).skip(erased) {
        frame.render_widget(
            Paragraph::new(symbol).style(Style::default().fg(Color::Yellow).bg(Color::Black)),
            Rect::new(x, y, 1, 1),
        );
    }
    if progress.alpha > 0.0 {
        let value = (245.0 * progress.alpha).round() as u8;
        let style = Style::default()
            .fg(Color::Rgb(value, value, value))
            .bg(Color::Black);
        frame.render_widget(
            Paragraph::new(ellipsis(&title, rect.width - 2)).style(style),
            Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, 1),
        );
        if state.hints {
            let hint = if usize::from(rect.width - 2) < state.prefix_label.len() + 2 {
                if state.prefix_label == "Ctrl+[" {
                    "^[+j".into()
                } else {
                    "^]+j".into()
                }
            } else {
                hint
            };
            frame.render_widget(
                Paragraph::new(ellipsis(&hint, rect.width - 2)).style(style),
                Rect::new(rect.x + 1, rect.y + 2, rect.width - 2, 1),
            );
        }
    }
    Some(rect)
}

fn ellipsis(text: &str, width: u16) -> String {
    if Line::from(text).width() <= usize::from(width) {
        return text.into();
    }
    let mut result = String::new();
    for ch in text.chars() {
        if Line::from(format!("{result}{ch}…")).width() > usize::from(width) {
            break;
        }
        result.push(ch);
    }
    result.push('…');
    result
}
