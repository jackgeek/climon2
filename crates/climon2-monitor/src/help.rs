//! The modal key introduction and help dialog (selector document, "Key
//! introduction and help"): the configured prefix followed by x, s and ?, the
//! **Don't show again** checkbox showing the saved choice, and the Continue
//! action that Enter takes.
use crate::preference::Preference;
use climon2_core::keys::ControlKey;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    text::Line,
    widgets::{Block, Clear, Padding, Paragraph, Widget},
};

const BINDINGS: [(char, &str); 3] = [
    ('x', "Exit the monitor"),
    ('s', "Open the session list"),
    ('?', "Show this dialog"),
];
const ENTER: u8 = b'\r';
const SPACE: u8 = b' ';

/// The dialog, open or closed, and the choice it shows.
pub(crate) struct Help {
    prefix: ControlKey,
    preference: Preference,
    open: bool,
}

/// What the open dialog shows.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Dialog {
    prefix: ControlKey,
    checked: bool,
    problem: Option<String>,
}

impl Help {
    /// At startup the dialog opens as the key introduction only when key
    /// hints are on and **Don't show again** is not saved.
    pub(crate) fn new(prefix: ControlKey, preference: Preference, hints: bool) -> Self {
        let open = hints && !preference.suppressed();
        Self {
            prefix,
            preference,
            open,
        }
    }

    /// Prefix+?, even with key hints off or **Don't show again** saved; while
    /// the dialog is open it changes nothing.
    pub(crate) fn open(&mut self) {
        self.open = true;
    }

    /// A typed key, which the dialog owns while it is open: Enter continues,
    /// and Space reverses the saved choice.
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

    /// The dialog to show, if it is open.
    pub(crate) fn dialog(&self) -> Option<Dialog> {
        self.open.then(|| Dialog {
            prefix: self.prefix,
            checked: self.preference.suppressed(),
            problem: self.preference.problem().map(str::to_owned),
        })
    }
}

impl Dialog {
    /// Draws the dialog centred in `area`, over whatever `buffer` holds there.
    pub(crate) fn render(&self, area: Rect, buffer: &mut Buffer) {
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
            .map(|(key, action)| Line::from(format!("{prefix} then {key}  {action}")))
            .collect();
        let mark = if self.checked { 'x' } else { ' ' };
        lines.push(Line::default());
        lines.push(Line::from(format!("[{mark}] Don't show again (Space)")));
        lines.extend(self.problem.clone().map(Line::from));
        lines.push(Line::default());
        lines.push(Line::from("Continue (Enter)"));
        lines
    }
}

fn length(cells: usize) -> Constraint {
    Constraint::Length(u16::try_from(cells).unwrap_or(u16::MAX))
}
