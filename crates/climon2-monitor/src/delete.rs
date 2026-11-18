//! The Delete confirmation (selector document, "Delete: terminate and
//! remove"; #117): Delete in the selector opens it on the highlighted row,
//! whose session ID it keeps however rows reorder. Its box, titled
//! `Delete session`, shows the row's label and status as they are now and
//! two buttons, Cancel first with the focus, then Terminate and remove, or
//! Remove once the session is final or gone. Tab, Left and Right move the
//! focus, Enter activates it and Escape cancels (owner decision on #117);
//! nothing else it receives, pastes and mouse reports included, does
//! anything, so no input reaches a session.
use crate::{
    registry::{Offer, View, truncate},
    router::{Key, Routed},
};
use climon2_core::SessionId;
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Clear, Padding, Paragraph, Widget},
};

/// What closing the confirmation decided.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Decision {
    /// Nothing is sent and nothing changes.
    Cancel,
    /// The offer, as it stands now, is confirmed for this session.
    Confirm(SessionId),
}

/// The open confirmation: its session and which button has the focus.
pub(crate) struct Confirmation {
    target: SessionId,
    /// The confirming button has the focus; Cancel has it at first.
    confirming: bool,
}

/// What a key does in the confirmation.
#[derive(Clone, Copy)]
enum Press {
    Cancel,
    Activate,
    /// Moves the focus: from whether the confirming button had it, to
    /// whether it has it.
    Focus(fn(bool) -> bool),
}

/// Right and Left, by their final byte.
const CURSOR_KEYS: [(u8, Press); 2] = [
    (b'C', Press::Focus(|_| true)),
    (b'D', Press::Focus(|_| false)),
];

/// Tab and Enter.
const KEYS: [(&[u8], Press); 2] = [
    (b"\t", Press::Focus(|confirming| !confirming)),
    (b"\r", Press::Activate),
];

/// The key that opens the confirmation from the selector: forward Delete
/// (`CSI 3 ~`) alone (owner decision on #117).
pub(crate) fn requested(input: &Routed) -> bool {
    *input == Routed::Key(Key::Bytes(b"\x1b[3~".to_vec()))
}

impl Confirmation {
    pub(crate) fn new(target: SessionId) -> Self {
        Self {
            target,
            confirming: false,
        }
    }

    pub(crate) fn target(&self) -> &SessionId {
        &self.target
    }

    /// Acts on input while open: a decision closes it.
    pub(crate) fn input(&mut self, input: &Routed) -> Option<Decision> {
        let Routed::Key(key) = input else {
            return None;
        };
        match press(key)? {
            Press::Cancel => Some(Decision::Cancel),
            Press::Activate => Some(self.activate()),
            Press::Focus(moved) => {
                self.confirming = moved(self.confirming);
                None
            }
        }
    }

    fn activate(&self) -> Decision {
        let confirm = Decision::Confirm(self.target.clone());
        [Decision::Cancel, confirm][usize::from(self.confirming)].clone()
    }

    /// Draws the confirmation for `row` with `offer`, centred in `area`
    /// over whatever `buffer` holds there.
    pub(crate) fn render(&self, row: &View, offer: Offer, area: Rect, buffer: &mut Buffer) {
        // Each side has a border and a column of space.
        let room = usize::from(area.width.saturating_sub(4));
        let lines = vec![
            Line::from(truncate(&row.label, room).into_owned()),
            Line::from(truncate(&row.status, room).into_owned()),
            Line::default(),
            self.buttons(offer),
        ];
        let width = lines.iter().map(Line::width).max().unwrap_or_default() + 4;
        let dialog = area.centered(length(width), length(lines.len() + 2));
        Clear.render(dialog, buffer);
        Paragraph::new(lines)
            .block(
                Block::bordered()
                    .title(" Delete session ")
                    .padding(Padding::horizontal(1)),
            )
            .render(dialog, buffer);
    }

    /// Cancel, then the offer, the focused one reversed.
    fn buttons(&self, offer: Offer) -> Line<'static> {
        let confirm = match offer {
            Offer::TerminateAndRemove => "[ Terminate and remove ]",
            Offer::Remove => "[ Remove ]",
        };
        let styles = [Style::new(), Style::new().reversed()];
        let focused = |focused: bool| styles[usize::from(focused)];
        Line::from(vec![
            Span::styled("[ Cancel ]", focused(!self.confirming)),
            Span::raw("  "),
            Span::styled(confirm, focused(self.confirming)),
        ])
    }
}

/// What `key` does in the confirmation, if anything.
fn press(key: &Key) -> Option<Press> {
    match key {
        Key::Escape => Some(Press::Cancel),
        Key::Cursor(last) => found(&CURSOR_KEYS, last),
        Key::Bytes(bytes) => found(&KEYS, &bytes.as_slice()),
        Key::Keypad(_) => None,
    }
}

fn found<T: PartialEq>(keys: &[(T, Press)], key: &T) -> Option<Press> {
    keys.iter()
        .find(|(bytes, _)| bytes == key)
        .map(|(_, press)| *press)
}

/// The open confirmation as a frame shows it: its row and offer now.
pub(crate) struct Asking<'a> {
    pub(crate) confirmation: &'a Confirmation,
    pub(crate) row: &'a View,
    pub(crate) offer: Offer,
}

impl Asking<'_> {
    pub(crate) fn render(&self, area: Rect, buffer: &mut Buffer) {
        self.confirmation.render(self.row, self.offer, area, buffer);
    }
}

/// After a confirmed Delete removed `id` from `before`, leaving `after`:
/// the row now at its place, else the one before it, else none (selector
/// document, "Delete: terminate and remove").
pub(crate) fn fallback(
    id: &SessionId,
    before: &[&SessionId],
    after: &[&SessionId],
) -> Option<SessionId> {
    let index = before.iter().position(|row| *row == id)?;
    let fallen = after.get(index).or(after.last());
    fallen.map(|row| (*row).clone())
}

fn length(cells: usize) -> Constraint {
    Constraint::Length(u16::try_from(cells).unwrap_or(u16::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mouse::Mouse, router::Pasted};
    use ratatui::style::Modifier;
    use std::time::SystemTime;
    use unicode_width::UnicodeWidthStr;

    fn id() -> SessionId {
        "00000000-0000-4000-8000-0000000000d1".parse().unwrap()
    }

    fn key(bytes: &[u8]) -> Routed {
        Routed::Key(Key::Bytes(bytes.to_vec()))
    }

    fn row(status: &str, label: &str) -> View {
        View {
            id: id(),
            status: status.to_owned(),
            label: label.to_owned(),
            since: SystemTime::UNIX_EPOCH,
        }
    }

    /// Each row of `buffer`, wide characters once, and the text of its
    /// reversed cells.
    fn drawn(buffer: &Buffer) -> (Vec<String>, String) {
        let area = buffer.area;
        let mut reversed = String::new();
        let lines = (area.y..area.bottom())
            .map(|y| {
                let mut line = String::new();
                let mut skip = 0;
                for x in area.x..area.right() {
                    let cell = &buffer[(x, y)];
                    if cell.modifier.contains(Modifier::REVERSED) {
                        reversed.push_str(cell.symbol());
                    }
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    skip = cell.symbol().width().saturating_sub(1);
                    line.push_str(cell.symbol());
                }
                line
            })
            .collect();
        (lines, reversed)
    }

    fn rendered(
        confirmation: &Confirmation,
        row: &View,
        offer: Offer,
        area: Rect,
    ) -> (Vec<String>, String) {
        let mut buffer = Buffer::empty(area);
        confirmation.render(row, offer, area, &mut buffer);
        drawn(&buffer)
    }

    /// DEL-01 (pure supplement, #117): only forward Delete opens the
    /// confirmation, never Backspace, DEL, a paste or another key.
    #[test]
    fn only_forward_delete_asks_for_a_confirmation() {
        assert!(requested(&key(b"\x1b[3~")));
        for other in [
            key(b"\x7f"),
            key(b"\x08"),
            key(b"\x1b[3;5~"),
            key(b"\x1b[2~"),
        ] {
            assert!(!requested(&other), "{other:?}");
        }
        let pasted = Routed::Paste(Pasted {
            begin: crate::router::Begin::Started,
            bytes: b"\x1b[3~".to_vec(),
            ends: true,
        });
        assert!(!requested(&pasted));
    }

    /// DEL-01 (pure supplement, owner decision on #117): Cancel has the
    /// focus first, so Enter cancels; Tab moves it either way, Right to the
    /// offer and Left back, each staying put at its end; Enter then
    /// confirms this session and Escape cancels wherever the focus is.
    /// Other keys, pastes, mouse reports and the global actions the monitor
    /// hands no dialog decide nothing and move nothing.
    #[test]
    fn cancel_has_the_focus_until_tab_or_the_arrows_move_it() {
        let enter = key(b"\r");
        let tab = key(b"\t");
        let right = Routed::Key(Key::Cursor(b'C'));
        let left = Routed::Key(Key::Cursor(b'D'));
        let mut confirmation = Confirmation::new(id());
        assert_eq!(confirmation.target(), &id());
        assert_eq!(confirmation.input(&enter), Some(Decision::Cancel));
        let inert = [
            key(b"y"),
            key(b" "),
            key(b"\x1b[3~"),
            Routed::Key(Key::Cursor(b'A')),
            Routed::Key(Key::Keypad(b'M')),
            Routed::Paste(Pasted {
                begin: crate::router::Begin::Started,
                bytes: b"\r".to_vec(),
                ends: true,
            }),
            Routed::Mouse(Mouse {
                code: 0,
                column: 1,
                row: 1,
                released: false,
            }),
            Routed::Unknown(0x1d, Key::Bytes(b"\r".to_vec())),
            Routed::Rest(Key::Bytes(b"\r".to_vec())),
            Routed::Focus(true),
        ];
        let focus = |confirmation: &mut Confirmation, moves: &[&Routed]| {
            for routed in moves {
                assert_eq!(confirmation.input(routed), None);
            }
            confirmation.confirming
        };
        assert!(!focus(&mut confirmation, &inert.iter().collect::<Vec<_>>()));
        assert!(focus(&mut confirmation, &[&tab]));
        assert!(!focus(&mut confirmation, &[&tab]));
        assert!(focus(&mut confirmation, &[&right, &right]));
        assert!(focus(&mut confirmation, &inert.iter().collect::<Vec<_>>()));
        assert_eq!(confirmation.input(&enter), Some(Decision::Confirm(id())));
        assert!(!focus(&mut confirmation, &[&left, &left]));
        assert!(focus(&mut confirmation, &[&right]));
        assert_eq!(
            confirmation.input(&Routed::Key(Key::Escape)),
            Some(Decision::Cancel)
        );
    }

    /// DEL-01 (pure supplement, #117): the box is titled `Delete session`
    /// and centred, showing the row's label and status, then Cancel,
    /// focused first, and the offer: Terminate and remove, or Remove; the
    /// focus is reversed. A label too long for the host ends in `…`.
    #[test]
    fn the_confirmation_shows_the_label_status_and_offer_with_cancel_focused() {
        let mut confirmation = Confirmation::new(id());
        let area = Rect::new(0, 0, 40, 8);
        let (lines, reversed) = rendered(
            &confirmation,
            &row("Running", "Build"),
            Offer::TerminateAndRemove,
            area,
        );
        assert_eq!(
            lines,
            [
                "",
                "┌ Delete session ──────────────────────┐",
                "│ Build                                │",
                "│ Running                              │",
                "│                                      │",
                "│ [ Cancel ]  [ Terminate and remove ] │",
                "└──────────────────────────────────────┘",
                "",
            ]
            .map(|line| format!("{line:<40}"))
        );
        assert_eq!(reversed, "[ Cancel ]");
        confirmation.input(&key(b"\t"));
        let row = row("Succeeded", "日本語のとても長いラベルです");
        let (lines, reversed) =
            rendered(&confirmation, &row, Offer::Remove, Rect::new(0, 0, 30, 8));
        assert_eq!(reversed, "[ Remove ]");
        assert_eq!(lines[2].trim_end(), " │ 日本語のとても長いラベル… │");
        assert_eq!(lines[5].trim_end(), " │ [ Cancel ]  [ Remove ]    │");
    }

    /// DEL-02 (pure supplement, selector document): a removed row's place
    /// goes to the row now there, else, when it was last, to the one before
    /// it, else to none; a row that was never there has no fallback.
    #[test]
    fn a_removed_row_falls_back_to_its_place_then_the_row_before() {
        let [a, b, c]: [SessionId; 3] = [1, 2, 3].map(|suffix| {
            format!("00000000-0000-4000-8000-0000000000{suffix:02x}")
                .parse()
                .unwrap()
        });
        assert_eq!(fallback(&b, &[&a, &b, &c], &[&a, &c]), Some(c.clone()));
        assert_eq!(fallback(&c, &[&a, &b, &c], &[&a, &b]), Some(b.clone()));
        assert_eq!(fallback(&a, &[&a], &[]), None);
        assert_eq!(fallback(&a, &[&b, &c], &[&b, &c]), None);
    }
}
