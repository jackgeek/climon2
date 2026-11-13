//! The read-only views a monitor paints a session from (#83), observed
//! through the public interface. Every expectation is written by hand from
//! the sequences' definitions (ECMA-48, DEC and xterm), never taken from
//! another run of the model.

use climon2_vt::{
    Geometry, Limits, Model,
    view::{
        Ansi, Cell, Colour, Content, Cursor, CursorShape, Glyph, Modes, MouseEncoding,
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

/// The cursor of a fresh model: home, visible, a steady block.
const HOME: Cursor = Cursor {
    row: 0,
    column: 0,
    visible: true,
    shape: CursorShape::Block,
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
/// (5, 6), blinking for odd Ps and steady for even, and Ps 0 the default, a
/// steady block; `CSI ? 12 h` and `l` start and stop blinking without
/// changing the shape.
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
        ("\x1b[5 q\x1b[0 q", CursorShape::Block, false),
        ("\x1b[4 q\x1b[?12h", CursorShape::Underline, true),
        ("\x1b[5 q\x1b[?12l", CursorShape::Bar, false),
    ] {
        assert_eq!(style(bytes), (shape, blinking), "{bytes:?}");
    }
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
