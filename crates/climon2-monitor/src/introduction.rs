//! The modal key introduction (selector document, "Key introduction and
//! help"): the configured prefix followed by each prefix action's key, the
//! **Don't show again** checkbox showing the saved choice, and the Continue
//! action that Enter takes. It opens at startup or on prefix+?.
use crate::{keys::BINDINGS, preference::Preference};
use climon2_core::keys::ControlKey;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph, Widget},
};

const ENTER: u8 = b'\r';
const SPACE: u8 = b' ';

/// The key introduction, open or closed, and the choice it shows.
pub(crate) struct KeyIntroduction {
    prefix: ControlKey,
    preference: Preference,
    open: bool,
}

impl KeyIntroduction {
    /// At startup the key introduction opens only when key hints are on and
    /// **Don't show again** is not saved.
    pub(crate) fn new(prefix: ControlKey, preference: Preference, hints: bool) -> Self {
        let open = hints && !preference.suppressed();
        Self {
            prefix,
            preference,
            open,
        }
    }

    /// Prefix+?, even with key hints off or **Don't show again** saved; while
    /// the key introduction is open it changes nothing.
    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    /// A typed key, which the key introduction owns while it is open: Enter
    /// continues, and Space reverses the saved choice.
    pub(crate) fn key(&mut self, byte: u8) {
        if !self.open {
            return;
        }
        match byte {
            ENTER => self.open = false,
            SPACE => self.preference.reverse(),
            _ => {}
        }
    }

    /// Why the saved choice is unknown, or was not saved, if it is.
    pub(crate) fn problem(&self) -> Option<&str> {
        self.preference.problem()
    }

    /// Draws the dialog centred in `area`, over whatever `buffer` holds there,
    /// while the key introduction is open.
    pub(crate) fn render(&self, area: Rect, buffer: &mut Buffer) {
        if !self.open {
            return;
        }
        let lines = self.lines();
        // Each side has a border and a column of space.
        let width = lines.iter().map(Line::width).max().unwrap_or_default() + 4;
        let height = lines.len() + 2;
        let dialog = area.centered(length(width), length(height));
        Clear.render(dialog, buffer);
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .title(" Keys ")
                    .padding(Padding::horizontal(1)),
            )
            .render(dialog, buffer);
    }

    fn lines(&self) -> Vec<Line<'static>> {
        let prefix = self.prefix;
        let mut lines: Vec<_> = BINDINGS
            .iter()
            .map(|binding| {
                let key = char::from(binding.key);
                Line::from(format!("{prefix} then {key}  {}", binding.label))
            })
            .collect();
        let mark = if self.preference.suppressed() {
            'x'
        } else {
            ' '
        };
        lines.push(Line::default());
        lines.push(Line::from(format!("[{mark}] Don't show again (Space)")));
        lines.extend(
            self.preference
                .problem()
                .map(|problem| Line::from(problem.to_owned())),
        );
        lines.push(Line::default());
        lines.push(Line::from("Continue (Enter)"));
        lines
    }
}

fn length(cells: usize) -> Constraint {
    Constraint::Length(u16::try_from(cells).unwrap_or(u16::MAX))
}
