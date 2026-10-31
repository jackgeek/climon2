//! THROWAWAY: public-API forwarding/transition qualification, not a renderer.

pub mod input;
pub mod timing;

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::{Term, TermMode};
use cursor_icon::CursorIcon;
use vte::ansi::*;

pub struct Observed<'a, T> {
    pub term: &'a mut Term<T>,
    pub transitions: Vec<Transition>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct View {
    pub rows: Vec<Vec<Cell>>,
    pub history_rows: usize,
    pub cursor: (i32, usize),
    pub mode: TermMode,
}

impl View {
    pub fn text(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.iter().map(|cell| cell.c).collect())
            .collect()
    }
}

pub fn view<T>(term: &Term<T>) -> View {
    let grid = term.grid();
    View {
        rows: (-(grid.history_size() as i32)..grid.screen_lines() as i32)
            .map(|line| {
                (0..grid.columns())
                    .map(|column| grid[Line(line)][Column(column)].clone())
                    .collect()
            })
            .collect(),
        history_rows: grid.history_size(),
        cursor: (grid.cursor.point.line.0, grid.cursor.point.column.0),
        mode: *term.mode(),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Transition {
    NormalBeforeAlternate(View),
    NormalExposed(View),
}

impl<'a, T> Observed<'a, T> {
    pub fn new(term: &'a mut Term<T>) -> Self {
        Self {
            term,
            transitions: Vec::new(),
        }
    }

    fn is_alternate(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    fn observe_normal_exposure(&mut self, was_alternate: bool) {
        if was_alternate && !self.is_alternate() {
            self.transitions
                .push(Transition::NormalExposed(view(self.term)));
        }
    }
}

macro_rules! forward {
    ($(fn $name:ident($($arg:ident: $ty:ty),*);)*) => {
        $(fn $name(&mut self, $($arg: $ty),*) {
            Handler::$name(self.term, $($arg),*);
        })*
    };
}

impl<T: EventListener> Handler for Observed<'_, T> {
    forward! {
        fn set_title(title: Option<String>);
        fn set_cursor_style(style: Option<CursorStyle>);
        fn set_cursor_shape(shape: CursorShape);
        fn input(c: char);
        fn goto(line: i32, column: usize);
        fn goto_line(line: i32);
        fn goto_col(column: usize);
        fn insert_blank(count: usize);
        fn move_up(count: usize);
        fn move_down(count: usize);
        fn identify_terminal(intermediate: Option<char>);
        fn device_status(status: usize);
        fn move_forward(columns: usize);
        fn move_backward(columns: usize);
        fn move_down_and_cr(rows: usize);
        fn move_up_and_cr(rows: usize);
        fn put_tab(count: u16);
        fn backspace();
        fn carriage_return();
        fn linefeed();
        fn bell();
        fn substitute();
        fn newline();
        fn set_horizontal_tabstop();
        fn scroll_up(count: usize);
        fn scroll_down(count: usize);
        fn insert_blank_lines(count: usize);
        fn delete_lines(count: usize);
        fn erase_chars(count: usize);
        fn delete_chars(count: usize);
        fn move_backward_tabs(count: u16);
        fn move_forward_tabs(count: u16);
        fn save_cursor_position();
        fn restore_cursor_position();
        fn clear_line(mode: LineClearMode);
        fn clear_screen(mode: ClearMode);
        fn clear_tabs(mode: TabulationClearMode);
        fn set_tabs(interval: u16);
        fn reverse_index();
        fn terminal_attribute(attr: Attr);
        fn set_mode(mode: Mode);
        fn unset_mode(mode: Mode);
        fn report_mode(mode: Mode);
        fn report_private_mode(mode: PrivateMode);
        fn set_scrolling_region(top: usize, bottom: Option<usize>);
        fn set_keypad_application_mode();
        fn unset_keypad_application_mode();
        fn set_active_charset(index: CharsetIndex);
        fn configure_charset(index: CharsetIndex, charset: StandardCharset);
        fn set_color(index: usize, color: Rgb);
        fn dynamic_color_sequence(prefix: String, index: usize, terminator: &str);
        fn reset_color(index: usize);
        fn clipboard_store(clipboard: u8, text: &[u8]);
        fn clipboard_load(clipboard: u8, terminator: &str);
        fn decaln();
        fn push_title();
        fn pop_title();
        fn text_area_size_pixels();
        fn text_area_size_chars();
        fn set_hyperlink(link: Option<Hyperlink>);
        fn set_mouse_cursor_icon(icon: CursorIcon);
        fn report_keyboard_mode();
        fn push_keyboard_mode(mode: KeyboardModes);
        fn pop_keyboard_modes(count: u16);
        fn set_keyboard_mode(mode: KeyboardModes, behavior: KeyboardModesApplyBehavior);
        fn set_modify_other_keys(mode: ModifyOtherKeys);
        fn report_modify_other_keys();
        fn set_scp(path: ScpCharPath, mode: ScpUpdateMode);
    }

    fn set_private_mode(&mut self, mode: PrivateMode) {
        if mode == PrivateMode::Named(NamedPrivateMode::SwapScreenAndSetRestoreCursor)
            && !self.is_alternate()
        {
            self.transitions
                .push(Transition::NormalBeforeAlternate(view(self.term)));
        }
        Handler::set_private_mode(self.term, mode);
    }

    fn unset_private_mode(&mut self, mode: PrivateMode) {
        let was_alternate = self.is_alternate();
        Handler::unset_private_mode(self.term, mode);
        self.observe_normal_exposure(was_alternate);
    }

    fn reset_state(&mut self) {
        let was_alternate = self.is_alternate();
        Handler::reset_state(self.term);
        self.observe_normal_exposure(was_alternate);
    }
}
