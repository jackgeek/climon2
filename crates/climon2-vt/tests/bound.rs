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
    static TOTAL: Cell<usize> = const { Cell::new(0) };
}

fn record(delta: isize) {
    let _ = LIVE.try_with(|live| {
        let now = live.get().wrapping_add(delta);
        live.set(now);
        let _ = PEAK.try_with(|peak| peak.set(peak.get().max(now)));
    });
    let _ =
        TOTAL.try_with(|total| total.set(total.get().wrapping_add(delta.max(0).cast_unsigned())));
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

/// Bytes this thread allocated and has not freed since `watch` started, the
/// most it held at once, and every byte it allocated.
struct Meter {
    start: isize,
    total: usize,
}

impl Meter {
    fn watch() -> Self {
        let start = LIVE.with(Cell::get);
        PEAK.with(|peak| peak.set(start));
        Self {
            start,
            total: TOTAL.with(Cell::get),
        }
    }

    fn total(&self) -> usize {
        TOTAL.with(Cell::get) - self.total
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

    model
        .feed(b"\x1b7hel\x1b[1mlo\x1b[0m\x1b8\r\nworld\r\n\xe4\xb8\x80x\x07")
        .unwrap();

    assert!(meter.peak() <= footprint);
    assert_eq!(model.footprint(), footprint);
    assert_eq!(model.screen(), ["hello", "world", "\u{4e00}x"]);
}

const ROWS: usize = 100;

/// The bytes a model needs before any output.
fn mandatory(at: Geometry) -> usize {
    needed(Model::new(at, limits(0, ROWS)))
}

/// A model with the smallest limit that accepts `bytes`, and that limit.
fn fitted(at: Geometry, bytes: &[u8]) -> (Model, usize) {
    let mut limit = mandatory(at);
    loop {
        let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
        match model.feed(bytes) {
            Ok(()) => return (model, limit),
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
    let bound = needed(fitted(at, before).0.feed(update));

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
/// adds a chunk of rows at a time, admitted before output that could fill the
/// current chunk. At the bound the next chunk is admitted and every row kept;
/// one byte below it the chunk is refused before it is allocated and the
/// oldest rows are discarded instead, while the update itself is accepted with
/// the newest page and every character intact.
#[test]
fn history_grows_at_the_bound_and_discards_its_oldest_row_one_byte_past_it() {
    let at = geometry(8, 2);
    let rows = |range: std::ops::Range<usize>| -> Vec<String> {
        range.map(|row| format!("row{row}")).collect()
    };
    let early = rows(0..900).join("\r\n");
    let late = format!("\r\n{}", rows(900..1003).join("\r\n"));
    let mut roomy = Model::new(at, limits(ROOMY, 2000)).unwrap();
    roomy.feed(early.as_bytes()).unwrap();
    let before = roomy.footprint();
    roomy.feed(late.as_bytes()).unwrap();
    let bound = roomy.footprint();
    assert!(bound > before);

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, 2000)).unwrap();
    model.feed(early.as_bytes()).unwrap();
    model.feed(late.as_bytes()).unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.history(), rows(0..1001));
    assert_eq!(model.screen(), rows(1001..1003));

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound - 1, 2000)).unwrap();
    model.feed(early.as_bytes()).unwrap();
    model.feed(late.as_bytes()).unwrap();
    assert!(meter.peak() < bound, "{} >= {bound}", meter.peak());
    assert_eq!(model.footprint(), before);
    assert_eq!(model.history(), rows(1..1001));
    assert_eq!(model.screen(), rows(1001..1003));
}

/// LIM-03 Contract: the next chunk is admitted only before output that could
/// fill the current one: output that at most reaches it grows nothing.
#[test]
fn history_grows_only_before_output_that_could_fill_it() {
    let at = geometry(8, 2);
    let mut model = Model::new(at, limits(ROOMY, 2000)).unwrap();
    for _ in 0..999 {
        model.feed(b"x\r\n").unwrap();
    }
    assert_eq!(model.history().len(), 998);
    let footprint = model.footprint();

    model.feed(b"a\n").unwrap();
    assert_eq!(model.footprint(), footprint);
    assert_eq!(model.history().len(), 999);
    model.feed(b"\r\n\r\n").unwrap();
    assert!(model.footprint() > footprint);
    assert_eq!(model.history().len(), 1001);
}

/// LIM-03 Contract: erased screens and cleared history release their marks.
/// A model whose running charge leaves no room measures what its cells hold
/// before it rejects anything, so released marks make room for new ones.
#[test]
fn released_marks_make_room_before_anything_is_rejected() {
    let at = geometry(10, 3);
    let update = format!("a{}", "\u{301}".repeat(31));
    let bound = needed(fitted(at, b"").0.feed(update.as_bytes()));
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

/// LIM-03 Contract: a restoration is staged into a separate model while the
/// current one stays visible, and the two are charged together. Restoration
/// bytes that fit beside the current model are accepted at the bound and
/// replace it whole when applied; one byte past it they are rejected, the
/// partial restoration is discarded and the last safe page stays current and
/// read-only.
#[test]
fn restoration_staging_is_accepted_at_the_bound_and_rejected_one_byte_past_it() {
    let at = geometry(10, 3);
    let restoration = format!("restored\r\na{}", "\u{301}".repeat(20));
    let mut probe = Model::new(at, limits(mandatory(at), ROWS)).unwrap();
    probe.feed(b"live page").unwrap();
    let both = needed(probe.stage(at));
    let mut probe = Model::new(at, limits(both, ROWS)).unwrap();
    probe.feed(b"live page").unwrap();
    probe.stage(at).unwrap();
    let bound = needed(probe.feed(restoration.as_bytes()));

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, ROWS)).unwrap();
    model.feed(b"live page").unwrap();
    model.stage(at).unwrap();
    model.feed(restoration.as_bytes()).unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.screen(), ["live page", "", ""]);
    assert!(model.apply());
    assert!(model.footprint() < both);
    assert_eq!(
        model.screen(),
        [
            "restored".to_owned(),
            format!("a{}", "\u{301}".repeat(20)),
            String::new()
        ]
    );

    let mut model = Model::new(at, limits(bound - 1, ROWS)).unwrap();
    model.feed(b"live page").unwrap();
    model.stage(at).unwrap();
    let rejected = Rejected {
        needed: bound,
        limit: bound - 1,
    };
    assert_eq!(model.feed(restoration.as_bytes()), Err(rejected));
    assert_eq!(model.feed(b"more"), Err(rejected));
    assert!(!model.apply());
    assert_eq!(model.screen(), ["live page", "", ""]);
}

/// LIM-03 Contract: a model never reflows in place; a new effective geometry
/// arrives with a restoration staged at that geometry. Staging a larger
/// geometry is accepted when both models fit (at the bound) and rejected one
/// byte past it before anything is allocated, keeping the current page.
#[test]
fn resizing_stages_a_replacement_at_the_bound_and_is_rejected_one_byte_past_it() {
    let small = geometry(10, 3);
    let large = geometry(200, 60);
    let mut probe = Model::new(small, limits(mandatory(small), ROWS)).unwrap();
    probe.feed(b"page").unwrap();
    let bound = needed(probe.stage(large));

    let meter = Meter::watch();
    let mut model = Model::new(small, limits(bound, ROWS)).unwrap();
    model.feed(b"page").unwrap();
    model.stage(large).unwrap();
    model.feed(b"\x1b[60;190Hcorner").unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.geometry(), small);
    assert!(model.apply());
    assert_eq!(model.geometry(), large);
    assert_eq!(model.screen()[59].trim_start(), "corner");

    let meter = Meter::watch();
    let mut model = Model::new(small, limits(bound - 1, ROWS)).unwrap();
    model.feed(b"page").unwrap();
    assert_eq!(
        model.stage(large),
        Err(Rejected {
            needed: bound,
            limit: bound - 1
        })
    );
    assert!(meter.peak() < bound);
    assert_eq!(model.geometry(), small);
    assert_eq!(model.screen(), ["page", "", ""]);
}

/// LIM-03 Contract: a superseded or abandoned restoration is discarded whole;
/// the current model is never touched by it, and its bytes are released.
#[test]
fn discarded_restorations_never_become_current() {
    let at = geometry(10, 3);
    let mut model = Model::new(at, limits(ROOMY, ROWS)).unwrap();
    model.feed(b"current").unwrap();
    let alone = model.footprint();
    model.stage(at).unwrap();
    model.feed(b"first").unwrap();
    model.stage(at).unwrap();
    model.feed(b"second").unwrap();
    assert!(model.footprint() > alone);

    model.discard();

    assert_eq!(model.footprint(), alone);
    assert!(!model.apply());
    assert_eq!(model.screen(), ["current", "", ""]);
    model.stage(at).unwrap();
    model.feed(b"third").unwrap();
    assert!(model.apply());
    assert_eq!(model.screen(), ["third", "", ""]);
}

/// A screenful of cells, each a letter carrying `marks` combining marks.
fn marked(at: Geometry, marks: usize) -> String {
    let cell = format!("a{}", "\u{301}".repeat(marks));
    let row = cell.repeat(usize::from(at.cols()));
    vec![row; usize::from(at.rows())].join("\r\n")
}

/// LIM-03 Contract: each screen's cell data is charged separately. Marks
/// left on the alternate screen stay charged while the normal screen is
/// active, because the engine keeps them, so the normal screen cannot use
/// their room; entering the alternate screen again clears it, and measuring
/// makes the room available.
#[test]
fn alternate_screen_marks_stay_charged_until_that_screen_is_cleared() {
    let at = geometry(10, 3);
    let alternate = format!("\x1b[?1049h\x1b[H{}", marked(at, 20));
    let (mut model, limit) = fitted(at, alternate.as_bytes());
    model.feed(b"\x1b[?1049l").unwrap();
    let normal = marked(at, 20);
    assert!(model.feed(normal.as_bytes()).is_err());
    assert_eq!(model.screen(), ["", "", ""]);
    drop(model);

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
    for _ in 0..3 {
        model.feed(alternate.as_bytes()).unwrap();
        model.feed(b"\x1b[?1049l").unwrap();
    }
    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert_eq!(model.screen(), ["", "", ""]);
}

/// LIM-03 Contract: a model that rejected output stays read-only until a
/// staged restoration replaces it whole; then it accepts output again.
#[test]
fn a_rejected_model_accepts_output_again_after_a_restoration_is_applied() {
    let at = geometry(10, 3);
    let marks = format!("a{}", "\u{301}".repeat(20));
    let mut model = Model::new(at, limits(mandatory(at) * 2 + 1000, ROWS)).unwrap();
    model.feed("kept\u{301}".as_bytes()).unwrap();
    assert!(model.feed(b"\x1b[65535b").is_err());
    let kept = model.screen();
    assert_eq!(kept, ["kept\u{301}", "", ""]);
    assert!(model.feed(b"x").is_err());

    model.stage(at).unwrap();
    model.feed(b"restored\r\n").unwrap();
    assert_eq!(model.screen(), kept);
    assert!(model.apply());
    model.feed(marks.as_bytes()).unwrap();

    assert_eq!(
        model.screen(),
        ["restored".to_owned(), marks, String::new()]
    );
}

/// LIM-03 Contract: apart from the reserve every model keeps, a model's
/// charge for its engine and screens is exactly what constructing it
/// allocated, at every geometry.
#[test]
fn a_fresh_model_charges_exactly_what_its_engine_allocated() {
    let built = |cols, rows| {
        let meter = Meter::watch();
        let model = Model::new(geometry(cols, rows), limits(ROOMY, 0)).unwrap();
        assert_eq!(model.footprint(), meter.peak() + Model::RESERVE);
        (meter.peak(), model.footprint())
    };
    let (small, small_charge) = built(2, 1);
    for (cols, rows) in [(80, 24), (200, 60), (500, 200)] {
        let (large, large_charge) = built(cols, rows);
        assert_eq!(large_charge - small_charge, large - small, "{cols}x{rows}");
    }
}

/// LIM-03 Contract: a long history at a narrow geometry is mostly the
/// engine's row vector, which doubles as history grows and briefly holds its
/// old buffer while it does. A model at the smallest limit that keeps ten
/// thousand rows never lets the allocator see that limit exceeded.
#[test]
fn a_long_history_stays_within_the_smallest_limit_that_keeps_it() {
    let at = geometry(2, 1);
    let output = "x\r\n".repeat(10_500);
    let mut roomy = Model::new(at, limits(ROOMY, 10_000)).unwrap();
    roomy.feed(output.as_bytes()).unwrap();
    let limit = roomy.footprint();

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
    model.feed(output.as_bytes()).unwrap();

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert_eq!(model.history().len(), 10_000);
}

/// LIM-03 Fault: a control sequence holding 4096 controls and then cancelled
/// releases them as 4097 separate tokens at once; that burst stays within
/// the reserve of a model at its mandatory limit.
#[test]
fn a_cancelled_sequence_releasing_held_controls_stays_within_the_limit() {
    let at = geometry(10, 3);
    let burst = [
        b"\x1b[".as_slice(),
        &[0x07; 4200],
        &[0x18],
        b"\x1b[".as_slice(),
        &[0x07; 4200],
        b"\x1b[m",
    ]
    .concat();
    let limit = mandatory(at);
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();

    model.feed(&burst).unwrap();

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert_eq!(model.footprint(), limit);
}

/// LIM-03 Contract: a full reset releases every mark on both screens and the
/// history holding them, so the room is available again after measuring.
#[test]
fn a_full_reset_releases_marks_and_history() {
    let at = geometry(10, 3);
    let heavy = format!("{}\r\n{}", marked(at, 20), marked(at, 20));
    let limit = fitted(at, heavy.as_bytes()).1;
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
    model.feed(heavy.as_bytes()).unwrap();

    for _ in 0..3 {
        model.feed(b"\x1bc").unwrap();
        model.feed(heavy.as_bytes()).unwrap();
    }

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert_eq!(model.history().len(), 3);
}

/// LIM-03 Contract: history storage is added a chunk at a time and kept: a
/// model printing thousands of lines, with sequences between them that push
/// nothing, allocates less in total than its footprint.
#[test]
fn history_storage_is_allocated_a_chunk_at_a_time() {
    let at = geometry(10, 2);
    let output = "\x1b[1mline\x1b[0m\r\n".repeat(2500);
    let mut model = Model::new(at, limits(ROOMY, 3000)).unwrap();
    let meter = Meter::watch();

    model.feed(output.as_bytes()).unwrap();

    assert!(
        meter.total() <= model.footprint(),
        "{} > {}",
        meter.total(),
        model.footprint()
    );
    assert_eq!(model.history().len(), 2499);
}

/// LIM-03 Contract: clearing history leaves the engine's storage rows, and
/// the marks on them, beyond the history it shows. The model drops those rows
/// at once, so measuring sees every mark still held: screenful after
/// screenful of marked cells pushed into history and cleared never takes the
/// allocator past the limit that fits one.
#[test]
fn cleared_history_releases_its_marks() {
    let at = geometry(100, 40);
    let screenful = format!("\x1b[H{}", marked(at, 1));
    let limit = fitted(at, screenful.as_bytes()).1;
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
    model.feed(screenful.as_bytes()).unwrap();

    for _ in 0..8 {
        model.feed(b"\x1b[2J\x1b[3J").unwrap();
        model.feed(screenful.as_bytes()).unwrap();
    }

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert_eq!(model.history(), Vec::<String>::new());
}

/// LIM-03 Contract: history configured as whole chunks never grows past
/// them, however much output scrolls by; the oldest rows go instead.
#[test]
fn history_never_grows_past_its_configured_rows() {
    let at = geometry(8, 2);
    let mut model = Model::new(at, limits(ROOMY, 1000)).unwrap();
    let footprint = model.footprint();

    model.feed("line\r\n".repeat(1500).as_bytes()).unwrap();

    assert_eq!(model.footprint(), footprint);
    assert_eq!(model.history().len(), 1000);
}

/// LIM-03 Contract: the alternate screen has no history of its own, and the
/// normal screen's history is not shown over it.
#[test]
fn the_alternate_screen_shows_no_history() {
    let mut model = Model::new(geometry(8, 2), limits(ROOMY, ROWS)).unwrap();
    model.feed(b"one\r\ntwo\r\nthree\x1b[?1049h").unwrap();
    assert_eq!(model.history(), Vec::<String>::new());
    model.feed(b"\x1b[?1049l").unwrap();
    assert_eq!(model.history(), ["one"]);
}

/// The bytes charged for feeding `update` after `before` into a roomy model.
fn charged(before: &[u8], update: &[u8]) -> usize {
    let mut model = Model::new(geometry(80, 3), limits(ROOMY, ROWS)).unwrap();
    model.feed(before).unwrap();
    let footprint = model.footprint();
    model.feed(update).unwrap();
    model.footprint() - footprint
}

const TINT: &[u8] = b"\x1b[58;5;1m";

/// LIM-03 Contract: what printing can grow depends on what it prints. Under
/// an underline colour, an invalid byte prints one replacement cell like any
/// narrow character and a wide character up to three cells; a combining mark
/// costs the same with or without one, and `CSI n b` costs `n` repetitions of
/// the worst character it could repeat, one when `n` is absent or zero.
#[test]
fn growth_is_charged_by_what_printing_can_hold() {
    let narrow = charged(TINT, b"x");
    assert!(narrow > 0);
    assert_eq!(charged(TINT, b"\xff"), narrow);
    assert_eq!(charged(TINT, b"x\xffx"), 3 * narrow);
    assert_eq!(charged(TINT, "\u{4e00}".as_bytes()), 3 * narrow);
    assert_eq!(charged(b"", b"x\xff\x07"), 0);

    let mark = charged(b"a", "\u{301}".as_bytes());
    assert_eq!(charged(&[TINT, b"a"].concat(), "\u{301}".as_bytes()), mark);
    assert_eq!(charged(b"a", b"\x1b[b"), mark);
    assert_eq!(charged(b"a", b"\x1b[0b"), mark);
    assert_eq!(charged(b"a", b"\x1b[25b"), 25 * mark);
    assert_eq!(
        charged(&[TINT, b"a"].concat(), b"\x1b[25b"),
        25 * 3 * narrow
    );
}

/// LIM-03 Contract: marks accumulating on every cell, a few per update, are
/// measured as they are kept, so however many updates a model accepts the
/// allocator never sees its limit exceeded.
#[test]
fn marks_accumulating_on_every_cell_stay_within_the_limit() {
    let at = geometry(80, 24);
    let limit = mandatory(at) + 400 * 1024;
    let round: String = (1..=24)
        .flat_map(|row| (2..=81).map(move |col| format!("\x1b[{row};{col}H\u{301}")))
        .collect();
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
    model.feed(marked(at, 0).as_bytes()).unwrap();

    let accepted = (0..200)
        .take_while(|_| model.feed(round.as_bytes()).is_ok())
        .count();

    assert!(accepted > 0 && accepted < 200, "{accepted}");
    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
}

/// LIM-03 Fault: repeated `CSI 65535 b` of a combining mark is rejected as
/// soon as one repetition would not fit, so marks never pile up past the
/// limit on one cell.
#[test]
fn repeated_repeats_stay_within_the_limit() {
    let at = geometry(10, 3);
    let limit = mandatory(at) + 2 * 1024 * 1024;
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, ROWS)).unwrap();
    model.feed("a\u{301}".as_bytes()).unwrap();

    let repeated = (0..20)
        .take_while(|_| model.feed(b"\x1b[65535b").is_ok())
        .count();

    assert_eq!(repeated, 0);
    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
}

/// LIM-03 Contract: the largest limits never overflow the accounting: a
/// model allowed every byte and every history row still admits output and
/// grows its history chunk by chunk.
#[test]
fn the_largest_limits_never_overflow_the_accounting() {
    let mut model = Model::new(geometry(10, 3), limits(usize::MAX, usize::MAX)).unwrap();

    model.feed("a\u{301}\x1b[65535b\r\n".as_bytes()).unwrap();
    model.feed("x\r\n".repeat(1100).as_bytes()).unwrap();

    assert!(model.footprint() < usize::MAX);
    assert_eq!(model.history().len(), 1099);
}
