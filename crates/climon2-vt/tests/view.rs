//! The read-only views a monitor paints a session from (#83), observed
//! through the public interface. Every expectation is written by hand from
//! the sequences' definitions (ECMA-48, DEC and xterm), never taken from
//! another run of the model.

use climon2_vt::{
    Geometry, Limits, Model, Rejected,
    view::{
        Ansi, Cell, Colour, Content, Cursor, CursorShape, Glyph, History, Modes, MouseEncoding,
        MouseReporting, Rendition, Rows, Screen, Underline, View,
    },
};

/// Room for everything these tests print.
const ROOMY: usize = 1 << 30;

/// History rows the models here keep at most.
const HISTORY: usize = 100;

fn geometry(cols: u16, rows: u16) -> Geometry {
    Geometry::for_host(cols, rows).unwrap()
}

fn limits(bytes: usize, history_rows: usize) -> Limits {
    Limits {
        bytes,
        history_rows,
    }
}

/// The bytes a model of `at` keeping `history_rows` needs before any
/// output.
fn mandatory(at: Geometry, history_rows: usize) -> usize {
    match Model::new(at, limits(0, history_rows)) {
        Ok(_) => panic!("a model needs some bytes"),
        Err(Rejected { needed, .. }) => needed,
    }
}

fn model(cols: u16, rows: u16) -> Model {
    Model::new(geometry(cols, rows), limits(ROOMY, HISTORY)).unwrap()
}

/// A model of `cols` x `rows` that has been fed `bytes`.
fn fed(cols: u16, rows: u16, bytes: &str) -> Model {
    let mut model = model(cols, rows);
    model.feed(bytes.as_bytes()).unwrap();
    model
}

/// Each row's cells, in order.
fn cells(rows: Rows<'_>) -> Vec<Vec<Cell<'_>>> {
    rows.iter().map(|row| row.cells().collect()).collect()
}

/// The active screen's cells, top row first.
fn screen<'a>(view: &View<'a>) -> Vec<Vec<Cell<'a>>> {
    let (Screen::Normal { rows, .. } | Screen::Alternate { rows }) = view.screen();
    cells(rows)
}

/// `text` in plain narrow cells, then blanks to `cols` columns.
fn row(text: &str, cols: usize) -> Vec<Cell<'static>> {
    let mut row: Vec<Cell<'static>> = text.chars().map(narrow).collect();
    row.resize(cols, blank());
    row
}

/// A cell holding `content` in the default colours and rendition.
fn cell(content: Content<'static>) -> Cell<'static> {
    Cell {
        content,
        foreground: Colour::Default,
        background: Colour::Default,
        underline: Colour::Default,
        rendition: Rendition::default(),
    }
}

fn narrow(character: char) -> Cell<'static> {
    marked(character, &[])
}

fn marked(character: char, combining: &'static [char]) -> Cell<'static> {
    cell(Content::Narrow(Glyph {
        character,
        combining,
    }))
}

fn wide(character: char, combining: &'static [char]) -> Cell<'static> {
    cell(Content::Wide(Glyph {
        character,
        combining,
    }))
}

fn spacer() -> Cell<'static> {
    cell(Content::Spacer)
}

fn blank() -> Cell<'static> {
    narrow(' ')
}

/// TRM-01 (crate case): wide and combining text. A combining mark joins the
/// character printed before it, in that character's cell, wide or narrow; a
/// wide character fills its cell and the next, whose spacer has nothing of
/// its own to draw; a wide character that no longer fits at the right
/// margin moves to the next row and leaves the last column blank.
#[test]
fn wide_and_combining_characters_fill_the_cells_they_cover() {
    let model = fed(6, 3, "e\u{301}\u{4e00}\u{302}x\r\nabcde\u{4e8c}");

    assert_eq!(
        screen(&model.view()),
        [
            vec![
                marked('e', &['\u{301}']),
                wide('\u{4e00}', &['\u{302}']),
                spacer(),
                narrow('x'),
                blank(),
                blank(),
            ],
            vec![
                narrow('a'),
                narrow('b'),
                narrow('c'),
                narrow('d'),
                narrow('e'),
                blank(),
            ],
            vec![
                wide('\u{4e8c}', &[]),
                spacer(),
                blank(),
                blank(),
                blank(),
                blank(),
            ],
        ]
    );
}

/// `character` in `foreground` on `background`, underlined in `underline`.
fn coloured(
    character: char,
    foreground: Colour,
    background: Colour,
    underline: Colour,
) -> Cell<'static> {
    Cell {
        foreground,
        background,
        underline,
        ..narrow(character)
    }
}

/// TRM-01 (crate case): colours. SGR selects the foreground (30-37, 90-97,
/// 38), background (40-47, 100-107, 48) and underline (58) colours from the
/// sixteen ANSI colours, the 256-colour palette (`;5;n`) or direct colour
/// (`;2;r;g;b`, or ITU T.416's `:2::r:g:b`); 39, 49, 59 and 0 return to the
/// defaults.
#[test]
fn cells_keep_the_colours_sgr_selected() {
    let model = fed(
        8,
        2,
        "\x1b[31ma\x1b[42mb\x1b[39;49mc\x1b[91;104md\
         \x1b[38;5;200;48;5;17me\x1b[38;2;1;2;3;48;2;4;5;6mf\
         \x1b[0;58;5;9mg\x1b[58:2::7:8:9mh\r\n\x1b[59mi\x1b[0mj",
    );

    let red = Colour::Ansi(Ansi::Red);
    let default = Colour::Default;
    assert_eq!(
        screen(&model.view()),
        [
            vec![
                coloured('a', red, default, default),
                coloured('b', red, Colour::Ansi(Ansi::Green), default),
                coloured('c', default, default, default),
                coloured(
                    'd',
                    Colour::Ansi(Ansi::BrightRed),
                    Colour::Ansi(Ansi::BrightBlue),
                    default
                ),
                coloured('e', Colour::Indexed(200), Colour::Indexed(17), default),
                coloured('f', Colour::Rgb(1, 2, 3), Colour::Rgb(4, 5, 6), default),
                coloured('g', default, default, Colour::Indexed(9)),
                coloured('h', default, default, Colour::Rgb(7, 8, 9)),
            ],
            [vec![narrow('i'), narrow('j')], vec![blank(); 6]].concat(),
        ]
    );
}

/// TRM-01 (crate case): SGR 30-37 select the eight ANSI colours, black, red,
/// green, yellow, blue, magenta, cyan and white, and SGR 90-97 their bright
/// variants.
#[test]
fn sgr_selects_each_of_the_sixteen_ansi_colours() {
    let text: String = (30..=37)
        .chain(90..=97)
        .map(|sgr| format!("\x1b[{sgr}mx"))
        .collect();
    let model = fed(16, 1, &text);

    let foregrounds: Vec<Colour> = screen(&model.view())[0]
        .iter()
        .map(|cell| cell.foreground)
        .collect();
    assert_eq!(
        foregrounds,
        [
            Ansi::Black,
            Ansi::Red,
            Ansi::Green,
            Ansi::Yellow,
            Ansi::Blue,
            Ansi::Magenta,
            Ansi::Cyan,
            Ansi::White,
            Ansi::BrightBlack,
            Ansi::BrightRed,
            Ansi::BrightGreen,
            Ansi::BrightYellow,
            Ansi::BrightBlue,
            Ansi::BrightMagenta,
            Ansi::BrightCyan,
            Ansi::BrightWhite,
        ]
        .map(Colour::Ansi)
    );
}

/// `character` drawn in `rendition`.
fn styled(character: char, rendition: Rendition) -> Cell<'static> {
    Cell {
        rendition,
        ..narrow(character)
    }
}

/// TRM-01 (crate case): rendition. SGR 1 is bold, 2 faint, 3 italic, 4
/// underlined (4:2 doubly, 4:3 curly, 4:4 dotted, 4:5 dashed), 7 inverse, 8
/// concealed and 9 crossed out; 22 cancels bold and faint, 23 italic, 24 and
/// 4:0 the underline, 27 inverse, 28 concealment, 29 crossing out, and 0
/// everything.
#[test]
fn cells_keep_the_rendition_sgr_selected() {
    let model = fed(
        12,
        2,
        "\x1b[1ma\x1b[22;2mb\x1b[22;3mc\x1b[23;4md\x1b[4:2me\x1b[4:3mf\
         \x1b[4:4mg\x1b[4:5mh\x1b[24;7mi\x1b[27;8mj\x1b[28;9mk\x1b[1;3;4;29ml\
         \r\n\x1b[4:0mm\x1b[0mn",
    );

    let plain = Rendition::default();
    let underlined = |underline| Rendition {
        underline: Some(underline),
        ..plain
    };
    let bold_italic = Rendition {
        bold: true,
        italic: true,
        ..plain
    };
    assert_eq!(
        screen(&model.view()),
        [
            vec![
                styled(
                    'a',
                    Rendition {
                        bold: true,
                        ..plain
                    }
                ),
                styled('b', Rendition { dim: true, ..plain }),
                styled(
                    'c',
                    Rendition {
                        italic: true,
                        ..plain
                    }
                ),
                styled('d', underlined(Underline::Single)),
                styled('e', underlined(Underline::Double)),
                styled('f', underlined(Underline::Curly)),
                styled('g', underlined(Underline::Dotted)),
                styled('h', underlined(Underline::Dashed)),
                styled(
                    'i',
                    Rendition {
                        inverse: true,
                        ..plain
                    }
                ),
                styled(
                    'j',
                    Rendition {
                        hidden: true,
                        ..plain
                    }
                ),
                styled(
                    'k',
                    Rendition {
                        strikeout: true,
                        ..plain
                    }
                ),
                styled(
                    'l',
                    Rendition {
                        underline: Some(Underline::Single),
                        ..bold_italic
                    }
                ),
            ],
            [
                vec![styled('m', bold_italic), narrow('n')],
                vec![blank(); 10]
            ]
            .concat(),
        ]
    );
}

/// TRM-01 (crate case): a horizontal tab moves the cursor to the next tab
/// stop, every eighth column, and draws nothing in the cells it passes.
#[test]
fn a_tab_leaves_the_cells_it_passes_blank() {
    let model = fed(10, 1, "a\tb");

    assert_eq!(
        screen(&model.view()),
        [[
            vec![narrow('a')],
            vec![blank(); 7],
            vec![narrow('b'), blank()]
        ]
        .concat()]
    );
}

/// The cursor of a fresh model: home, visible, in the host's default shape
/// and not asked to blink.
const HOME: Cursor = Cursor {
    row: 0,
    column: 0,
    visible: true,
    shape: CursorShape::Default,
    blinking: false,
};

/// TRM-01 (crate case): cursor position. CUP (`CSI row;column H`, counting
/// from 1) moves the cursor, which the view reports counting from 0; after a
/// character is printed in the last column the cursor stays there until the
/// next one wraps.
#[test]
fn the_cursor_is_where_the_application_moved_it() {
    let cursor = |bytes| fed(5, 3, bytes).view().cursor();

    assert_eq!(cursor(""), HOME);
    assert_eq!(
        cursor("\x1b[2;4H"),
        Cursor {
            row: 1,
            column: 3,
            ..HOME
        }
    );
    assert_eq!(
        cursor("\x1b[3;1Habcde"),
        Cursor {
            row: 2,
            column: 4,
            ..HOME
        }
    );
}

/// TRM-01 (crate case): cursor visibility, shape and blink. DECTCEM
/// (`CSI ? 25 l` and `h`) hides and shows the cursor. DECSCUSR
/// (`CSI Ps SP q`) selects a block (Ps 1, 2), an underline (3, 4) or a bar
/// (5, 6), blinking for odd Ps and steady for even; Ps 0 returns to the
/// host's default shape, as RIS (`ESC c`) does, and is told apart from an
/// explicit steady block, also once history has grown. `CSI ? 12 h` and `l`
/// start and stop blinking without changing the shape, the default one
/// included.
#[test]
fn the_cursor_has_the_visibility_shape_and_blink_the_application_selected() {
    let cursor = |bytes| fed(5, 3, bytes).view().cursor();
    let style = |bytes| {
        let cursor = cursor(bytes);
        (cursor.shape, cursor.blinking)
    };

    assert!(!cursor("\x1b[?25l").visible);
    assert!(cursor("\x1b[?25l\x1b[?25h").visible);
    for (bytes, shape, blinking) in [
        ("\x1b[1 q", CursorShape::Block, true),
        ("\x1b[3 q\x1b[2 q", CursorShape::Block, false),
        ("\x1b[3 q", CursorShape::Underline, true),
        ("\x1b[4 q", CursorShape::Underline, false),
        ("\x1b[5 q", CursorShape::Bar, true),
        ("\x1b[6 q", CursorShape::Bar, false),
        ("\x1b[5 q\x1b[0 q", CursorShape::Default, false),
        ("\x1b[3 q\x1bc", CursorShape::Default, false),
        ("\x1b[4 q\x1b[?12h", CursorShape::Underline, true),
        ("\x1b[5 q\x1b[?12l", CursorShape::Bar, false),
        ("\x1b[?12h", CursorShape::Default, true),
        ("\x1b[?12h\x1b[?12l", CursorShape::Default, false),
    ] {
        assert_eq!(style(bytes), (shape, blinking), "{bytes:?}");
    }
    let past_a_history_chunk = "\n".repeat(1100);
    assert_eq!(style(&past_a_history_chunk), (CursorShape::Default, false));
}

/// TRM-01 (crate case): the 1049 alternate screen. `CSI ? 1049 h` saves the
/// cursor and switches to the alternate screen, cleared; what scrolls off
/// its top is gone. `CSI ? 1049 l` returns to the normal screen as it was
/// and restores the cursor.
#[test]
fn the_alternate_screen_replaces_the_normal_screen_until_it_is_left() {
    let mut model = model(4, 2);
    model.feed(b"one\r\ntwo\r\nab\x1b[?1049h").unwrap();

    let view = model.view();
    let Screen::Alternate { rows } = view.screen() else {
        panic!("the alternate screen is active");
    };
    assert_eq!(cells(rows), [row("", 4), row("", 4)]);
    assert_eq!(
        view.cursor(),
        Cursor {
            row: 1,
            column: 2,
            ..HOME
        }
    );

    model.feed(b"\x1b[Hxy\r\nzw\r\nv").unwrap();
    assert_eq!(screen(&model.view()), [row("zw", 4), row("v", 4)]);

    model.feed(b"\x1b[?1049l").unwrap();
    let view = model.view();
    let Screen::Normal { rows, .. } = view.screen() else {
        panic!("the normal screen is active");
    };
    assert_eq!(cells(rows), [row("two", 4), row("ab", 4)]);
    assert_eq!(
        view.cursor(),
        Cursor {
            row: 1,
            column: 2,
            ..HOME
        }
    );
}

/// The modes of a fresh model: no input mode is set, and alternate scroll is
/// on, as Alacritty starts.
const FRESH: Modes = Modes {
    application_cursor_keys: false,
    application_keypad: false,
    bracketed_paste: false,
    mouse_reporting: MouseReporting::Off,
    mouse_encoding: MouseEncoding::Default,
    focus_reporting: false,
    alternate_scroll: true,
};

/// The modes a model is left in by `bytes`.
fn modes(bytes: &str) -> Modes {
    fed(5, 2, bytes).view().modes()
}

/// Each `(bytes, modes)` pair: the modes `bytes` leave a fresh model in.
#[track_caller]
fn assert_modes(cases: &[(&str, Modes)]) {
    assert_eq!(modes(""), FRESH);
    for (bytes, expected) in cases {
        assert_eq!(modes(bytes), *expected, "{bytes:?}");
    }
}

/// INP-02 (crate case): cursor-key and keypad modes. DECCKM (`CSI ? 1 h`
/// and `l`) switches the cursor keys between application and normal
/// sequences, and DECKPAM (`ESC =`) and DECKPNM (`ESC >`) the keypad.
#[test]
fn cursor_key_and_keypad_modes_follow_decckm_and_deckpam() {
    assert_modes(&[
        (
            "\x1b[?1h",
            Modes {
                application_cursor_keys: true,
                ..FRESH
            },
        ),
        ("\x1b[?1h\x1b[?1l", FRESH),
        (
            "\x1b=",
            Modes {
                application_keypad: true,
                ..FRESH
            },
        ),
        ("\x1b=\x1b>", FRESH),
    ]);
}

/// INP-03 (crate case): bracketed paste. `CSI ? 2004 h` asks for pastes to
/// be bracketed and `CSI ? 2004 l` stops it.
#[test]
fn bracketed_paste_follows_mode_2004() {
    assert_modes(&[
        (
            "\x1b[?2004h",
            Modes {
                bracketed_paste: true,
                ..FRESH
            },
        ),
        ("\x1b[?2004h\x1b[?2004l", FRESH),
    ]);
}

/// INP-05 (crate case): mouse reporting and encoding. `CSI ? 1000 h` reports
/// button presses and releases, 1002 also motion while a button is held, and
/// 1003 all motion; the latest of them wins and resetting it stops
/// reporting. `CSI ? 1005 h` encodes reports in UTF-8 and 1006 as SGR; the
/// latest wins, and resetting it returns to the default encoding.
#[test]
fn mouse_reporting_and_encoding_follow_their_modes() {
    let reporting = |mouse_reporting| Modes {
        mouse_reporting,
        ..FRESH
    };
    let encoding = |mouse_encoding| Modes {
        mouse_encoding,
        ..FRESH
    };
    assert_modes(&[
        ("\x1b[?1000h", reporting(MouseReporting::Clicks)),
        ("\x1b[?1002h", reporting(MouseReporting::Drag)),
        ("\x1b[?1003h", reporting(MouseReporting::Motion)),
        ("\x1b[?1003h\x1b[?1000h", reporting(MouseReporting::Clicks)),
        ("\x1b[?1000h\x1b[?1002h", reporting(MouseReporting::Drag)),
        ("\x1b[?1000h\x1b[?1003h", reporting(MouseReporting::Motion)),
        ("\x1b[?1002h\x1b[?1002l", FRESH),
        ("\x1b[?1005h", encoding(MouseEncoding::Utf8)),
        ("\x1b[?1006h", encoding(MouseEncoding::Sgr)),
        ("\x1b[?1006h\x1b[?1005h", encoding(MouseEncoding::Utf8)),
        ("\x1b[?1005h\x1b[?1006h", encoding(MouseEncoding::Sgr)),
        ("\x1b[?1006h\x1b[?1006l", FRESH),
        (
            "\x1b[?1002;1006h",
            Modes {
                mouse_reporting: MouseReporting::Drag,
                mouse_encoding: MouseEncoding::Sgr,
                ..FRESH
            },
        ),
    ]);
}

/// FOC-01 (crate case): focus reporting. `CSI ? 1004 h` asks for focus in
/// and out to be reported and `CSI ? 1004 l` stops it.
#[test]
fn focus_reporting_follows_mode_1004() {
    assert_modes(&[
        (
            "\x1b[?1004h",
            Modes {
                focus_reporting: true,
                ..FRESH
            },
        ),
        ("\x1b[?1004h\x1b[?1004l", FRESH),
    ]);
}

/// TRM-05 (crate case): alternate scroll. `CSI ? 1007 l` stops the wheel
/// sending cursor keys on the alternate screen and `CSI ? 1007 h` restores
/// it.
#[test]
fn alternate_scroll_follows_mode_1007() {
    let off = Modes {
        alternate_scroll: false,
        ..FRESH
    };
    assert_modes(&[("\x1b[?1007l", off), ("\x1b[?1007l\x1b[?1007h", FRESH)]);
}

/// The normal screen's history, or a panic on the alternate screen.
fn normal_history<'a>(view: &View<'a>) -> History<'a> {
    match view.screen() {
        Screen::Normal { history, .. } => history,
        Screen::Alternate { .. } => panic!("the normal screen is active"),
    }
}

/// TRM-05 (crate case): normal history. Rows scrolled off the top of the
/// normal screen are its history, oldest first, every cell as it was drawn;
/// nothing has been discarded from it.
#[test]
fn rows_scrolled_off_the_normal_screen_are_its_history() {
    let model = fed(4, 2, "a\r\n\x1b[31mb\x1b[0m\r\nc\r\nd");

    let view = model.view();
    let history = normal_history(&view);
    let red = coloured(
        'b',
        Colour::Ansi(Ansi::Red),
        Colour::Default,
        Colour::Default,
    );
    let (a, b) = (row("a", 4), [vec![red], vec![blank(); 3]].concat());
    assert_eq!(cells(history.rows), [a.clone(), b.clone()]);
    assert!(!history.truncated);
    assert_eq!(history.rows.len(), 2);
    assert!(!history.rows.is_empty());
    let at = |index| {
        history
            .rows
            .get(index)
            .map(|row| row.cells().collect::<Vec<_>>())
    };
    assert_eq!((at(0), at(1), at(2)), (Some(a), Some(b), None));
    assert_eq!(screen(&view), [row("c", 4), row("d", 4)]);

    let model = fed(4, 2, "a\r\nb");
    let empty = normal_history(&model.view());
    assert!(empty.rows.is_empty());
    assert_eq!(empty.rows.len(), 0);
}

/// Lines `row<n>` for each `n` in `numbers`, as printed and as cells
/// `cols` wide.
fn lines(numbers: std::ops::Range<usize>, cols: usize) -> (String, Vec<Vec<Cell<'static>>>) {
    let text: Vec<String> = numbers.map(|number| format!("row{number}")).collect();
    let cells = text.iter().map(|text| row(text, cols)).collect();
    (text.join("\r\n"), cells)
}

/// TRM-05, LIM-03 (crate cases): history truncation at the row limit. A
/// model keeping 100 history rows shows every row scrolled off, complete,
/// when exactly 100 have been; one row past it the oldest is discarded and
/// the history says so.
#[test]
fn history_is_truncated_one_row_past_its_limit() {
    for (scrolled, kept, truncated) in [
        (HISTORY, 0..HISTORY, false),
        (HISTORY + 1, 1..HISTORY + 1, true),
    ] {
        let (text, cells_of) = lines(0..scrolled + 2, 8);
        let model = fed(8, 2, &text);

        let view = model.view();
        let history = normal_history(&view);
        assert_eq!(cells(history.rows), cells_of[kept], "{scrolled} rows");
        assert_eq!(history.truncated, truncated, "{scrolled} rows");
        assert_eq!(screen(&view), cells_of[scrolled..]);
    }
}

/// TRM-05 (crate case): clearing history. ED 3 (`CSI 3 J`) erases the
/// history and RIS (`ESC c`) resets the whole terminal: either way the
/// history starts again empty, with nothing discarded from it, and rows
/// scrolled off later join it.
#[test]
fn cleared_history_starts_again_with_nothing_discarded() {
    let (text, cells_of) = lines(0..HISTORY + 3, 8);
    for (clear, later) in [
        (
            "\x1b[3J",
            vec![
                cells_of[HISTORY + 1].clone(),
                cells_of[HISTORY + 2].clone(),
                row("", 8),
            ],
        ),
        ("\x1bc", vec![row("", 8), row("", 8)]),
    ] {
        let mut model = fed(8, 2, &text);
        assert!(normal_history(&model.view()).truncated);

        model.feed(clear.as_bytes()).unwrap();
        let history = normal_history(&model.view());
        assert!(history.rows.is_empty(), "{clear:?}");
        assert!(!history.truncated, "{clear:?}");

        model.feed(b"\r\n\r\n\r\n").unwrap();
        let history = normal_history(&model.view());
        assert_eq!(cells(history.rows), later, "{clear:?}");
        assert!(!history.truncated, "{clear:?}");
    }
}

/// LIM-03, TRM-05 (crate cases): history the allocation bound stops growing.
/// History grows in the engine's 1000-row chunks while they fit. A model
/// keeping up to 2000 rows whose limit admits the second chunk keeps every
/// row scrolled off, complete; one byte below that limit the chunk is
/// refused, the oldest row is discarded instead, and the history says so.
#[test]
fn history_the_bound_stops_growing_is_truncated() {
    let at = geometry(8, 2);
    let (text, cells_of) = lines(0..1003, 8);
    let fed_within = |bytes| {
        let mut model = Model::new(at, limits(bytes, 2000)).unwrap();
        model.feed(text.as_bytes()).unwrap();
        model
    };
    let bound = fed_within(ROOMY).footprint();

    for (limit, kept, truncated) in [(bound, 0..1001, false), (bound - 1, 1..1001, true)] {
        let model = fed_within(limit);
        let view = model.view();
        let history = normal_history(&view);
        assert_eq!(cells(history.rows), cells_of[kept], "limit {limit}");
        assert_eq!(history.truncated, truncated, "limit {limit}");
        assert_eq!(screen(&view), cells_of[1001..]);
    }
}

/// LIM-03, TRM-05 (crate cases): history shortened under pressure (#119).
/// Discarding the oldest chunk of a history that reaches into its second
/// discards rows, and from then on the history says so, while later rows
/// join it. A chunk admitted ahead of rows that never came holds none, so
/// discarding it loses nothing.
#[test]
fn shortened_history_is_truncated_only_when_rows_are_discarded() {
    let at = geometry(80, 2);
    let shortened = |scrolled: usize| {
        let (text, cells_of) = lines(0..scrolled + 2, 80);
        let mut model = Model::new(at, limits(ROOMY, 10_000)).unwrap();
        model.feed(text.as_bytes()).unwrap();
        assert!(model.shorten_history(), "{scrolled} rows");
        (model, cells_of)
    };

    let (model, cells_of) = shortened(1000);
    let history = normal_history(&model.view());
    assert_eq!(cells(history.rows), cells_of[..1000]);
    assert!(!history.truncated);

    let (mut model, _) = shortened(1500);
    let history = normal_history(&model.view());
    let (_, cells_of) = lines(0..1600, 80);
    assert_eq!(cells(history.rows), cells_of[500..1500]);
    assert!(history.truncated);

    let (more, _) = lines(1502..1600, 80);
    model.feed(format!("\r\n{more}").as_bytes()).unwrap();
    let history = normal_history(&model.view());
    assert_eq!(cells(history.rows), cells_of[500..1598]);
    assert!(history.truncated);
}

/// LIM-03, TRM-05 (crate cases): a restoration replaces history whole.
/// Staging one beside a model takes room from that model's history first:
/// within the bound every row stays and nothing is discarded; one byte
/// below it the oldest chunk's rows go and the history says so. Applying the
/// restoration replaces the model and its truncated history with the
/// restored ones, complete.
#[test]
fn a_restoration_replaces_a_truncated_history_whole() {
    let at = geometry(80, 24);
    let (text, cells_of) = lines(0..1524, 80);
    let current = |limit| {
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(text.as_bytes()).unwrap();
        model
    };
    let bound = current(ROOMY).footprint() + mandatory(at, 10_000);

    for (limit, kept, truncated) in [(bound, 0..1500, false), (bound - 1, 500..1500, true)] {
        let mut model = current(limit);
        model.stage(at).unwrap();
        let history = normal_history(&model.view());
        assert_eq!(cells(history.rows), cells_of[kept], "limit {limit}");
        assert_eq!(history.truncated, truncated, "limit {limit}");
    }

    let mut model = current(bound - 1);
    model.stage(at).unwrap();
    let (restored, cells_of) = lines(2000..2030, 80);
    model.feed(restored.as_bytes()).unwrap();
    assert!(model.apply());
    let view = model.view();
    let history = normal_history(&view);
    assert_eq!(cells(history.rows), cells_of[..6]);
    assert!(!history.truncated);
    assert_eq!(screen(&view), cells_of[6..]);
}

/// TRM-05, LIM-03 (crate cases): history behind the alternate screen. The
/// alternate screen shows no history; leaving it shows the normal screen's
/// history again with its truncation state, including rows discarded while
/// the alternate screen was active because its cells needed their room.
#[test]
fn the_alternate_screen_shows_no_history_until_it_is_left() {
    let at = geometry(10, 3);
    let (text, cells_of) = lines(0..1503, 10);
    let marks = format!("\x1b[?1049h\x1b[H{}", "a\u{301}".repeat(30));
    let fed_within = |limit| {
        let mut model = Model::new(at, limits(limit, 10_000)).unwrap();
        model.feed(text.as_bytes()).unwrap();
        model.feed(marks.as_bytes()).unwrap();
        model
    };
    let bound = fed_within(ROOMY).footprint();

    for (limit, kept, truncated) in [(bound, 0..1500, false), (bound - 1, 500..1500, true)] {
        let mut model = fed_within(limit);
        assert!(matches!(model.view().screen(), Screen::Alternate { .. }));

        model.feed(b"\x1b[?1049l").unwrap();
        let view = model.view();
        let history = normal_history(&view);
        assert_eq!(cells(history.rows), cells_of[kept], "limit {limit}");
        assert_eq!(history.truncated, truncated, "limit {limit}");
        assert_eq!(screen(&view), cells_of[1500..]);
    }
}

/// A model of `cols` x `rows` keeping `history_rows`, fed lines `row0`
/// onwards one at a time until `scrolled` have scrolled off the top of a full
/// screen, and every line's cells. One short line at a time never admits a
/// history chunk ahead of the rows to come.
fn scrolled_line_by_line(
    cols: u16,
    rows: u16,
    history_rows: usize,
    scrolled: usize,
) -> (Model, Vec<Vec<Cell<'static>>>) {
    let total = scrolled + usize::from(rows);
    let mut model = Model::new(geometry(cols, rows), limits(ROOMY, history_rows)).unwrap();
    for number in 0..total {
        let end = if number + 1 < total { "\r\n" } else { "" };
        model.feed(format!("row{number}{end}").as_bytes()).unwrap();
    }
    (model, lines(0..total, usize::from(cols)).1)
}

/// TRM-05, LIM-03 (crate cases): output that exactly fills the history
/// chunks admitted for it discards nothing. On an 80x24 screen, SU 24
/// (`CSI 24 S`) over 976 history rows, ED 2 (`CSI 2 J`) over as many, which
/// Alacritty keeps by scrolling the screen's rows into history, and 64 line
/// feeds at the bottom over 936 rows each bring the history to exactly its
/// first 1000-row chunk: every row is kept and none was discarded, and the
/// next row joins a complete history.
#[test]
fn output_filling_the_admitted_history_exactly_discards_nothing() {
    for (scrolled, output) in [
        (976, "\x1b[24S".to_owned()),
        (976, "\x1b[2J".to_owned()),
        (936, "\n".repeat(64)),
    ] {
        let (mut model, cells_of) = scrolled_line_by_line(80, 24, 10_000, scrolled);
        let mut expected = cells_of.clone();
        expected.resize(1000, row("", 80));

        model.feed(output.as_bytes()).unwrap();
        let history = normal_history(&model.view());
        assert_eq!(cells(history.rows), expected, "{output:?}");
        assert!(!history.truncated, "{output:?}");

        model.feed(b"\r\n").unwrap();
        let history = normal_history(&model.view());
        assert_eq!(history.rows.len(), 1001, "{output:?}");
        assert!(!history.truncated, "{output:?}");
    }
}

/// TRM-05, LIM-03 (crate cases): a history full at its row limit. A model
/// keeping 1000 rows that SU 24 fills exactly has discarded nothing, and
/// switching to the alternate screen and back pushes nothing. Full, though,
/// it loses its oldest row to every row pushed without the engine reporting
/// it, so once further output may have pushed one the history reports
/// truncation, even if none was (the owner's decision on #83, 2026-11-13).
#[test]
fn a_history_full_at_its_limit_reports_truncation_once_output_could_push_a_row_out() {
    let (mut model, cells_of) = scrolled_line_by_line(80, 24, 1000, 976);
    let complete = |model: &Model| {
        let history = normal_history(&model.view());
        assert_eq!(cells(history.rows), cells_of);
        !history.truncated
    };

    model.feed(b"\x1b[24S").unwrap();
    assert!(complete(&model));
    model.feed(b"\x1b[?1049h\x1b[?1049l").unwrap();
    assert!(complete(&model));
    model.feed(b"x").unwrap();
    assert!(!complete(&model));
}
