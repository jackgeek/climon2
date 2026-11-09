//! The unchanged Alacritty engine behind a small private seam: feeding steps,
//! draining events, history capacity and reading grids.

use crate::geometry::Geometry;
use alacritty_terminal::{
    Grid, Term,
    event::{Event, EventListener},
    grid::Dimensions,
    term::{Config, Osc52, TermMode, cell::Cell},
    vte::ansi::{CursorStyle, Processor, Timeout},
};
use std::{cell::RefCell, rc::Rc, time::Duration};

#[derive(Clone, Default)]
struct Listener(Rc<RefCell<Vec<Event>>>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        self.0.borrow_mut().push(event);
    }
}

/// Never pending, so the engine never buffers a synchronized update; the
/// adapter's own first-begin latch owns synchronized output.
#[derive(Default)]
pub(crate) struct Unbuffered;

impl Timeout for Unbuffered {
    fn set_timeout(&mut self, _: Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}

pub(crate) struct Size(pub(crate) Geometry);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.screen_lines()
    }

    fn screen_lines(&self) -> usize {
        usize::from(self.0.rows())
    }

    fn columns(&self) -> usize {
        usize::from(self.0.cols())
    }
}

pub(crate) fn config(history: usize) -> Config {
    Config {
        scrolling_history: history,
        kitty_keyboard: false,
        osc52: Osc52::Disabled,
        ..Config::default()
    }
}

pub(crate) struct Engine {
    term: Term<Listener>,
    processor: Processor<Unbuffered>,
    events: Rc<RefCell<Vec<Event>>>,
    signals: Signals,
}

/// What the engine reported through events: the application's title
/// (`None` for the default) and how many bells rang.
#[derive(Default)]
struct Signals {
    title: Option<String>,
    bells: u64,
}

impl Engine {
    pub(crate) fn new(geometry: Geometry, history: usize) -> Self {
        let listener = Listener::default();
        let events = listener.0.clone();
        Self {
            term: Term::new(config(history), &Size(geometry), listener),
            processor: Processor::new(),
            events,
            signals: Signals::default(),
        }
    }

    /// Feed one step and collect the replies it produced.
    pub(crate) fn advance(&mut self, bytes: &[u8], replies: &mut Vec<u8>) {
        self.processor.advance(&mut self.term, bytes);
        let events = std::mem::take(&mut *self.events.borrow_mut());
        events
            .into_iter()
            .for_each(|event| self.event(event, replies));
    }

    fn event(&mut self, event: Event, replies: &mut Vec<u8>) {
        match event {
            Event::PtyWrite(text) => replies.extend_from_slice(text.as_bytes()),
            Event::Bell => self.signals.bells += 1,
            other => self.retitle(other),
        }
    }

    fn retitle(&mut self, event: Event) {
        match event {
            Event::Title(title) => self.signals.title = Some(title),
            Event::ResetTitle => self.signals.title = None,
            _ => {}
        }
    }

    /// The application's title; `None` means the default title.
    pub(crate) fn title(&self) -> Option<&str> {
        self.signals.title.as_deref()
    }

    pub(crate) fn bells(&self) -> u64 {
        self.signals.bells
    }

    /// `ESC c` clears the engine's title without an event.
    pub(crate) fn reset_title(&mut self) {
        self.signals.title = None;
    }

    pub(crate) fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    /// Change the normal screen's history capacity, dropping its oldest rows
    /// beyond it. The title event `set_options` always emits is discarded.
    pub(crate) fn set_history(&mut self, rows: usize) {
        self.term.set_options(config(rows));
        self.events.borrow_mut().clear();
    }

    pub(crate) fn resize(&mut self, geometry: Geometry) {
        self.term.resize(Size(geometry));
    }

    pub(crate) fn alternate(&self) -> bool {
        self.mode().contains(TermMode::ALT_SCREEN)
    }

    /// History rows the normal screen holds, when it is the active screen.
    pub(crate) fn normal_history(&self) -> Option<usize> {
        (!self.alternate()).then(|| self.term.grid().history_size())
    }

    pub(crate) fn cursor_style(&self) -> CursorStyle {
        self.term.cursor_style()
    }

    pub(crate) fn active(&self) -> &Grid<Cell> {
        self.term.grid()
    }

    /// Read the normal screen's grid. While the alternate screen is active
    /// this swaps screens and back, then restores the alternate grid exactly;
    /// the swap sets the normal saved cursor to its current cursor (E1).
    pub(crate) fn normal<R>(&mut self, read: impl FnOnce(&Grid<Cell>) -> R) -> R {
        if !self.alternate() {
            return read(self.term.grid());
        }
        let alternate = self.term.grid().clone();
        self.term.swap_alt();
        let result = read(self.term.grid());
        self.term.swap_alt();
        *self.term.grid_mut() = alternate;
        result
    }
}
