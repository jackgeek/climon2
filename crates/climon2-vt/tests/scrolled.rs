//! The normal screen's viewport, scrolled back into history (#112),
//! observed through the public interface. Every expectation is written by
//! hand from the sequences' definitions, never taken from another run of
//! the model.

use climon2_vt::{
    Geometry, Limits, Model, Parser,
    view::{Screen, View},
};

/// Room for everything these tests print.
const ROOMY: usize = 1 << 30;

fn model(history_rows: usize) -> Model {
    let limits = Limits {
        bytes: ROOMY,
        history_rows,
    };
    Model::new(Geometry::for_host(8, 2).unwrap(), limits).unwrap()
}

fn feed(model: &mut Model, bytes: &str) {
    model.feed(&mut Parser::new(), bytes.as_bytes()).unwrap();
}

/// `row<first>` to `row<last>`, one per line, the last not ended.
fn rows(first: usize, last: usize) -> String {
    let rows: Vec<_> = (first..=last).map(|row| format!("row{row}")).collect();
    rows.join("\r\n")
}

/// A model keeping `history_rows` that printed `row0` to `row<last>` on
/// its two rows, so every row but the last two is history.
fn printed(history_rows: usize, last: usize) -> Model {
    let mut model = model(history_rows);
    feed(&mut model, &rows(0, last));
    model
}

/// How far the view is scrolled back, and whether its history is
/// truncated.
fn scrolled(model: &Model) -> (usize, bool) {
    let view = model.view();
    (view.scrolled(), truncated(&view))
}

fn truncated(view: &View<'_>) -> bool {
    match view.screen() {
        Screen::Normal { history, .. } => history.truncated,
        Screen::Alternate { .. } => false,
    }
}

/// TRM-05 (crate case): a live model is scrolled back nowhere. Scrolling
/// back moves into history a row at a time and stops at its oldest row;
/// scrolling forward stops at the live screen, and back to live returns
/// there from anywhere. With nothing in history there is nowhere to go.
#[test]
fn scrolling_stays_within_history_and_live() {
    let mut model = printed(100, 6);
    assert_eq!(scrolled(&model), (0, false));
    model.scroll(1);
    assert_eq!(model.view().scrolled(), 1);
    model.scroll(3);
    assert_eq!(model.view().scrolled(), 4);
    model.scroll(1);
    assert_eq!(model.view().scrolled(), 5, "the oldest row is the limit");
    model.scroll(1);
    assert_eq!(model.view().scrolled(), 5, "one past it");
    model.scroll(-2);
    assert_eq!(model.view().scrolled(), 3);
    model.scroll(-3);
    assert_eq!(model.view().scrolled(), 0);
    model.scroll(-1);
    assert_eq!(model.view().scrolled(), 0, "never past live");
    model.scroll(2);
    model.return_live();
    assert_eq!(model.view().scrolled(), 0);

    let mut empty = printed(100, 1);
    empty.scroll(1);
    assert_eq!(empty.view().scrolled(), 0);
}

/// TRM-05 (crate case): the viewport stays anchored while output
/// continues. Each row output pushes into history moves a scrolled view
/// back by one, so it keeps showing the same rows, whether the rows come
/// one at a time, several in one read or by wrapping; output that pushes
/// none (a scroll region below the top row, the cursor moving) moves
/// nothing, and a live view stays live.
#[test]
fn a_scrolled_view_stays_anchored_while_output_continues() {
    let mut model = printed(100, 6);
    model.scroll(2);
    feed(&mut model, "\r\nrow7");
    assert_eq!(model.view().scrolled(), 3);
    feed(&mut model, "\r\nrow8\r\nrow9\r\nrow10");
    assert_eq!(model.view().scrolled(), 6);
    feed(&mut model, "\r\n12345678abcdefgh");
    assert_eq!(model.view().scrolled(), 8, "a wrap pushes a row too");
    feed(&mut model, "\x1b[1;1Hx\x1b[2;8H");
    assert_eq!(model.view().scrolled(), 8, "the cursor moving");

    let limits = Limits {
        bytes: ROOMY,
        history_rows: 100,
    };
    let mut tall = Model::new(Geometry::for_host(8, 4).unwrap(), limits).unwrap();
    feed(&mut tall, &rows(0, 8));
    tall.scroll(2);
    feed(&mut tall, "\x1b[2;4r\x1b[4;1H\r\n\r\n\x1b[r");
    assert_eq!(tall.view().scrolled(), 2, "a region below the top row");
    feed(&mut tall, "\x1b[4;1H\r\n");
    assert_eq!(tall.view().scrolled(), 3);

    let mut live = printed(100, 6);
    feed(&mut live, "\r\nrow7\r\nrow8");
    assert_eq!(live.view().scrolled(), 0);
}

/// TRM-05 (crate case): eviction under a scrolled view. A model showing
/// three history rows, scrolled to its second oldest, stays anchored when
/// one more row comes: that row is now the oldest. One row more evicts it,
/// so the view moves to the oldest retained row with the truncation notice
/// above it: one row further back than the oldest row. Later output keeps
/// it there, never moving it to live, and scrolling forward leaves it.
#[test]
fn eviction_moves_the_view_to_the_notice_above_the_oldest_row() {
    let mut model = printed(3, 4);
    assert_eq!(scrolled(&model), (0, false));
    model.scroll(2);
    feed(&mut model, "\r\nrow5");
    assert_eq!(scrolled(&model), (3, true));
    feed(&mut model, "\r\nrow6");
    assert_eq!(scrolled(&model), (4, true));
    feed(&mut model, "\r\nrow7\r\nrow8");
    assert_eq!(scrolled(&model), (4, true));
    model.scroll(1);
    assert_eq!(scrolled(&model), (4, true), "the notice is the limit");
    model.scroll(-1);
    assert_eq!(scrolled(&model), (3, true));
}

/// TRM-05 (crate case): clearing history (`CSI 3 J`) under a scrolled view
/// leaves nothing to show but the live screen, and so does a full reset.
#[test]
fn clearing_history_returns_a_scrolled_view_return_live() {
    for clear in ["\x1b[3J", "\x1bc"] {
        let mut model = printed(100, 6);
        model.scroll(3);
        feed(&mut model, clear);
        assert_eq!(model.view().scrolled(), 0, "{clear:?}");
    }
}

/// TRM-05 (crate case): the alternate screen never shows normal history.
/// While it is active the view is not scrolled and scrolling does nothing;
/// leaving it returns the normal screen's view where it was, anchored
/// through no output there, since the alternate screen pushes no rows.
#[test]
fn the_alternate_screen_is_never_scrolled() {
    let mut model = printed(100, 6);
    model.scroll(2);
    feed(&mut model, "\x1b[?1049h");
    assert_eq!(model.view().scrolled(), 0);
    model.scroll(1);
    feed(&mut model, "\r\n\r\n\r\nalt");
    assert_eq!(model.view().scrolled(), 0);
    feed(&mut model, "\x1b[?1049l");
    assert_eq!(model.view().scrolled(), 2);
    feed(&mut model, "\r\nrow7");
    assert_eq!(model.view().scrolled(), 3, "counting resumes exactly");
}

/// TRM-05, LIM-03 (crate cases): history shortened under pressure beneath a
/// scrolled view. A view anchored in the rows the oldest chunk held moves
/// to the oldest retained row with the notice above it; one anchored in
/// rows that stay keeps its place.
#[test]
fn shortening_history_beneath_a_scrolled_view_moves_it_to_the_notice() {
    let shortened = |back: usize| {
        let mut model = model(10_000);
        feed(&mut model, &rows(0, 1501));
        model.scroll(isize::try_from(back).unwrap());
        assert!(model.shorten_history());
        scrolled(&model)
    };
    assert_eq!(shortened(1000), (1000, true));
    assert_eq!(shortened(1001), (1001, true));
    assert_eq!(shortened(1500), (1001, true));
}

/// TRM-04, TRM-05 (crate cases): a restoration replaces the model whole, so
/// once applied its view is live.
#[test]
fn an_applied_restoration_is_live() {
    let mut model = printed(100, 6);
    model.scroll(2);
    model.stage(Geometry::for_host(8, 2).unwrap()).unwrap();
    feed(&mut model, &rows(0, 6));
    assert_eq!(model.view().scrolled(), 2, "the current model is shown");
    assert!(model.apply());
    assert_eq!(model.view().scrolled(), 0);
}

/// TRM-01 (crate case): synchronized updates. The view says an update is
/// open from its begin to its end, whatever else is printed meanwhile; a
/// second begin changes nothing, and a full reset ends one.
#[test]
fn a_synchronized_update_is_open_from_its_begin_to_its_end() {
    let mut model = printed(100, 1);
    assert!(!model.view().synchronizing());
    for (bytes, open) in [
        ("\x1b[?2026h", true),
        ("half\r\ndrawn", true),
        ("\x1b[?2026h", true),
        ("\x1b[?2026l", false),
        ("more", false),
        ("\x1b[?2026l", false),
        ("\x1b[?2026h", true),
        ("\x1bc", false),
    ] {
        feed(&mut model, bytes);
        assert_eq!(model.view().synchronizing(), open, "{bytes:?}");
    }
}
