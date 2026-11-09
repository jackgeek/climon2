//! The monitor model's allocation bound (LIM-03, Contract and Fault cases),
//! observed through the public interface and checked against an independent
//! oracle: this test binary's global allocator counts every byte each test
//! thread holds, so a model that grew past its limit, or charged less than it
//! held, fails here whatever the model believes about itself.

use climon2_vt::{Geometry, Limits, Model, Rejected};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Counting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
    static PEAK: Cell<isize> = const { Cell::new(0) };
}

fn record(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get().wrapping_add(delta);
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
}

// SAFETY: every call is forwarded unchanged to the system allocator; the
// counters only observe sizes.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record(layout.size().cast_signed());
        // SAFETY: forwarded with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        record(layout.size().cast_signed().wrapping_neg());
        // SAFETY: forwarded with the caller's pointer and layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// Bytes this thread allocated and has not freed since `watch` started, and
/// the most it held at once.
struct Meter {
    start: isize,
}

impl Meter {
    fn watch() -> Self {
        let start = LIVE.with(Cell::get);
        PEAK.with(|peak| peak.set(start));
        Self { start }
    }

    fn live(&self) -> usize {
        (LIVE.with(Cell::get) - self.start).try_into().unwrap()
    }

    fn peak(&self) -> usize {
        (PEAK.with(Cell::get) - self.start).try_into().unwrap()
    }
}

fn geometry(cols: u16, rows: u16) -> Geometry {
    Geometry::for_host(cols, rows).unwrap()
}

fn limits(bytes: usize, history_rows: usize) -> Limits {
    Limits {
        bytes,
        history_rows,
    }
}

/// The bytes a model reports it needs when its limit is too small.
#[track_caller]
fn needed(result: Result<impl Sized, Rejected>) -> usize {
    match result {
        Ok(_) => panic!("expected a rejection"),
        Err(rejected) => rejected.needed,
    }
}

/// LIM-03 Contract: a model is admitted exactly when its limit covers the
/// mandatory model (both screens, the protected newest page and the engine),
/// and the bytes it charges cover everything construction allocated.
#[test]
fn a_model_is_admitted_at_its_mandatory_bytes_and_rejected_one_below() {
    let at = geometry(80, 24);
    let mandatory = needed(Model::new(at, limits(0, 10_000)));

    assert_eq!(
        Model::new(at, limits(mandatory - 1, 10_000)).err(),
        Some(Rejected {
            needed: mandatory,
            limit: mandatory - 1
        })
    );
    let meter = Meter::watch();
    let model = Model::new(at, limits(mandatory, 10_000)).unwrap();
    assert!(meter.peak() <= model.footprint());
    assert_eq!(model.footprint(), mandatory);
}

const ROOMY: usize = 64 * 1024 * 1024;

/// LIM-03 Contract: plain text fills cells the engine already holds, so it is
/// accepted unchanged without charging anything more.
#[test]
fn plain_text_fills_held_cells_without_growth() {
    let meter = Meter::watch();
    let mut model = Model::new(geometry(10, 3), limits(ROOMY, 100)).unwrap();
    let footprint = model.footprint();

    model.feed(b"hello\r\nworld\r\n\xe4\xb8\x80x").unwrap();

    assert!(meter.peak() <= footprint);
    assert_eq!(model.footprint(), footprint);
    assert_eq!(model.screen(), ["hello", "world", "\u{4e00}x"]);
}

const ROWS: usize = 100;

/// The bytes a model needs before any output.
fn mandatory(at: Geometry) -> usize {
    needed(Model::new(at, limits(0, ROWS)))
}

/// A model with the smallest limit that accepts `bytes`.
fn fitted(at: Geometry, bytes: &[u8]) -> Model {
    let mut limit = mandatory(at);
    loop {
        let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
        match model.feed(bytes) {
            Ok(()) => return model,
            Err(rejected) => limit = rejected.needed,
        }
    }
}

/// Feed `before`, then `update`, into models whose limits sit exactly at the
/// bytes `update` needs and one byte below. At the bound the update is
/// accepted and the allocator never saw more than the limit; one byte below it
/// is rejected before it reaches the engine, leaving the page `before`
/// produced. Returns the accepted model's screen.
#[track_caller]
fn assert_bound(at: Geometry, before: &[u8], update: &[u8]) -> Vec<String> {
    let bound = needed(fitted(at, before).feed(update));

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, ROWS)).unwrap();
    model.feed(before).unwrap();
    model.feed(update).unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    let accepted = model.screen();

    let mut model = Model::new(at, limits(bound - 1, ROWS)).unwrap();
    model.feed(before).unwrap();
    let page = model.screen();
    assert_eq!(
        model.feed(update),
        Err(Rejected {
            needed: bound,
            limit: bound - 1
        })
    );
    assert_eq!(model.screen(), page);
    accepted
}

/// LIM-03 Contract: combining marks grow a cell's extra data. At the bound
/// every mark is kept; one byte past it the update is rejected before growth
/// and the page is intact.
#[test]
fn combining_marks_are_accepted_at_the_bound_and_rejected_one_byte_past_it() {
    let marks = "\u{301}".repeat(31);
    let accepted = assert_bound(geometry(10, 3), b"page\r\n", format!("a{marks}").as_bytes());
    assert_eq!(
        accepted,
        ["page".to_owned(), format!("a{marks}"), String::new()]
    );
}

/// LIM-03 Contract: cells printed under an underline colour (SGR 58) share
/// the cursor template's extra data, and each can come to hold its own copy.
/// At the bound they are accepted with their characters; one byte past it,
/// rejected before growth with the page intact.
#[test]
fn underline_coloured_cells_are_accepted_at_the_bound_and_rejected_one_byte_past_it() {
    let accepted = assert_bound(geometry(10, 3), b"page\r\n\x1b[58;5;1m", b"under");
    assert_eq!(accepted, ["page", "under", ""]);
}

/// LIM-03 Contract: a wide character writes up to three cells (a leading
/// spacer where it does not fit, itself and its spacer), and under an
/// underline colour each can hold extra data.
#[test]
fn wide_characters_are_accepted_at_the_bound_and_rejected_one_byte_past_it() {
    let accepted = assert_bound(
        geometry(5, 3),
        b"page\r\nabcd\x1b[58;5;1m",
        "\u{4e00}\u{4e8c}".as_bytes(),
    );
    assert_eq!(accepted, ["page", "abcd", "\u{4e00}\u{4e8c}"]);
}

/// LIM-03 Contract: plain text grows only history storage, which the engine
/// adds a chunk of rows at a time. At the bound the next chunk is admitted
/// and every row kept; one byte below it the chunk is refused before it is
/// allocated and the oldest row is discarded instead, while the update itself
/// is accepted with the newest page and every character intact.
#[test]
fn history_grows_at_the_bound_and_discards_its_oldest_row_one_byte_past_it() {
    let at = geometry(6, 2);
    let page = b"row0\r\nrow1\r\nrow2\r\nrow3";
    let mut roomy = Model::new(at, limits(ROOMY, ROWS)).unwrap();
    roomy.feed(page).unwrap();
    assert_eq!(roomy.history(), ["row0", "row1"]);
    let before = roomy.footprint();
    roomy.feed(b"\r\nrow4").unwrap();
    let bound = roomy.footprint();
    assert!(bound > before);

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, ROWS)).unwrap();
    model.feed(page).unwrap();
    model.feed(b"\r\nrow4").unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.history(), ["row0", "row1", "row2"]);
    assert_eq!(model.screen(), ["row3", "row4"]);

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound - 1, ROWS)).unwrap();
    model.feed(page).unwrap();
    model.feed(b"\r\nrow4").unwrap();
    assert!(meter.peak() < bound, "{} >= {bound}", meter.peak());
    assert_eq!(model.footprint(), before);
    assert_eq!(model.history(), ["row1", "row2"]);
    assert_eq!(model.screen(), ["row3", "row4"]);
}

/// LIM-03 Contract: erased screens and cleared history release their marks.
/// A model whose running charge leaves no room measures what its cells hold
/// before it rejects anything, so released marks make room for new ones.
#[test]
fn released_marks_make_room_before_anything_is_rejected() {
    let at = geometry(10, 3);
    let update = format!("a{}", "\u{301}".repeat(31));
    let bound = needed(fitted(at, b"").feed(update.as_bytes()));
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, ROWS)).unwrap();

    for _ in 0..100 {
        model.feed(b"\x1b[2J\x1b[3J\x1b[H").unwrap();
        model.feed(update.as_bytes()).unwrap();
    }

    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.footprint(), bound);
    assert_eq!(model.screen(), [update.as_str(), "", ""]);
}

/// LIM-03 Fault: `CSI 65535 b` makes vte print its preceding character, here
/// a combining mark, 65535 more times onto one cell from a nine-byte
/// sequence. It is charged before the engine sees it: at the bound every mark
/// is kept, one byte past it the sequence is rejected with the page intact.
#[test]
fn repeated_marks_are_accepted_at_the_bound_and_rejected_one_byte_past_it() {
    let accepted = assert_bound(
        geometry(10, 3),
        "page\r\na\u{301}".as_bytes(),
        b"\x1b[65535b",
    );
    assert_eq!(
        accepted,
        [
            "page".to_owned(),
            format!("a{}", "\u{301}".repeat(65536)),
            String::new()
        ]
    );
}

/// LIM-03 Fault: what the live output policy never forwards to a host
/// (titles and the title stack, hyperlinks, device control strings, clipboard
/// and queries) never reaches the engine. A model at its mandatory limit
/// takes any amount of it, and the allocator never sees the limit exceeded.
#[test]
fn sequences_no_host_receives_never_grow_the_model() {
    let at = geometry(10, 3);
    let long = "t".repeat(4000);
    let stream = [
        format!("\x1b]2;{long}\x07").repeat(16),
        "\x1b[22t".repeat(4096),
        format!("\x1b]8;id={long};https://{long}\x1b\\link\x1b]8;;\x1b\\"),
        format!("\x1bP+q{long}\x1b\\\x1b]52;c;{long}\x07"),
        "\x1b[c\x1b[>c\x1b[6n\x1b]11;?\x07\x1b[?2026$p\x1b[18t".repeat(256),
    ]
    .concat();
    let limit = mandatory(at);
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();

    model.feed(stream.as_bytes()).unwrap();

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert!(meter.live() <= limit);
    assert_eq!(model.footprint(), limit);
    assert_eq!(model.screen(), ["link", "", ""]);
}
