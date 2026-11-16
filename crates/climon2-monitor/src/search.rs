//! The selector's search (selector document, "Selector"; #113): one line of
//! text, always focused, that typing, Backspace and Left, Right, Home and
//! End edit, and that a bracketed paste fills as one line. It holds at most
//! `ui.search_bytes` UTF-8 bytes: an edit that would exceed them is refused
//! whole. A row matches when every search character occurs, in order and
//! ignoring case, in its displayed status, a space and its label, as
//! `fuzzy-matcher`'s `SkimMatcherV2` finds them; scores are never used.
use crate::registry::{View, clip};
use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use unicode_width::UnicodeWidthChar;

/// What an edit did to the search text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Edit {
    Changed,
    Unchanged,
    /// Refused: the text would exceed its limit.
    Refused,
}

pub(crate) struct Search {
    text: String,
    /// The cursor's byte offset in `text`, on a character boundary.
    cursor: usize,
    /// `ui.search_bytes`.
    limit: usize,
    /// The bytes typed so far of a character not yet complete.
    typed: Utf8,
    /// The bracketed paste under way, if any.
    pasting: Option<Pasting>,
    matcher: SkimMatcherV2,
}

impl Search {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            limit,
            typed: Utf8::default(),
            pasting: None,
            matcher: SkimMatcherV2::default(),
        }
    }

    /// Empties the search, as the selector opens.
    pub(crate) fn clear(&mut self) {
        *self = Self::new(self.limit);
    }

    /// `ui.search_bytes`.
    pub(crate) fn limit(&self) -> usize {
        self.limit
    }

    /// One typed byte: a character is inserted once its UTF-8 bytes are
    /// complete, unless it is a control character; invalid bytes are
    /// discarded.
    pub(crate) fn typed(&mut self, byte: u8) -> Edit {
        match self.typed.push(byte) {
            Some(character) if !character.is_control() => {
                self.insert(character.encode_utf8(&mut [0; 4]))
            }
            _ => Edit::Unchanged,
        }
    }

    /// Inserts `text` at the cursor, or refuses all of it if the search
    /// would then exceed its limit.
    fn insert(&mut self, text: &str) -> Edit {
        if self.text.len() + text.len() > self.limit {
            return Edit::Refused;
        }
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        [Edit::Unchanged, Edit::Changed][usize::from(!text.is_empty())]
    }

    /// Deletes the character before the cursor.
    pub(crate) fn backspace(&mut self) -> Edit {
        let Some(previous) = self.previous() else {
            return Edit::Unchanged;
        };
        self.text.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        Edit::Changed
    }

    pub(crate) fn left(&mut self) -> Edit {
        self.cursor = self.previous().unwrap_or(self.cursor);
        Edit::Unchanged
    }

    pub(crate) fn right(&mut self) -> Edit {
        let next = self.text[self.cursor..].chars().next();
        self.cursor += next.map_or(0, char::len_utf8);
        Edit::Unchanged
    }

    pub(crate) fn home(&mut self) -> Edit {
        self.cursor = 0;
        Edit::Unchanged
    }

    pub(crate) fn end(&mut self) -> Edit {
        self.cursor = self.text.len();
        Edit::Unchanged
    }

    /// Where the character before the cursor starts, if there is one.
    fn previous(&self) -> Option<usize> {
        self.text[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
    }

    /// A bracketed paste begins.
    pub(crate) fn paste_start(&mut self) {
        self.pasting = Some(Pasting::default());
    }

    /// Bytes of the paste under way, if one is.
    pub(crate) fn paste_data(&mut self, bytes: &[u8]) {
        let limit = self.limit;
        if let Some(pasting) = &mut self.pasting {
            for &byte in bytes {
                pasting.push(byte, limit);
            }
        }
    }

    /// The paste ends: its one line is inserted whole, or refused.
    pub(crate) fn paste_end(&mut self) -> Edit {
        let pasted = self.pasting.take().unwrap_or_default();
        self.insert(&pasted.line)
    }

    /// Whether `row` matches: every search character in order, ignoring
    /// case, in its displayed status, a space, then its label.
    pub(crate) fn matches(&self, row: &View) -> bool {
        let choice = format!("{} {}", row.status, row.label).to_lowercase();
        let pattern = self.text.to_lowercase();
        self.matcher.fuzzy_match(&choice, &pattern).is_some()
    }

    /// The part of the text shown in `cells` display cells, and the
    /// cursor's cell in it: the text from its start while the cursor fits,
    /// otherwise from where the cursor is the last cell.
    pub(crate) fn shown(&self, cells: usize) -> (&str, usize) {
        let room = cells.saturating_sub(1);
        let mut start = self.cursor;
        let mut used = 0;
        for (index, character) in self.text[..self.cursor].char_indices().rev() {
            let width = character.width().unwrap_or_default();
            if used + width > room {
                break;
            }
            used += width;
            start = index;
        }
        (clip(&self.text[start..], cells), used)
    }
}

/// Bytes decoded as UTF-8, one character at a time.
#[derive(Default)]
struct Utf8(Vec<u8>);

impl Utf8 {
    /// The character `byte` completes, if any. Bytes that cannot begin or
    /// continue a character are discarded with those before them.
    fn push(&mut self, byte: u8) -> Option<char> {
        self.0.push(byte);
        match std::str::from_utf8(&self.0) {
            Ok(text) => {
                let character = text.chars().next();
                self.0.clear();
                character
            }
            Err(error) => {
                if error.error_len().is_some() {
                    self.0.clear();
                }
                None
            }
        }
    }
}

/// A bracketed paste as one line of search text (selector document,
/// "Selector"): each line break (CR, LF or CRLF) and tab becomes a space,
/// other control characters and invalid UTF-8 are discarded. Once the line
/// exceeds the search's limit it stops growing: such a paste is refused.
#[derive(Default)]
struct Pasting {
    bytes: Utf8,
    line: String,
    /// The character before was a CR, so an LF completes its line break.
    carriage: bool,
}

impl Pasting {
    fn push(&mut self, byte: u8, limit: usize) {
        if let Some(character) = self.bytes.push(byte) {
            self.character(character, limit);
        }
    }

    fn character(&mut self, character: char, limit: usize) {
        let carriage = std::mem::replace(&mut self.carriage, character == '\r');
        let kept = in_line(character, carriage);
        if self.line.len() <= limit {
            self.line.extend(kept);
        }
    }
}

/// What a pasted character puts in the line: an LF completing a CRLF
/// nothing, a line break or tab a space, any other control character
/// nothing.
fn in_line(character: char, after_carriage_return: bool) -> Option<char> {
    match character {
        '\n' if after_carriage_return => None,
        '\r' | '\n' | '\t' => Some(' '),
        _ => Some(character).filter(|character| !character.is_control()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn row(status: &str, label: &str) -> View {
        View {
            id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            status: status.to_owned(),
            label: label.to_owned(),
            since: SystemTime::UNIX_EPOCH,
        }
    }

    fn typed(search: &mut Search, text: &[u8]) -> Vec<Edit> {
        text.iter().map(|byte| search.typed(*byte)).collect()
    }

    fn searching(text: &str) -> Search {
        let mut search = Search::new(4096);
        typed(&mut search, text.as_bytes());
        search
    }

    fn pasted(search: &mut Search, chunks: &[&[u8]]) -> Edit {
        search.paste_start();
        for chunk in chunks {
            search.paste_data(chunk);
        }
        search.paste_end()
    }

    /// SEL-02 (Contract, pure supplement; selector document's examples):
    /// ordered-subsequence matching over the displayed status, a space and
    /// the label, ignoring case: exact substrings, subsequences, spaced
    /// queries and matches across the status and label, but never reordered,
    /// substituted or missing characters, with no typo or short-query rule.
    #[test]
    fn search_matches_ordered_subsequences_of_status_and_label() {
        let cases = [
            ("bld", "Idle", "Build", true),
            ("blr", "Idle", "Build Replica", true),
            ("Fo Br", "Idle", "foo bar", true),
            ("rgb", "Running", "Build", true),
            ("ild", "Running", "Build", true),
            ("le bu", "Idle", "Build", true),
            ("failed (exit 143) b", "Failed (exit 143)", "Build", true),
            ("BUILD", "Idle", "build", true),
            ("", "Idle", "Build", true),
            ("b", "Idle", "Build", true),
            ("x", "Idle", "Build", false),
            ("dib", "Idle", "Build", false),
            ("biuld", "Idle", "Build", false),
            ("buildd", "Idle", "Build", false),
            ("bulid", "Idle", "Build", false),
            ("foobar", "Idle", "foo-baz", false),
            ("ib", "Idle", "", false),
            ("  ", "Idle", "ab", false),
        ];
        for (query, status, label, expected) in cases {
            let search = searching(query);
            assert_eq!(
                search.matches(&row(status, label)),
                expected,
                "{query:?} in {status:?} {label:?}"
            );
        }
    }

    /// SEL-02 (Contract, pure supplement, Unicode): case is ignored beyond
    /// ASCII, and wide and combining characters match as themselves.
    #[test]
    fn search_ignores_unicode_case_and_matches_wide_text() {
        let cases = [
            ("ÉTÉ", "Idle", "été", true),
            ("été", "Idle", "ÉTÉ", true),
            ("日語", "Idle", "日本語", true),
            ("語日", "Idle", "日本語", false),
            ("e\u{301}", "Idle", "cafe\u{301}", true),
            ("ß", "Idle", "STRASSE", false),
        ];
        for (query, status, label, expected) in cases {
            assert_eq!(
                searching(query).matches(&row(status, label)),
                expected,
                "{query:?} in {label:?}"
            );
        }
    }

    /// SEL-02 (pure supplement, editing): typed characters insert at the
    /// cursor, Backspace deletes before it, and Left, Right, Home and End
    /// move it by character, stopping at either end; multi-byte characters
    /// typed byte by byte insert once complete.
    #[test]
    fn typing_backspace_and_cursor_keys_edit_at_the_cursor() {
        let mut search = Search::new(4096);
        assert_eq!(
            typed(&mut search, "aé".as_bytes()),
            [Edit::Changed, Edit::Unchanged, Edit::Changed]
        );
        assert_eq!(search.left(), Edit::Unchanged);
        assert_eq!(search.typed(b'x'), Edit::Changed);
        assert_eq!(search.shown(10), ("axé", 2));
        search.home();
        search.left();
        assert_eq!(search.shown(10), ("axé", 0));
        assert_eq!(search.backspace(), Edit::Unchanged);
        search.right();
        search.right();
        assert_eq!(search.shown(10), ("axé", 2));
        assert_eq!(search.backspace(), Edit::Changed);
        assert_eq!(search.shown(10), ("aé", 1));
        search.end();
        search.right();
        assert_eq!(search.shown(10), ("aé", 2));
        assert_eq!(search.backspace(), Edit::Changed);
        assert_eq!(search.shown(10), ("a", 1));
        search.clear();
        assert_eq!(search.shown(10), ("", 0));
    }

    /// SEL-02 (pure supplement): Tab and every other control character
    /// typed, and invalid UTF-8, insert nothing; a byte that breaks a
    /// character discards it.
    #[test]
    fn control_characters_and_invalid_bytes_insert_nothing() {
        let mut search = Search::new(4096);
        let edits = typed(&mut search, b"\t\x00\x1f\x7f\xff\xc3a\xc2\x85\xe6\x97");
        assert!(
            edits.iter().all(|edit| *edit == Edit::Unchanged),
            "{edits:?}"
        );
        assert_eq!(search.shown(10), ("", 0));
        assert_eq!(typed(&mut search, b"\xa5")[0], Edit::Changed);
        assert_eq!(search.shown(10), ("日", 2));
    }

    /// Operational limits (`ui.search_bytes`, pure supplement): the search
    /// holds exactly its limit in UTF-8 bytes; a typed character that would
    /// pass it, one byte or several, is refused whole and changes nothing.
    #[test]
    fn typing_past_the_search_limit_is_refused() {
        let mut search = Search::new(4096);
        assert!(
            typed(&mut search, &[b'a'; 4094])
                .iter()
                .all(|edit| *edit == Edit::Changed)
        );
        assert_eq!(typed(&mut search, "日".as_bytes())[2], Edit::Refused);
        assert_eq!(typed(&mut search, "é".as_bytes())[1], Edit::Changed);
        assert_eq!(search.typed(b'b'), Edit::Refused);
        assert_eq!(search.text.len(), 4096);
        assert!(search.text.ends_with("aé"));
        assert_eq!(search.limit(), 4096);
    }

    /// SEL-02 (paste normalization, pure supplement): a paste is one line:
    /// CR, LF, CRLF and tabs become one space each, other control
    /// characters and invalid bytes are discarded, wherever its chunks
    /// split; it inserts at the cursor; it is never a key.
    #[test]
    fn a_paste_is_inserted_as_one_line() {
        let mut search = searching("<>");
        search.left();
        let edit = pasted(
            &mut search,
            &[
                b"a\r",
                b"\nb\rc\nd\te\x1b[1m\x07\xff",
                "é".as_bytes(),
                b"\x7f\r\r\n\xc2",
                b"\x85f",
            ],
        );
        assert_eq!(edit, Edit::Changed);
        assert_eq!(search.shown(40), ("<a b c d e[1mé  f>", 17));
        assert_eq!(pasted(&mut search, &[b"\x00\x1b"]), Edit::Unchanged);
        search.paste_data(b"x");
        assert_eq!(search.paste_end(), Edit::Unchanged);
        assert_eq!(search.shown(40), ("<a b c d e[1mé  f>", 17));
    }

    /// Operational limits (`ui.search_bytes`, paste, pure supplement): a
    /// paste that would take the search past its limit, by one byte or by
    /// far, is refused whole; one that reaches it exactly is inserted.
    #[test]
    fn a_paste_past_the_search_limit_is_refused_whole() {
        let mut search = searching("ab");
        assert_eq!(pasted(&mut search, &[&[b'x'; 4095]]), Edit::Refused);
        assert_eq!(
            pasted(&mut search, &[&[b'x'; 2000], &[b'x'; 9000]]),
            Edit::Refused
        );
        assert_eq!(search.text, "ab");
        let mut exact = vec![b'y'; 4091];
        exact.extend_from_slice("é\r\n".as_bytes());
        assert_eq!(pasted(&mut search, &[&exact]), Edit::Changed);
        assert_eq!(search.text.len(), 4096);
        assert!(search.text.ends_with("yé "));
        let mut empty = Search::new(4096);
        assert_eq!(pasted(&mut empty, &[&[b'z'; 4097]]), Edit::Refused);
        assert_eq!(pasted(&mut empty, &[&[b'z'; 4096]]), Edit::Changed);
    }

    /// SEL-01 (search line, pure supplement): the search shows from its
    /// start while the cursor fits, otherwise ending with the cursor in the
    /// last cell, never splitting a wide character.
    #[test]
    fn the_search_line_keeps_the_cursor_in_view() {
        let search = searching("abc日本");
        assert_eq!(search.shown(8), ("abc日本", 7));
        assert_eq!(search.shown(7), ("bc日本", 6));
        assert_eq!(search.shown(6), ("c日本", 5));
        assert_eq!(search.shown(5), ("日本", 4));
        assert_eq!(search.shown(4), ("本", 2));
        assert_eq!(search.shown(1), ("", 0));
        assert_eq!(search.shown(0), ("", 0));
        let mut search = searching("abcdef");
        search.home();
        search.right();
        assert_eq!(search.shown(3), ("abc", 1));
        let mut search = searching("a日");
        search.home();
        assert_eq!(search.shown(2), ("a", 0));
    }
}
