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

/// LIM-03 Contract and CFG-01 Contract: `Model::mandatory` is the allocation
/// `Model::new` admits a blank model with, at the bound and one byte below
/// it, with and without history. At the largest geometry, 500x200, it is the
/// documented 6,948,039 bytes without history and 19,063,239 with the first
/// history chunk, which holds the protected newest page
/// (docs/monitor-operational-limits.md).
#[test]
fn a_models_mandatory_bytes_are_what_it_is_admitted_with() {
    let largest = geometry(500, 200);
    assert_eq!(Model::mandatory(largest, NO_HISTORY), 6_948_039);
    assert_eq!(Model::mandatory(largest, 10_000), 19_063_239);
    let small = geometry(80, 24);
    for (at, rows) in [(small, NO_HISTORY), (small, 10_000), (largest, 10_000)] {
        let mandatory = Model::mandatory(at, rows);
        assert_eq!(
            Model::new(at, limits(mandatory - 1, rows)).err(),
            Some(Rejected {
                needed: mandatory,
                limit: mandatory - 1
            })
        );
        let model = Model::new(at, limits(mandatory, rows)).unwrap();
        assert_eq!(model.footprint(), mandatory);
    }
}

const ROOMY: usize = 64 * 1024 * 1024;

/// LIM-03 Contract: plain text, invalid bytes included, fills cells the
/// engine already holds, so it is accepted unchanged without charging
/// anything more.
#[test]
fn plain_text_fills_held_cells_without_growth() {
    let meter = Meter::watch();
    let mut model = Model::new(geometry(10, 3), limits(ROOMY, NO_HISTORY)).unwrap();
    let footprint = model.footprint();

    model
        .feed(b"\x1b7hel\x1b[1mlo\x1b[0m\x1b8\r\nworld\r\n\xe4\xb8\x80x\xff\x07")
        .unwrap();

    assert!(meter.peak() <= footprint);
    assert_eq!(model.footprint(), footprint);
    assert_eq!(model.screen(), ["hello", "world", "\u{4e00}x\u{fffd}"]);
}

/// History rows for tests about anything but history: none, so a charged but
/// still unallocated history chunk cannot hide an undercharge.
const NO_HISTORY: usize = 0;

/// History rows for tests about history.
const HISTORY: usize = 100;

/// The bytes a model needs before any output.
fn mandatory(at: Geometry) -> usize {
    needed(Model::new(at, limits(0, NO_HISTORY)))
}

/// A model with the smallest limit that accepts `bytes`, and that limit.
fn fitted(at: Geometry, bytes: &[u8]) -> (Model, usize) {
    fitted_with(at, NO_HISTORY, bytes)
}

/// [`fitted`] for a model keeping `history_rows` history rows.
fn fitted_with(at: Geometry, history_rows: usize, bytes: &[u8]) -> (Model, usize) {
    let mut limit = needed(Model::new(at, limits(0, history_rows)));
    loop {
        let mut model = Model::new(at, limits(limit, history_rows)).unwrap();
        match model.feed(bytes) {
            Ok(()) => return (model, limit),
            Err(rejected) => limit = rejected.needed,
        }
    }
}

/// Feed `before`, then `update`, into models whose limits sit exactly at the
/// bytes `update` needs and one byte below. At the bound the update is
/// accepted, the allocator never saw more than the limit, and what the update
/// left allocated is no more than it was charged, so the reserve's unused
/// room cannot hide an undercharge; one byte below it is rejected before it
/// reaches the engine, leaving the page `before` produced. Returns the
/// accepted model's screen.
#[track_caller]
fn assert_bound(at: Geometry, before: &[u8], update: &[u8]) -> Vec<String> {
    let bound = needed(fitted(at, before).0.feed(update));

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, NO_HISTORY)).unwrap();
    model.feed(before).unwrap();
    let (held, charged) = (meter.live(), model.footprint());
    model.feed(update).unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    let kept = meter.live().saturating_sub(held);
    let charge = model.footprint() - charged;
    assert!(kept <= charge, "kept {kept} > charged {charge}");
    let accepted = model.screen();

    let mut model = Model::new(at, limits(bound - 1, NO_HISTORY)).unwrap();
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
    for (update, shown) in [
        (&b"under"[..], "under"),
        (b"in\xffvalid", "in\u{fffd}valid"),
    ] {
        let accepted = assert_bound(geometry(10, 3), b"page\r\n\x1b[58;5;1m", update);
        assert_eq!(accepted, ["page", shown, ""]);
    }
}

/// LIM-03 Contract: an underline colour changed between cells leaves each
/// earlier cell holding its own copy of the extra data. Whatever is printed
/// that way, narrow, invalid or wide, the model keeps no more than it was
/// charged, once the front filter has held text.
#[test]
fn cells_under_changing_underline_colours_keep_no_more_than_they_were_charged() {
    for glyph in [&b"x"[..], b"\xff", "\u{4e00}".as_bytes()] {
        let meter = Meter::watch();
        let mut model = Model::new(geometry(10, 3), limits(ROOMY, NO_HISTORY)).unwrap();
        model.feed(b"\r").unwrap();
        let (held, charged) = (meter.live(), model.footprint());

        for colour in 1..=30 {
            let tinted = [format!("\x1b[58;5;{colour}m").as_bytes(), glyph].concat();
            model.feed(&tinted).unwrap();
        }

        let kept = meter.live() - held;
        let charge = model.footprint() - charged;
        assert!(kept <= charge, "{glyph:?}: kept {kept} > charged {charge}");
    }
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

/// Rows `row<n>` for each `n` in `range`.
fn rows(range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|row| format!("row{row}")).collect()
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
    let mut model = Model::new(at, limits(bound, NO_HISTORY)).unwrap();

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

/// LIM-03 Fault: a repeat count is the one the parser uses, wherever C0
/// controls sit in the sequence: before the introducer, before the digits or
/// among them; an absent or zero count repeats once. Each form is accepted
/// at the bound, with every mark it repeats, and rejected one byte past it.
#[test]
fn repeat_counts_are_the_parsers_whatever_controls_sit_in_them() {
    for (repeat, marks) in [
        (&b"\x1b\x07[65535b"[..], 65536),
        (b"\x1b[\x0765535b", 65536),
        (b"\x1b[6\x075535b", 65536),
        (b"\x1b[b", 2),
        (b"\x1b[0b", 2),
    ] {
        let accepted = assert_bound(geometry(10, 3), "page\r\na\u{301}".as_bytes(), repeat);
        assert_eq!(
            accepted,
            [
                "page".to_owned(),
                format!("a{}", "\u{301}".repeat(marks)),
                String::new()
            ],
            "{repeat:?}"
        );
    }
}

/// LIM-03 Contract: one token can push several chunks of history: a long
/// repeat scrolls a row per screen width it prints, and an abandoned
/// sequence releases the line feeds it held as one token. Every chunk such a
/// token could fill is admitted before it, so every row it pushes is kept.
/// For the line feeds that is exactly at the bound; one byte below it, the
/// chunks that fit are admitted and only the oldest rows beyond them are
/// discarded.
#[test]
fn one_token_admits_every_history_chunk_it_could_fill() {
    let at = geometry(2, 1);
    let early = "x\r\n".repeat(900);
    let mut model = Model::new(at, limits(ROOMY, 10_000)).unwrap();
    model.feed(early.as_bytes()).unwrap();

    model.feed(b"y\x1b[2500b").unwrap();

    let history = model.history();
    assert_eq!(history.len(), 900 + 1250);
    assert_eq!(history[..900], vec!["x".to_owned(); 900]);
    assert_eq!(history[900..], vec!["yy".to_owned(); 1250]);
    assert_eq!(model.screen(), ["y"]);

    let feeds = [b"\x1b[".as_slice(), &[b'\n'; 2500], b"\x18"].concat();
    let fed = |limit| {
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(early.as_bytes()).unwrap();
        model.feed(&feeds).unwrap();
        model
    };
    let bound = fed(ROOMY).footprint();
    let kept = |x: usize| [vec!["x".to_owned(); x], vec![String::new(); 2500]].concat();

    let meter = Meter::watch();
    let model = fed(bound);
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.history(), kept(900));

    let meter = Meter::watch();
    let model = fed(bound - 1);
    assert!(meter.peak() < bound, "{} >= {bound}", meter.peak());
    assert_eq!(model.history(), kept(500));
}

/// LIM-03 Fault: what the live output policy never forwards to a host
/// (titles and the title stack, hyperlinks, device control strings, clipboard
/// and queries) never reaches the engine, including after a C0 control
/// between `ESC` and the introducer. A model with no history at its mandatory
/// limit takes any amount of it, and the allocator never sees the limit
/// exceeded.
#[test]
fn sequences_no_host_receives_never_grow_the_model() {
    let at = geometry(10, 3);
    let long = "t".repeat(4000);
    let hostile = |esc: &str| {
        [
            format!("{esc}]2;{long}\x07").repeat(16),
            format!("{esc}[22t").repeat(4096),
            format!("{esc}]8;id={long};https://{long}\x1b\\"),
            format!("{esc}P+q{long}\x1b\\{esc}]52;c;{long}\x07"),
            format!("{esc}[c{esc}[>c{esc}[6n{esc}]11;?\x07{esc}[?2026$p{esc}[18t").repeat(256),
        ]
        .concat()
    };
    let stream = [
        hostile("\x1b"),
        "link".to_owned(),
        hostile("\x1b\x07"),
        "LINK".to_owned(),
    ]
    .concat();
    let limit = mandatory(at);
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();

    model.feed(stream.as_bytes()).unwrap();

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert!(meter.live() <= limit);
    assert_eq!(model.footprint(), limit);
    assert_eq!(model.screen(), ["linkLINK", "", ""]);
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
    let mut probe = Model::new(at, limits(mandatory(at), NO_HISTORY)).unwrap();
    probe.feed(b"live page").unwrap();
    let both = needed(probe.stage(at));
    let mut probe = Model::new(at, limits(both, NO_HISTORY)).unwrap();
    probe.feed(b"live page").unwrap();
    probe.stage(at).unwrap();
    let bound = needed(probe.feed(restoration.as_bytes()));

    let meter = Meter::watch();
    let mut model = Model::new(at, limits(bound, NO_HISTORY)).unwrap();
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

    let mut model = Model::new(at, limits(bound - 1, NO_HISTORY)).unwrap();
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
    let mut probe = Model::new(small, limits(mandatory(small), NO_HISTORY)).unwrap();
    probe.feed(b"page").unwrap();
    let bound = needed(probe.stage(large));

    let meter = Meter::watch();
    let mut model = Model::new(small, limits(bound, NO_HISTORY)).unwrap();
    model.feed(b"page").unwrap();
    model.stage(large).unwrap();
    model.feed(b"\x1b[60;190Hcorner").unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.geometry(), small);
    assert!(model.apply());
    assert_eq!(model.geometry(), large);
    assert_eq!(model.screen()[59].trim_start(), "corner");

    let meter = Meter::watch();
    let mut model = Model::new(small, limits(bound - 1, NO_HISTORY)).unwrap();
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
    let mut model = Model::new(at, limits(ROOMY, NO_HISTORY)).unwrap();
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
    let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();
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
    let mut model = Model::new(at, limits(mandatory(at) * 2 + 1000, NO_HISTORY)).unwrap();
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
/// or restarted releases them at once; they reach the engine within the
/// reserve of a model with no history at its mandatory limit.
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
    let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();

    model.feed(&burst).unwrap();

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    assert_eq!(model.footprint(), limit);
}

/// LIM-03 Fault: the reserve covers what a model holds only briefly. A model
/// with no history at its mandatory limit takes a control string, a device
/// control string and a CSI of the largest body held across one-byte reads,
/// 4000 controls before an introducer, repeated full resets and cursor
/// templates tinted on both screens, and the allocator never sees the limit
/// exceeded.
#[test]
fn the_reserve_covers_what_is_held_briefly() {
    let at = geometry(80, 24);
    let body = "A".repeat(4090);
    let tints = "\x1b[58;5;1m\x1b7\x1b[?1049h\x1b[58;5;2m\x1b7\x1b[?1049l\x1b[58;5;3m\x1b[?1049h";
    for (stream, read) in [
        (format!("\x1b]52;c;{body}\x07"), 1),
        (format!("\x1bP{body}\x1b\\"), 1),
        (format!("\x1b[{}m", "1;".repeat(2047)), 1),
        (format!("\x1b{}]52;c;aGVsbG8=\x07", "\x07".repeat(4000)), 64),
        ("\x1bc".repeat(100), 64),
        (tints.to_owned(), 64),
    ] {
        let limit = mandatory(at);
        let meter = Meter::watch();
        let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();

        for read in stream.as_bytes().chunks(read) {
            model.feed(read).unwrap();
        }

        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    }
}

/// LIM-03 Contract: a full reset releases every mark on both screens and the
/// history holding them, so the room is available again after measuring.
#[test]
fn a_full_reset_releases_marks_and_history() {
    let at = geometry(10, 3);
    let heavy = format!("{}\r\n{}", marked(at, 20), marked(at, 20));
    let limit = fitted_with(at, HISTORY, heavy.as_bytes()).1;
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, HISTORY)).unwrap();
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
    let limit = fitted_with(at, HISTORY, screenful.as_bytes()).1;
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, HISTORY)).unwrap();
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

/// LIM-03 Contract: a model shows at most `history_rows` rows of history, the
/// newest. At the limit every row scrolled into history is shown; one row
/// past it the oldest is no longer shown, though the engine's first chunk
/// still holds it.
#[test]
fn history_shows_its_rows_at_the_limit_and_not_the_oldest_one_past_it() {
    let at = geometry(8, 2);
    for (scrolled, shown) in [(HISTORY, 0..HISTORY), (HISTORY + 1, 1..HISTORY + 1)] {
        let mut model = Model::new(at, limits(ROOMY, HISTORY)).unwrap();

        model
            .feed(rows(0..scrolled + 2).join("\r\n").as_bytes())
            .unwrap();

        assert_eq!(model.history(), rows(shown), "{scrolled} rows scrolled");
        assert_eq!(model.screen(), rows(scrolled..scrolled + 2));
    }
}

/// Row `n` as printed at `at`: its number, then cells each carrying a
/// combining mark to the screen's width.
fn marked_row(at: Geometry, n: usize) -> String {
    let number = format!("{n:06}");
    let cells = usize::from(at.cols()) - number.len();
    format!("{number}{}", "a\u{301}".repeat(cells))
}

/// [`marked_row`] for each `n` in `range`.
fn marked_rows(at: Geometry, range: std::ops::Range<usize>) -> Vec<String> {
    range.map(|n| marked_row(at, n)).collect()
}

/// LIM-03 Contract (worked case from the #159 review; history discards its
/// oldest rows as it exceeds either ceiling, not required live updates): at
/// 200x60 with 64 MiB and 10,000 history rows, rows whose every cell carries
/// a combining mark outgrow the room for history about 3,000 rows in. The
/// model discards its oldest history a chunk at a time instead of rejecting
/// them: it accepts all 6,000 rows, keeps the newest, never fewer than its
/// first chunk holds, and the allocator never sees the limit exceeded.
#[test]
fn live_output_discards_the_oldest_history_instead_of_being_rejected() {
    let at = geometry(200, 60);
    let limit = 64 << 20;
    let rows = marked_rows(at, 0..6000);
    let output = rows.join("\r\n");
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(limit, 10_000)).unwrap();

    model.feed(output.as_bytes()).unwrap();

    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    let history = model.history();
    assert!(history.len() >= 1000, "{} rows kept", history.len());
    assert_eq!(history, rows[5940 - history.len()..5940]);
    assert_eq!(model.screen(), rows[5940..]);
}

/// LIM-03 Contract: live output that fits beside every history row keeps
/// them all, at the bound; one byte below it, the oldest chunk's rows are
/// discarded instead, the newest 2,000 of 2,500 are kept, and the output is
/// accepted with the page intact. The allocator never sees either limit
/// exceeded.
#[test]
fn history_yields_its_oldest_chunk_to_live_output_one_byte_below_the_bound() {
    let at = geometry(200, 60);
    let rows = marked_rows(at, 0..2559);
    let before = format!("{}\r\n", rows.join("\r\n"));
    let update = "a\u{301}".repeat(50);
    let fed = |limit| {
        let meter = Meter::watch();
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(before.as_bytes()).unwrap();
        model.feed(update.as_bytes()).unwrap();
        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
        model
    };
    let bound = fed(ROOMY).footprint();
    let page = [&rows[2500..], std::slice::from_ref(&update)].concat();

    let model = fed(bound);
    assert_eq!(model.history(), rows[..2500]);
    assert_eq!(model.screen(), page);

    let model = fed(bound - 1);
    assert_eq!(model.history(), rows[500..2500]);
    assert_eq!(model.screen(), page);
}

/// LIM-03 Contract: history never yields its first chunk, which holds the
/// protected newest page. With 1,500 marked rows in two chunks, a repeat of
/// 65535 marks that fits only once the oldest chunk is discarded is accepted
/// at that bound with the newest 1,000 rows; one byte below it, the repeat
/// is rejected before growth, with the newest 1,000 rows and the page intact.
#[test]
fn history_never_yields_its_first_chunk() {
    let at = geometry(10, 3);
    let rows = marked_rows(at, 0..1502);
    let before = format!("{}\r\na\u{301}", rows.join("\r\n"));
    let fed = |limit| {
        let meter = Meter::watch();
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(before.as_bytes()).unwrap();
        let fed = model.feed(b"\x1b[65535b");
        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
        (model, fed)
    };
    let mut roomy = Model::new(at, limits(ROOMY, 10_000)).unwrap();
    roomy.feed(before.as_bytes()).unwrap();
    let bound = needed(fed(roomy.footprint()).1);
    let page = |marks: usize| {
        let last = format!("a{}", "\u{301}".repeat(marks));
        [&rows[1500..], &[last]].concat()
    };

    let (model, accepted) = fed(bound);
    assert_eq!(accepted, Ok(()));
    assert_eq!(model.history(), rows[500..1500]);
    assert_eq!(model.screen(), page(65536));

    let (model, rejected) = fed(bound - 1);
    assert_eq!(
        rejected,
        Err(Rejected {
            needed: bound,
            limit: bound - 1
        })
    );
    assert_eq!(model.history(), rows[500..1500]);
    assert_eq!(model.screen(), page(1));
}

/// LIM-03 Contract: history yields while the alternate screen is active,
/// whose cells it never touches. A model with 1,500 history rows in two
/// chunks fills its alternate screen with marked cells: at the bound every
/// history row stays; one byte below it the oldest chunk's rows are
/// discarded, the alternate screen keeps every mark, and leaving it shows
/// the normal screen as it was with the newest 1,000 rows.
#[test]
fn history_yields_to_the_alternate_screen() {
    let at = geometry(10, 3);
    let rows = rows(0..1503);
    let before = format!("{}\x1b[?1049h\x1b[H", rows.join("\r\n"));
    let alternate = marked_rows(at, 0..3);
    let marks = alternate.join("\r\n");
    let fed = |limit| {
        let meter = Meter::watch();
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(before.as_bytes()).unwrap();
        model.feed(marks.as_bytes()).unwrap();
        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
        model
    };
    let bound = fed(ROOMY).footprint();

    for (limit, kept) in [(bound, 0..1500), (bound - 1, 500..1500)] {
        let mut model = fed(limit);
        assert_eq!(model.screen(), alternate);
        model.feed(b"\x1b[?1049l").unwrap();
        assert_eq!(model.history(), rows[kept]);
        assert_eq!(model.screen(), rows[1500..]);
    }
}

/// LIM-03 Contract (worked case from the #159 review): at 500x200 with
/// 64 MiB, a model whose 3,100 history rows fill four chunks refused every
/// restoration, because both models could not fit. The current model's
/// oldest history now yields first: staging is accepted at the bound with
/// every row, and one byte below it, or at 64 MiB, with the newest 3,000.
#[test]
fn staging_a_restoration_discards_the_current_models_oldest_history_first() {
    let at = geometry(500, 200);
    let rows = rows(0..3300);
    let output = rows.join("\r\n");
    let current = |limit| {
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(output.as_bytes()).unwrap();
        model
    };
    let bound = current(ROOMY).footprint() + needed(Model::new(at, limits(0, 10_000)));

    for (limit, kept) in [
        (bound, 0..3100),
        (bound - 1, 100..3100),
        (64 << 20, 100..3100),
    ] {
        let meter = Meter::watch();
        let mut model = current(limit);
        model.stage(at).unwrap();
        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
        assert_eq!(model.history(), rows[kept]);
        assert_eq!(model.screen(), rows[3100..]);
    }
}

/// LIM-03 Contract: staging never takes the current model's first history
/// chunk. A model at 80x24 with 3,100 history rows stages a 500x200
/// restoration that fits only beside that chunk: at the bound it is staged
/// and the newest 1,000 rows stay; one byte below it the staging is rejected
/// before anything is allocated, keeping the newest 1,000 rows and the page,
/// and the model is read-only.
#[test]
fn staging_never_takes_the_current_models_first_history_chunk() {
    let (small, large) = (geometry(80, 24), geometry(500, 200));
    let rows = rows(0..3124);
    let output = rows.join("\r\n");
    let current = |limit| {
        let mut model = Model::new(small, limits(limit, 10_000)).unwrap();
        model.feed(output.as_bytes()).unwrap();
        model
    };
    let mut first = current(ROOMY);
    while first.shorten_history() {}
    let bound = first.footprint() + needed(Model::new(large, limits(0, 10_000)));

    let meter = Meter::watch();
    let mut model = current(bound);
    model.stage(large).unwrap();
    assert!(meter.peak() <= bound, "{} > {bound}", meter.peak());
    assert_eq!(model.history(), rows[2100..3100]);

    let mut model = current(bound - 1);
    let rejected = Rejected {
        needed: bound,
        limit: bound - 1,
    };
    assert_eq!(model.stage(large), Err(rejected));
    assert_eq!(model.feed(b"x"), Err(rejected));
    assert_eq!(model.history(), rows[2100..3100]);
    assert_eq!(model.screen(), rows[3100..]);
}

/// LIM-03 Contract: while a restoration is staged, the current model's
/// history yields to it, since applying the restoration replaces the current
/// model whole. At 500x200, a restoration of 3,100 history rows staged
/// beside a current model with 3,100 rows keeps every row at the bound,
/// where only the current model's first chunk is left beside it; one byte
/// below it the restoration keeps its newest 3,000.
#[test]
fn a_staged_restoration_takes_room_from_the_current_models_history_first() {
    let at = geometry(500, 200);
    let rows = rows(0..3300);
    let restored: Vec<String> = (0..3300).map(|n| format!("new{n}")).collect();
    let (output, restoration) = (rows.join("\r\n"), restored.join("\r\n"));
    let mut first = Model::new(at, limits(ROOMY, 10_000)).unwrap();
    first.feed(output.as_bytes()).unwrap();
    while first.shorten_history() {}
    let mut alone = Model::new(at, limits(ROOMY, 10_000)).unwrap();
    alone.feed(restoration.as_bytes()).unwrap();
    let bound = first.footprint() + alone.footprint();

    for (limit, kept) in [(bound, 0..3100), (bound - 1, 100..3100)] {
        let meter = Meter::watch();
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(output.as_bytes()).unwrap();
        model.stage(at).unwrap();
        model.feed(restoration.as_bytes()).unwrap();
        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
        assert!(model.apply());
        assert_eq!(model.history(), restored[kept]);
        assert_eq!(model.screen(), restored[3100..]);
    }
}

/// LIM-03 Contract: the monitor can shorten a model's history under pressure
/// across sessions (#119): each call discards the oldest chunk and releases
/// its rows at once, while the row vector keeps the capacity ten chunks
/// took, so the allocator never holds more than the lower footprint; the
/// first chunk, which holds the protected newest page, is never discarded,
/// and a model without history has nothing to shorten.
#[test]
fn shortening_history_releases_the_oldest_chunk_and_never_the_first() {
    let at = geometry(4, 1);
    let rows: Vec<String> = (0..9500).map(|n| format!("{n:04}")).collect();
    let output = rows.join("\r\n");
    let meter = Meter::watch();
    let mut model = Model::new(at, limits(ROOMY, 10_000)).unwrap();
    model.feed(output.as_bytes()).unwrap();

    for chunks in (1..10).rev() {
        let footprint = model.footprint();
        assert!(model.shorten_history());
        assert!(model.footprint() < footprint);
        let (live, charged) = (meter.live(), model.footprint());
        assert!(live <= charged, "{chunks} chunks: {live} > {charged}");
        assert_eq!(model.history(), rows[9499 - chunks * 1000..9499]);
    }
    assert!(!model.shorten_history());
    assert_eq!(model.history(), rows[8499..9499]);
    assert_eq!(model.screen(), rows[9499..]);
    assert!(
        !Model::new(at, limits(ROOMY, NO_HISTORY))
            .unwrap()
            .shorten_history()
    );
}

/// LIM-03 Contract: a history chunk admitted ahead of output but not yet
/// filled yields without losing a row, and the engine then holds no row
/// beyond the lower limit. With 1,990 history rows and a third chunk
/// admitted for the rows to come, a marked cell is accepted at the bound
/// and one byte below it, where that chunk yields; every row stays in both.
/// Below the bound, marked cells then take most of the room the chunk left,
/// and the rows that follow fill the lower limit, the oldest going, with the
/// allocator never past the limit.
#[test]
fn an_unfilled_history_chunk_yields_without_losing_a_row() {
    let at = geometry(200, 60);
    let plain = rows(0..2149);
    let cell = "a\u{301}";
    let marked = cell.repeat(200);
    let before = format!("{}\r\n", plain[..2049].join("\r\n"));
    let cover: String = (1..=59)
        .map(|row| format!("\x1b[{row};1H{marked}"))
        .collect();
    let after = format!("\x1b[60;2H\r\n{}\r\n", plain[2049..].join("\r\n"));
    let lines = [
        &plain[..1990],
        &vec![marked; 59],
        &[cell.to_owned()],
        &plain[2049..],
    ]
    .concat();
    let updated = |limit| {
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(before.as_bytes()).unwrap();
        model.feed(cell.as_bytes()).unwrap();
        model
    };
    let bound = updated(ROOMY).footprint();
    assert_eq!(updated(bound).history(), plain[..1990]);
    assert_eq!(updated(bound - 1).history(), plain[..1990]);

    let meter = Meter::watch();
    let mut model = updated(bound - 1);
    model.feed(cover.as_bytes()).unwrap();
    model.feed(after.as_bytes()).unwrap();

    assert!(meter.peak() < bound, "{} >= {bound}", meter.peak());
    assert_eq!(model.history(), lines[91..2091]);
    assert_eq!(model.screen()[..59], lines[2091..]);
}

/// LIM-03 Contract: the alternate screen has no history of its own, and the
/// normal screen's history is not shown over it.
#[test]
fn the_alternate_screen_shows_no_history() {
    let mut model = Model::new(geometry(8, 2), limits(ROOMY, HISTORY)).unwrap();
    model.feed(b"one\r\ntwo\r\nthree\x1b[?1049h").unwrap();
    assert_eq!(model.history(), Vec::<String>::new());
    model.feed(b"\x1b[?1049l").unwrap();
    assert_eq!(model.history(), ["one"]);
}

/// A deterministic xorshift sequence, as the tokenizer tests use.
struct Rng(u64);

impl Rng {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a [u8]]) -> &'a [u8] {
        items[self.below(items.len())]
    }
}

/// The tokenizer tests' tricky bytes: controls the parser executes inside a
/// sequence, CAN and SUB, ESC, and bytes it ignores.
const TRICKY: &[&[u8]] = &[
    b"\x07", b"\x08", b"\n", b"\r", b"\x00", b"\x1c", b"\x1f", b"\x18", b"\x1a", b"\x1b", b"\x7f",
    b"\x9c",
];

/// Narrow, wide, combining and invalid text, combining marks most often.
const PRINTED: &[&[u8]] = &[
    b"a",
    b" ",
    "\u{e9}".as_bytes(),
    "\u{4e00}".as_bytes(),
    "\u{301}".as_bytes(),
    "\u{301}".as_bytes(),
    "\u{301}".as_bytes(),
    b"\xff",
    b"\xf0\x80",
];

/// CSI parameters and finals that print, tint, scroll, erase, switch
/// screens, push titles, query and negotiate.
const PARAMETERS: &[&[u8]] = &[
    b"",
    b"1",
    b"25",
    b"58;5;1",
    b"58:2::1:2:3",
    b"4:3",
    b"?1049",
    b"?47;25",
    b"?2026",
    b"22",
    b"3",
    b">1",
    b"=1;1",
];
const FINALS: &[&[u8]] = &[
    b"m", b"t", b"h", b"l", b"J", b"K", b"S", b"L", b"@", b"P", b"X", b"H", b"r", b"c", b"n", b"q",
    b"u",
];

/// Repeat counts for `CSI n b`: absent, zero, small, and past what `n`
/// can hold.
const COUNTS: &[&[u8]] = &[b"", b"0", b"1", b"25", b"8192", b"99999"];

/// Escape sequences after `ESC`.
const ESCAPES: &[&[u8]] = &[
    b"7", b"8", b"c", b"#8", b"D", b"E", b"M", b"H", b"(B", b"=", b"Z", b"\\",
];

/// Control strings after `ESC`, and what ends them.
const STRINGS: &[&[u8]] = &[
    b"]0;title",
    b"]8;;http://x/",
    b"]8;id=1;http://y/",
    b"]52;c;aGVsbG8=",
    b"]11;?",
    b"P$qm",
    b"_Gi=1;AAAA",
    b"^pm",
    b"Xsos",
];
const ENDS: &[&[u8]] = &[b"\x07", b"\x1b\\", b"\x18", b"\x1a", b"\x1b7", b"\x9c"];

/// Half the time, a tricky byte.
fn slip(rng: &mut Rng, out: &mut Vec<u8>) {
    if rng.below(2) == 0 {
        out.extend(rng.pick(TRICKY));
    }
}

/// `ESC`, perhaps a tricky byte, `[`, then `parameters` with perhaps a
/// tricky byte among them, and `last`.
fn csi(rng: &mut Rng, parameters: &[u8], last: &[u8], out: &mut Vec<u8>) {
    out.push(0x1b);
    slip(rng, out);
    out.push(b'[');
    let split = rng.below(parameters.len() + 1);
    out.extend(&parameters[..split]);
    slip(rng, out);
    out.extend(&parameters[split..]);
    out.extend(last);
}

/// One piece of an adversarial stream: text, a tricky byte, a repeat, or
/// another CSI, an escape or a control string, with tricky bytes slipped
/// after `ESC` and among parameters.
fn piece(rng: &mut Rng, out: &mut Vec<u8>) {
    match rng.below(6) {
        0 => (0..=rng.below(6)).for_each(|_| out.extend(rng.pick(PRINTED))),
        1 => out.extend(rng.pick(TRICKY)),
        2 => {
            let count = rng.pick(COUNTS);
            csi(rng, count, b"b", out);
        }
        3 => {
            let (parameters, last) = (rng.pick(PARAMETERS), rng.pick(FINALS));
            csi(rng, parameters, last, out);
        }
        4 => {
            out.push(0x1b);
            slip(rng, out);
            out.extend(rng.pick(ESCAPES));
        }
        _ => {
            out.push(0x1b);
            slip(rng, out);
            out.extend(rng.pick(STRINGS));
            out.extend(rng.pick(ENDS));
        }
    }
}

/// LIM-03 Fault: adversarial streams never take a model past its limit. They
/// mix narrow, wide, combining and invalid text with sequences that print,
/// repeat, tint, scroll, switch screens and carry strings, with the tokenizer
/// tests' tricky bytes slipped between `ESC` and the introducer and among
/// parameters, and arrive in reads of any size. A model with no history,
/// from its mandatory limit up, rejects before the allocator sees the limit
/// exceeded, and what it keeps between reads is charged: beyond the reserve,
/// only the front filter's held partial token and the cursor templates' cell
/// data, within `HELD`, go uncharged. After a rejection it keeps its page and
/// refuses everything.
#[test]
fn adversarial_streams_never_take_a_model_past_its_limit() {
    const HELD: usize = 16 << 10;
    let at = geometry(10, 3);
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    for _ in 0..50 {
        let mut stream = Vec::new();
        (0..40).for_each(|_| piece(&mut rng, &mut stream));
        for extra in [0, 4 << 10, 64 << 10, 1 << 20] {
            let limit = mandatory(at) + extra;
            let reads: Vec<usize> = (0..stream.len()).map(|_| 1 + rng.below(64)).collect();
            let meter = Meter::watch();
            let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();
            let mut rest = &stream[..];
            let mut sizes = reads.iter();
            let rejected = loop {
                let Some(&size) = sizes.next().filter(|_| !rest.is_empty()) else {
                    break None;
                };
                let (read, next) = rest.split_at(size.min(rest.len()));
                rest = next;
                if let Err(rejected) = model.feed(read) {
                    break Some(rejected);
                }
                let uncharged = meter
                    .live()
                    .saturating_sub(model.footprint() - Model::RESERVE);
                assert!(uncharged <= HELD, "{uncharged} uncharged: {stream:?}");
            };
            assert!(
                meter.peak() <= limit,
                "{} > {limit}: {stream:?}",
                meter.peak()
            );
            if let Some(rejected) = rejected {
                let page = model.screen();
                assert_eq!(model.feed(rest), Err(rejected));
                assert_eq!(model.feed(b"x"), Err(rejected));
                assert_eq!(model.screen(), page);
            }
        }
    }
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
    let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();
    model.feed(marked(at, 0).as_bytes()).unwrap();

    let accepted = (0..200)
        .take_while(|_| model.feed(round.as_bytes()).is_ok())
        .count();

    assert!(accepted > 0 && accepted < 200, "{accepted}");
    assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
}

/// LIM-03 Fault: repeated `CSI 65535 b` of a combining mark is rejected as
/// soon as one repetition would not fit, so marks never pile up past the
/// limit on one cell, wherever C0 controls sit in the sequence.
#[test]
fn repeated_repeats_stay_within_the_limit() {
    let at = geometry(10, 3);
    let limit = mandatory(at) + 2 * 1024 * 1024;
    for repeat in [
        &b"\x1b[65535b"[..],
        b"\x1b\x07[65535b",
        b"\x1b[\x0765535b",
        b"\x1b[6\x075535b",
    ] {
        let meter = Meter::watch();
        let mut model = Model::new(at, limits(limit, NO_HISTORY)).unwrap();
        model.feed("a\u{301}".as_bytes()).unwrap();

        let repeated = (0..20).take_while(|_| model.feed(repeat).is_ok()).count();

        assert_eq!(repeated, 0, "{repeat:?}");
        assert!(meter.peak() <= limit, "{} > {limit}", meter.peak());
    }
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

/// LIM-03 Contract: the engine sees exactly what the live output policy
/// forwards to a host. A hyperlink, which the policy strips, never reaches
/// it, so the text printed under it carries no extra data and costs
/// nothing; from `CSI ? 47;1049 h` the engine still sees the alternate
/// screen it implements.
#[test]
fn the_engine_sees_only_what_a_host_would_receive() {
    let mut model = Model::new(geometry(8, 2), limits(ROOMY, NO_HISTORY)).unwrap();
    let footprint = model.footprint();

    model
        .feed(b"\x1b]8;;https://example.com/\x07link\x1b]8;;\x07")
        .unwrap();
    assert_eq!(model.footprint(), footprint);
    assert_eq!(model.screen(), ["link", ""]);

    model.feed(b"\x1b[?47;1049h").unwrap();
    assert_eq!(model.screen(), ["", ""]);
}
