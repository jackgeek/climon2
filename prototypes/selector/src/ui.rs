use crate::{Focus, State, Variant};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

#[derive(Default)]
pub struct Hits {
    pub rows: Vec<(u128, Rect)>,
    pub buttons: Vec<(Focus, Rect)>,
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

fn box_text(frame: &mut Frame, area: Rect, title: &str, text: String) {
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title.to_owned()),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn details_text(state: &State) -> String {
    match state.highlight.and_then(|id| state.session(id)) {
        Some(session) => format!(
            "Title: {}\nDirectory: {}\nStatus: {}",
            literal(session.label()),
            literal(&session.cwd),
            session.status_text(),
        ),
        None => "No session highlighted".into(),
    }
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
    if area.width < 24 || area.height < 10 {
        frame.render_widget(Paragraph::new("Terminal\ntoo small"), content);
        return hits;
    }
    let terminal_text = match selected {
        None => "Selected session is unavailable. Open the selector.".into(),
        Some(session) if state.inspecting || session.final_outcome() => {
            let end = session.cache.len().saturating_sub(state.history_offset);
            format!(
                "READ-ONLY retained terminal\n{}",
                session.cache[..end]
                    .iter()
                    .rev()
                    .take(usize::from(content.height.saturating_sub(4)))
                    .rev()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
        Some(session) if !session.connected => {
            "Disconnected. Explicitly inspect cached terminal from the selector.".into()
        }
        Some(session) if !session.controlled => {
            let mut text = "Session is being viewed elsewhere.".to_owned();
            if state.hints {
                text.push_str(&format!(
                    "\n{}+s opens selector for explicit selection; Ctrl+T takes control.",
                    state.prefix_label
                ));
            }
            text
        }
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
    frame.render_widget(Paragraph::new(literal_multiline(&terminal_text)), content);
    let footer = Rect::new(0, area.height - 3, area.width, 3);
    let selected_label = selected
        .map(|s| literal(s.label()))
        .unwrap_or_else(|| "(none)".into());
    let highlight_label = state
        .highlight
        .and_then(|id| state.session(id))
        .map(|s| literal(s.label()))
        .unwrap_or_else(|| "(none)".into());
    let debug = format!(
        "PROTOTYPE {:?} | selected: {} | highlight: {} | menu:{} cache:{} input:{}\n{}\nF2 layout F3 hints F4 attention F5 title F6 disconnect F7 remove F8 reset",
        state.variant,
        selected_label,
        highlight_label,
        state.menu,
        state.inspecting,
        state.input.len(),
        state.message,
    );
    frame.render_widget(
        Paragraph::new(debug).style(Style::default().fg(Color::Yellow).bg(Color::Black)),
        footer,
    );
    if !state.menu {
        if state.inspecting {
            let button = Rect::new(0, area.height - 4, 21, 1);
            frame.render_widget(
                Paragraph::new("[Return to session]").style(if state.focus == Focus::Cancel {
                    highlighted()
                } else {
                    Style::default().fg(Color::Cyan)
                }),
                button,
            );
            hits.buttons.push((Focus::Cancel, button));
        }
        if state.hints && !state.inspecting {
            frame.render_widget(
                Paragraph::new(format!(
                    "{}+s selector  {}+x exit  {}+j attention",
                    state.prefix_label, state.prefix_label, state.prefix_label
                )),
                Rect::new(0, 1, area.width, 1),
            );
        }
        return hits;
    }

    let width = area.width.saturating_sub(4).min(94);
    let height = area
        .height
        .saturating_sub(if area.height < 14 { 2 } else { 4 })
        .min(26);
    let overlay = Rect::new((area.width - width) / 2, 1, width, height);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Sessions - {:?} ", state.variant));
    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);
    if state.details {
        let parts = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(inner);
        frame.render_widget(
            Paragraph::new(details_text(state))
                .wrap(Wrap { trim: false })
                .scroll((state.details_offset, 0)),
            parts[0],
        );
        frame.render_widget(Paragraph::new("[Back]").style(highlighted()), parts[1]);
        hits.buttons.push((Focus::Cancel, parts[1]));
        return hits;
    }
    let narrow = inner.width < 60;
    let show_hints = state.hints && inner.height >= 7;
    let sections = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(if narrow { 2 } else { 1 }),
        Constraint::Length(u16::from(show_hints)),
    ])
    .split(inner);
    hits.search = sections[0];
    let search_width = usize::from(hits.search.width.saturating_sub(2)).max(1);
    let mut first = 0;
    while first < state.cursor
        && Line::from(state.query[first..state.cursor].iter().collect::<String>()).width()
            >= search_width
    {
        first += 1;
    }
    let query = state.query[first..].iter().collect::<String>();
    frame.render_widget(
        Paragraph::new(query).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search titles / command arguments"),
        ),
        hits.search,
    );
    let compact = inner.width < 60 || sections[1].height < 9;
    let (list, details) = if compact {
        (sections[1], None)
    } else {
        match state.variant {
            Variant::A => {
                let parts = Layout::vertical([Constraint::Min(4), Constraint::Length(6)])
                    .split(sections[1]);
                (parts[0], Some(parts[1]))
            }
            Variant::B => {
                let parts =
                    Layout::horizontal([Constraint::Percentage(53), Constraint::Percentage(47)])
                        .split(sections[1]);
                (parts[0], Some(parts[1]))
            }
            Variant::C => {
                let parts = Layout::vertical([Constraint::Length(6), Constraint::Min(4)])
                    .split(sections[1]);
                (parts[1], Some(parts[0]))
            }
        }
    };
    if let Some(details) = details {
        box_text(frame, details, "Highlighted session", details_text(state));
    }
    hits.list = list;
    let row_height = if state.variant == Variant::C && !compact {
        3
    } else {
        1
    };
    hits.page_size = usize::from(list.height / row_height).max(1);
    let ids = state.visible_ids();
    hits.start = state.page_start(hits.page_size);
    if ids.is_empty() {
        frame.render_widget(Paragraph::new("No matching sessions"), list);
    }
    for (index, id) in ids.iter().skip(hits.start).take(hits.page_size).enumerate() {
        let session = state.session(*id).expect("visible ID belongs to session");
        let y = list.y + index as u16 * row_height;
        let rect = Rect::new(
            list.x,
            y,
            list.width,
            row_height.min(list.bottom().saturating_sub(y)),
        );
        let marker = if state.selected == Some(*id) {
            "*"
        } else {
            " "
        };
        let control = if session.connected && !session.controlled && !session.final_outcome() {
            " / control elsewhere"
        } else {
            ""
        };
        let text = if row_height == 3 {
            format!(
                "{marker} {}\n  {}{control}\n",
                literal(session.label()),
                session.status_text()
            )
        } else {
            format!(
                "{marker} {} | {}{control}",
                literal(session.label()),
                session.status_text()
            )
        };
        let style = if state.highlight == Some(*id) {
            highlighted()
        } else {
            Style::default()
        };
        frame.render_widget(Paragraph::new(text).style(style), rect);
        hits.rows.push((*id, rect));
    }
    let actions = [
        (Focus::Select, "Select"),
        (Focus::Inspect, "Inspect"),
        (Focus::Details, "Details"),
        (Focus::Cancel, "Cancel"),
    ];
    let cells = if narrow {
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)])
            .split(sections[2])
            .iter()
            .flat_map(|row| {
                Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(*row)
                    .to_vec()
            })
            .collect::<Vec<_>>()
    } else {
        Layout::horizontal(
            actions
                .iter()
                .map(|(_, label)| Constraint::Length(label.len() as u16 + 2)),
        )
        .split(sections[2])
        .to_vec()
    };
    for ((focus, label), rect) in actions.into_iter().zip(cells) {
        let enabled = match focus {
            Focus::Select | Focus::Details => state.highlight.is_some(),
            Focus::Inspect => state
                .highlight
                .and_then(|id| state.session(id))
                .is_some_and(|s| !s.cache.is_empty()),
            _ => true,
        };
        let style = if !enabled {
            Style::default().fg(Color::DarkGray)
        } else if state.focus == focus {
            highlighted()
        } else {
            Style::default().fg(Color::Cyan)
        };
        frame.render_widget(Paragraph::new(format!("[{label}]")).style(style), rect);
        hits.buttons.push((focus, rect));
    }
    if show_hints {
        frame.render_widget(
            Paragraph::new(Line::from(
                "Enter selects  Esc cancels  Arrows navigate  Tab actions  * current",
            )),
            sections[3],
        );
    }
    if state.focus == Focus::Search {
        let prefix: String = state.query[first..state.cursor].iter().collect();
        let offset = Line::from(prefix).width() as u16;
        let x = hits.search.x + 1 + offset;
        if x < hits.search.right().saturating_sub(1) {
            frame.set_cursor_position((x, hits.search.y + 1));
        }
    }
    hits
}

fn literal_multiline(text: &str) -> String {
    text.split('\n').map(literal).collect::<Vec<_>>().join("\n")
}
