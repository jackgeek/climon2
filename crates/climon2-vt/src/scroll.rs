//! When a monitor model's normal screen could first scroll rows into
//! history, so that its first history chunk is admitted only then (#166),
//! yet still before the engine allocates the row it pushes. Every answer is
//! an upper bound: a token said to scroll may not, but one said not to never
//! does.

use crate::classify::Scrolling;

/// The engine's scroll region, which it keeps private, mirrored from the
/// sequences that set it: whether it starts at the top row, which alone
/// scrolls rows into history, and the row below its bottom, counted from 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Region {
    top: bool,
    end: usize,
}

/// Where the engine's cursor is on the active screen, counted from 0. A
/// wrap is pending only in the last column.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Cursor {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

impl Region {
    /// The whole screen, as a new or fully reset engine has it.
    pub(crate) fn full(rows: usize) -> Self {
        Self {
            top: true,
            end: rows,
        }
    }

    /// DECSTBM as the engine applies it: ignored unless the 1-based `top`
    /// is above `bottom` (by default the last row); both are clamped to the
    /// screen.
    pub(crate) fn set(self, top: usize, bottom: Option<usize>, rows: usize) -> Self {
        let bottom = bottom.unwrap_or(rows);
        if top >= bottom {
            return self;
        }
        Self {
            top: top == 1,
            end: bottom.min(rows),
        }
    }

    /// Whether line feeds and wraps from `line` can push rows into history:
    /// only a region that starts at the top row pushes them, and only from
    /// within it.
    fn within(self, line: usize) -> bool {
        self.top && line < self.end
    }

    /// Whether `moves` rows down from `line` could scroll rows into history,
    /// leaving the region's bottom row.
    fn reached(self, line: usize, moves: usize) -> bool {
        self.within(line) && line + moves >= self.end
    }

    /// Whether a control sequence doing `scrolling`, with `feeds` line feeds
    /// or repeated characters in it, could push rows into history from
    /// `line`. Erasing the normal screen scrolls its rows into history
    /// whatever the region; scrolling up does so for a region at the top
    /// row, and deleting lines when the cursor is on that row.
    pub(crate) fn could_scroll(self, line: usize, scrolling: Scrolling, feeds: usize) -> bool {
        let anywhere = match scrolling {
            Scrolling::ClearScreen => true,
            Scrolling::Up => self.top,
            Scrolling::DeleteLines => self.top && line == 0,
            _ => false,
        };
        let feeds = feeds + usize::from(scrolling == Scrolling::LineFeed);
        anywhere || self.reached(line, feeds)
    }

    /// Whether text could push rows into history at `cursor`, on a screen
    /// `cols` wide, when it is one unit vte acts on and on the region's
    /// bottom row ([`Region::piece`]): a line feed, or anything but a
    /// control printed in the last column, where a wrap can be pending.
    pub(crate) fn text_scrolls(self, text: &[u8], cursor: Cursor, cols: usize) -> bool {
        let feeds = matches!(text, [0x0a..=0x0c]);
        let control = matches!(text, [0x00..=0x1f]);
        let wraps = !control && cursor.column + 1 == cols;
        self.reached(cursor.line, 1) && (feeds || wraps)
    }

    /// The bytes of `text` to feed next from `line`: as many units as cannot
    /// reach the region's bottom row, since each moves the cursor down at
    /// most one row, or one unit on that row. Text that cannot push rows
    /// into history from there is fed whole.
    pub(crate) fn piece(self, text: &[u8], line: usize) -> usize {
        if !self.within(line) {
            return text.len();
        }
        let room = self.end - 1 - line;
        units(text).take(room.max(1)).map(<[u8]>::len).sum()
    }
}

/// Each unit vte acts on in `text`: a character, or an invalid sequence,
/// which prints one replacement character or executes one C1 control.
fn units(text: &[u8]) -> impl Iterator<Item = &[u8]> {
    text.utf8_chunks().flat_map(|chunk| {
        let valid = chunk.valid();
        let characters = valid
            .char_indices()
            .map(move |(at, c)| &valid.as_bytes()[at..at + c.len_utf8()]);
        let invalid = Some(chunk.invalid()).filter(|invalid| !invalid.is_empty());
        characters.chain(invalid)
    })
}
