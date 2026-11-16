//! Logical notifications: complete OSC 9 and OSC 777 sequences, and OSC 99
//! chunks assembled under the session's notification assembly limits, made
//! inert UTF-8 within the release-constant text and record bounds. An
//! oversized notification is ignored, never truncated; overflow and expiry
//! log a diagnostic and complete nothing.
use super::notifications::{Chunk, Parsed, Part};
use climon2_core::{
    config::Assembly,
    wire::{ControlRecord, Counter, Notification, NotificationSource},
};
use std::time::Duration;

/// The longest notification title, in UTF-8 bytes.
pub const TITLE_BYTES: usize = 4_096;
/// The longest notification body, in UTF-8 bytes.
pub const BODY_BYTES: usize = 8_192;
/// The longest metadata record, its JSON body, in bytes.
pub const RECORD_BYTES: usize = 16_384;

/// A recognized logical notification, ready for its episode.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Notified {
    source: NotificationSource,
    source_id: Option<String>,
    title: Option<String>,
    body: Option<String>,
}

impl Notified {
    /// The notification these bytes make, replacing invalid UTF-8: none
    /// when title and body are both missing or empty, or when the title,
    /// the body or the record, with the longest episode identifier, would
    /// exceed its bound.
    fn new(
        source: NotificationSource,
        source_id: Option<&[u8]>,
        title: Option<&[u8]>,
        body: Option<&[u8]>,
    ) -> Option<Self> {
        let notified = Self {
            source,
            source_id: source_id.map(text),
            title: title.map(text),
            body: body.map(text),
        };
        let meaningful = !(blank(&notified.title) && blank(&notified.body));
        let within = within(&notified.title, TITLE_BYTES) && within(&notified.body, BODY_BYTES);
        (meaningful && within && notified.fits()).then_some(notified)
    }

    /// Whether its record fits with any episode.
    fn fits(&self) -> bool {
        let longest = Counter::new(u64::MAX).expect("the last counter");
        let record = self.clone().record(Some(longest));
        record.to_frame().body().len() <= RECORD_BYTES
    }

    /// Its record, carrying the episode its attention request left
    /// pending, if any.
    pub fn record(self, episode_id: Option<Counter>) -> Notification {
        Notification {
            source: self.source,
            source_id: self.source_id,
            title: self.title,
            body: self.body,
            episode_id,
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn blank(text: &Option<String>) -> bool {
    text.as_deref().is_none_or(str::is_empty)
}

fn within(text: &Option<String>, bytes: usize) -> bool {
    text.as_ref().is_none_or(|text| text.len() <= bytes)
}

/// The title and the body an OSC 99 notification assembled so far.
#[derive(Default)]
struct Text {
    title: Option<Vec<u8>>,
    body: Option<Vec<u8>>,
}

impl Text {
    /// The bytes it holds.
    fn held(&self) -> u64 {
        [&self.title, &self.body]
            .into_iter()
            .flatten()
            .map(|text| text.len() as u64)
            .sum()
    }

    /// The text `part` adds to and its bound; none for auxiliary data.
    fn slot(&mut self, part: Part) -> Option<(&mut Option<Vec<u8>>, usize)> {
        match part {
            Part::Title => Some((&mut self.title, TITLE_BYTES)),
            Part::Body => Some((&mut self.body, BODY_BYTES)),
            Part::Auxiliary => None,
        }
    }

    /// Adds `chunk`'s text when the result stays within its bound and the
    /// chunk within `budget`; false when it would not.
    fn add(&mut self, chunk: &Chunk<'_>, budget: u64) -> bool {
        let Some((slot, bound)) = self.slot(chunk.part) else {
            return true;
        };
        let length = slot.as_ref().map_or(0, Vec::len) + chunk.payload.len();
        let fits = length <= bound && chunk.payload.len() as u64 <= budget;
        if fits {
            slot.get_or_insert_default()
                .extend_from_slice(&chunk.payload);
        }
        fits
    }
}

/// One incomplete OSC 99 notification, from its first chunk.
struct Pending {
    id: Option<Box<[u8]>>,
    created: Duration,
    /// `None` once the notification was discarded: its later chunks are
    /// swallowed, and it completes nothing.
    text: Option<Text>,
}

impl Pending {
    fn new(id: Option<&[u8]>, created: Duration) -> Self {
        Self {
            id: id.map(Box::from),
            created,
            text: Some(Text::default()),
        }
    }

    fn held(&self) -> u64 {
        self.text.as_ref().map_or(0, Text::held)
    }

    /// Adds `chunk`'s text, or discards the notification when the text
    /// would pass its bound or, unless the chunk completes it, the `room`
    /// the other assemblies leave.
    fn add(&mut self, chunk: &Chunk<'_>, room: u64) {
        let Some(text) = &mut self.text else {
            return;
        };
        let budget = if chunk.done {
            u64::MAX
        } else {
            room.saturating_sub(text.held())
        };
        if !text.add(chunk, budget) {
            tracing::warn!("a notification assembly passed its bounds and was discarded");
            self.text = None;
        }
    }

    /// The notification it completes, unless it was discarded.
    fn complete(self) -> Option<Notified> {
        let Text { title, body } = self.text?;
        Notified::new(
            NotificationSource::Osc99,
            self.id.as_deref(),
            title.as_deref(),
            body.as_deref(),
        )
    }
}

/// The session's incomplete OSC 99 notifications, under its fixed limits.
pub struct Assembler {
    limits: Assembly,
    pending: Vec<Pending>,
}

impl Assembler {
    pub fn new(limits: Assembly) -> Self {
        Self {
            limits,
            pending: Vec::new(),
        }
    }

    /// The logical notification `parsed` completes at `now`, if any.
    pub fn notification(&mut self, now: Duration, parsed: Parsed<'_>) -> Option<Notified> {
        match parsed {
            Parsed::Complete {
                source,
                title,
                body,
            } => Notified::new(source, None, title, body),
            Parsed::Chunk(chunk) => self.chunk(now, &chunk),
        }
    }

    /// Ends every assembly whose fixed lifetime has passed at `now`,
    /// completing nothing.
    pub fn expire(&mut self, now: Duration) {
        let lifetime = Duration::from_millis(self.limits.lifetime_ms().into());
        self.pending
            .retain(|pending| now.saturating_sub(pending.created) < lifetime || expired());
    }

    /// Joins `chunk` to its notification's assembly, or starts one; a
    /// completing chunk ends it.
    fn chunk(&mut self, now: Duration, chunk: &Chunk<'_>) -> Option<Notified> {
        self.expire(now);
        let mut pending = self.take(chunk.id).or_else(|| self.admit(now, chunk))?;
        pending.add(chunk, self.room());
        if chunk.done {
            return pending.complete();
        }
        self.pending.push(pending);
        None
    }

    fn take(&mut self, id: Option<&[u8]>) -> Option<Pending> {
        let index = self
            .pending
            .iter()
            .position(|pending| pending.id.as_deref() == id)?;
        Some(self.pending.remove(index))
    }

    /// A new assembly for `chunk`: at once for a chunk that completes it,
    /// otherwise only while fewer assemblies than the limit are incomplete.
    fn admit(&self, now: Duration, chunk: &Chunk<'_>) -> Option<Pending> {
        let room = self.pending.len() < self.limits.records() as usize;
        (chunk.done || room)
            .then(|| Pending::new(chunk.id, now))
            .or_else(dropped)
    }

    /// The text bytes the other incomplete assemblies leave.
    fn room(&self) -> u64 {
        let held: u64 = self.pending.iter().map(Pending::held).sum();
        self.limits.bytes().saturating_sub(held)
    }
}

/// An assembly reached its lifetime: never kept.
fn expired() -> bool {
    tracing::warn!("a notification assembly expired incomplete");
    false
}

/// A chunk that would open one assembly too many: never assembled.
fn dropped() -> Option<Pending> {
    tracing::warn!("too many incomplete notifications; a chunk was dropped");
    None
}

#[cfg(test)]
mod tests {
    use super::{Assembler, Notified};
    use crate::status::notifications::{Chunk, Parsed, Part};
    use climon2_core::{
        config::Assembly,
        wire::{Counter, Notification, NotificationSource},
    };
    use std::{borrow::Cow, time::Duration};

    use NotificationSource::{Osc9, Osc99, Osc777};

    /// An assembler holding at most two assemblies of 40 text bytes
    /// together, each living one second.
    fn limited() -> Assembler {
        Assembler::new(Assembly::new(2, 40, 1_000).unwrap())
    }

    fn at(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn complete<'a>(
        source: NotificationSource,
        title: Option<&'a [u8]>,
        body: Option<&'a [u8]>,
    ) -> Parsed<'a> {
        Parsed::Complete {
            source,
            title,
            body,
        }
    }

    fn chunk<'a>(id: Option<&'a str>, done: bool, part: Part, payload: &'a [u8]) -> Parsed<'a> {
        Parsed::Chunk(Chunk {
            id: id.map(str::as_bytes),
            done,
            part,
            payload: Cow::Borrowed(payload),
        })
    }

    /// An incomplete title chunk of notification `id`.
    fn open<'a>(id: &'a str, payload: &'a [u8]) -> Parsed<'a> {
        chunk(Some(id), false, Part::Title, payload)
    }

    /// The empty title chunk that completes notification `id`.
    fn close(id: &str) -> Parsed<'_> {
        chunk(Some(id), true, Part::Title, b"")
    }

    fn notification(
        source: NotificationSource,
        source_id: Option<&str>,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Option<Notification> {
        Some(Notification {
            source,
            source_id: source_id.map(str::to_owned),
            title: title.map(str::to_owned),
            body: body.map(str::to_owned),
            episode_id: None,
        })
    }

    fn record(notified: Option<Notified>) -> Option<Notification> {
        notified.map(|notified| notified.record(None))
    }

    /// STA-06 (pure, complete sources): OSC 9 text is a body and OSC 777 a
    /// title with an optional body, inert UTF-8 with invalid bytes
    /// replaced; a title or a body alone suffices, but both missing or
    /// empty is no notification. The episode is the record's.
    #[test]
    fn complete_sources_become_inert_notifications() {
        let mut assembler = limited();
        let mut notified = |parsed| record(assembler.notification(at(0), parsed));
        let cases = [
            (
                complete(Osc9, None, Some(b"Built")),
                notification(Osc9, None, None, Some("Built")),
            ),
            (complete(Osc9, None, Some(b"")), None),
            (
                complete(Osc777, Some(b"T"), None),
                notification(Osc777, None, Some("T"), None),
            ),
            (
                complete(Osc777, Some(b""), Some(b"b;c")),
                notification(Osc777, None, Some(""), Some("b;c")),
            ),
            (complete(Osc777, Some(b""), Some(b"")), None),
            (complete(Osc777, Some(b""), None), None),
            (
                complete(Osc777, Some(b"a\xffb"), Some(b"\xe2\x82")),
                notification(Osc777, None, Some("a\u{fffd}b"), Some("\u{fffd}")),
            ),
        ];
        for (index, (parsed, expected)) in cases.into_iter().enumerate() {
            assert_eq!(notified(parsed), expected, "case {index}");
        }
        let episode = Counter::new(7).unwrap();
        let built = limited().notification(at(0), complete(Osc9, None, Some(b"x")));
        assert_eq!(
            built.unwrap().record(Some(episode)).episode_id,
            Some(episode)
        );
    }

    /// LIM-01 (pure, release-constant text bounds): a title of 4,096 UTF-8
    /// bytes and a body of 8,192 are kept, multi-byte characters filling
    /// them exactly; one byte more, or an invalid byte whose replacement
    /// passes the bound, ignores the notification instead of truncating it.
    #[test]
    fn titles_and_bodies_are_bounded_in_utf8_bytes() {
        let euros = |bytes: usize| "\u{20ac}".repeat(bytes / 3) + &"e".repeat(bytes % 3);
        let mut assembler = limited();
        let mut kept = |title: &str, body: &str| {
            let parsed = complete(Osc777, Some(title.as_bytes()), Some(body.as_bytes()));
            assembler.notification(at(0), parsed).is_some()
        };
        assert!(kept(&euros(4_096), &euros(8_192)));
        assert!(!kept(&euros(4_097), "b"));
        assert!(!kept("t", &euros(8_193)));
        let mut replaced = "t".repeat(4_093).into_bytes();
        replaced.push(0xff);
        let parsed = complete(Osc777, Some(&replaced), None);
        assert!(limited().notification(at(0), parsed).is_some());
        replaced.insert(0, b't');
        let parsed = complete(Osc777, Some(&replaced), None);
        assert_eq!(limited().notification(at(0), parsed), None);
    }

    /// LIM-01 (pure, release-constant text bounds in OSC 99): a title of
    /// 4,096 UTF-8 bytes and a body of 8,192, each in the one chunk that
    /// completes its notification, are kept whole; one byte more ignores
    /// the notification instead of truncating it.
    #[test]
    fn osc_99_text_is_bounded_from_its_first_chunk() {
        let euros = |bytes: usize| "\u{20ac}".repeat(bytes / 3) + &"e".repeat(bytes % 3);
        let mut assembler = limited();
        let mut fed = |part, text: &str| {
            let parsed = chunk(Some("w"), true, part, text.as_bytes());
            record(assembler.notification(at(0), parsed))
        };
        let (title, body) = (euros(4_096), euros(8_192));
        assert_eq!(
            fed(Part::Title, &title),
            notification(Osc99, Some("w"), Some(&title), None)
        );
        assert_eq!(
            fed(Part::Body, &body),
            notification(Osc99, Some("w"), None, Some(&body))
        );
        assert_eq!(fed(Part::Title, &euros(4_097)), None);
        assert_eq!(fed(Part::Body, &euros(8_193)), None);
    }

    /// LIM-01 (pure, metadata record bound): a notification is kept only
    /// when its record, with the longest episode identifier, fits 16,384
    /// bytes, here a body whose escaped quotes fill it exactly; one more
    /// byte ignores it.
    #[test]
    fn records_fit_the_metadata_record_bound() {
        let frame = r#"{"source":"osc9","source_id":null,"title":null,"body":"","episode_id":"18446744073709551615"}"#;
        let quotes = (16_384 - frame.len()) / 2;
        let odd = (16_384 - frame.len()) % 2;
        let body = "\"".repeat(quotes) + &"b".repeat(odd);
        let parsed = complete(Osc9, None, Some(body.as_bytes()));
        assert!(limited().notification(at(0), parsed).is_some());
        let body = body + "b";
        let parsed = complete(Osc9, None, Some(body.as_bytes()));
        assert_eq!(limited().notification(at(0), parsed), None);
    }

    /// STA-06 (pure, OSC 99 assembly): chunks of one identifier
    /// concatenate into its title and body until a completing chunk, a
    /// multi-byte character split between them included; other
    /// notifications, with or without an identifier, assemble beside it.
    /// A chunk that completes alone is a whole notification. Auxiliary
    /// data completes a notification that has text, but never makes one
    /// alone.
    #[test]
    fn osc_99_chunks_assemble_one_notification_per_identifier() {
        let mut assembler = limited();
        let mut fed = |parsed| record(assembler.notification(at(0), parsed));
        assert_eq!(fed(open("a", b"Tes")), None);
        assert_eq!(fed(chunk(None, false, Part::Body, b"anon")), None);
        assert_eq!(fed(chunk(Some("a"), false, Part::Body, b"\xe2\x82")), None);
        assert_eq!(fed(chunk(Some("a"), false, Part::Title, b"ts")), None);
        assert_eq!(
            fed(chunk(Some("a"), true, Part::Body, b"\xac!")),
            notification(Osc99, Some("a"), Some("Tests"), Some("\u{20ac}!"))
        );
        assert_eq!(
            fed(chunk(None, true, Part::Body, b"ymous")),
            notification(Osc99, None, None, Some("anonymous"))
        );
        assert_eq!(
            fed(chunk(Some("b"), true, Part::Title, b"Once")),
            notification(Osc99, Some("b"), Some("Once"), None)
        );
        assert_eq!(fed(chunk(Some("c"), true, Part::Auxiliary, b"icon")), None);
        assert_eq!(fed(open("d", b"Done")), None);
        assert_eq!(fed(chunk(Some("d"), false, Part::Auxiliary, b"icon")), None);
        assert_eq!(
            fed(chunk(Some("d"), true, Part::Auxiliary, b"")),
            notification(Osc99, Some("d"), Some("Done"), None)
        );
        assert_eq!(fed(open("e", b"")), None);
        assert_eq!(fed(close("e")), None);
    }

    /// LIM-01 (pure, assembly records at the limit and one past): two
    /// notifications assemble at once; a third one's chunk is dropped, so
    /// it completes nothing, while the two complete; a notification that
    /// completes in one chunk needs no assembly.
    #[test]
    fn incomplete_assemblies_are_bounded_in_number() {
        let mut assembler = limited();
        let mut fed = |parsed| record(assembler.notification(at(0), parsed));
        assert_eq!(fed(open("a", b"A")), None);
        assert_eq!(fed(open("b", b"B")), None);
        assert_eq!(fed(open("c", b"C")), None);
        assert_eq!(
            fed(chunk(Some("x"), true, Part::Title, b"X")),
            notification(Osc99, Some("x"), Some("X"), None)
        );
        assert_eq!(fed(close("c")), None);
        assert_eq!(
            fed(close("a")),
            notification(Osc99, Some("a"), Some("A"), None)
        );
        assert_eq!(
            fed(close("b")),
            notification(Osc99, Some("b"), Some("B"), None)
        );
    }

    /// LIM-01 (pure, assembly bytes at the limit and one past): incomplete
    /// assemblies hold 40 text bytes together; a chunk that would hold one
    /// more discards its notification, whose later chunks complete nothing
    /// and whose bytes are freed. A completing chunk is not held, so it
    /// may pass the budget.
    #[test]
    fn incomplete_assemblies_are_bounded_in_bytes() {
        let mut assembler = limited();
        let mut fed = |parsed| record(assembler.notification(at(0), parsed));
        assert_eq!(fed(open("a", &[b'a'; 30])), None);
        assert_eq!(fed(open("b", &[b'b'; 11])), None);
        assert_eq!(fed(chunk(Some("b"), true, Part::Body, b"late")), None);
        assert_eq!(fed(open("b", &[b'b'; 10])), None);
        let long = "c".repeat(50);
        assert_eq!(
            fed(chunk(Some("c"), true, Part::Body, long.as_bytes())),
            notification(Osc99, Some("c"), None, Some(&long))
        );
        assert_eq!(
            fed(close("a")),
            notification(Osc99, Some("a"), Some(&"a".repeat(30)), None)
        );
        assert_eq!(
            fed(close("b")),
            notification(Osc99, Some("b"), Some(&"b".repeat(10)), None)
        );
        assert_eq!(fed(open("d", &[b'd'; 40])), None);
        assert_eq!(fed(open("e", b"")), None);
        assert_eq!(fed(open("e", b"e")), None);
        assert_eq!(fed(close("e")), None);
        assert_eq!(
            fed(close("d")),
            notification(Osc99, Some("d"), Some(&"d".repeat(40)), None)
        );
    }

    /// LIM-01 (pure, assembly bytes): a discarded notification holds no
    /// bytes, so the others may use the whole budget.
    #[test]
    fn discarded_assemblies_hold_no_bytes() {
        let mut assembler = limited();
        let mut fed = |parsed| record(assembler.notification(at(0), parsed));
        assert_eq!(fed(open("a", &[b'a'; 41])), None);
        assert_eq!(fed(open("b", &[b'b'; 40])), None);
        assert_eq!(fed(close("a")), None);
        assert_eq!(
            fed(close("b")),
            notification(Osc99, Some("b"), Some(&"b".repeat(40)), None)
        );
    }

    /// LIM-01 (pure, assembly lifetime at the limit and one past): an
    /// assembly lives one second from its first chunk, however many chunks
    /// follow: completed at 999 ms it is a notification, while at 1,000 ms
    /// it has expired and the completing chunk alone is empty. Until it
    /// expires it keeps its place, which expiry then frees.
    #[test]
    fn assemblies_expire_a_fixed_lifetime_after_creation() {
        let mut assembler = limited();
        assert_eq!(assembler.notification(at(0), open("a", b"A")), None);
        assert_eq!(assembler.notification(at(500), open("b", b"B")), None);
        assert_eq!(assembler.notification(at(998), open("a", b"a")), None);
        assert_eq!(
            record(assembler.notification(at(999), close("a"))),
            notification(Osc99, Some("a"), Some("Aa"), None)
        );
        assert_eq!(assembler.notification(at(1_000), open("c", b"C")), None);
        assembler.expire(at(1_499));
        assert_eq!(assembler.notification(at(1_499), open("d", b"D")), None);
        assembler.expire(at(1_500));
        assert_eq!(assembler.notification(at(1_500), open("e", b"E")), None);
        assert_eq!(assembler.notification(at(1_500), close("b")), None);
        assert_eq!(
            record(assembler.notification(at(1_500), close("c"))),
            notification(Osc99, Some("c"), Some("C"), None)
        );
        assert_eq!(assembler.notification(at(1_500), close("d")), None);
        assert_eq!(
            record(assembler.notification(at(2_499), close("e"))),
            notification(Osc99, Some("e"), Some("E"), None)
        );
    }
}
