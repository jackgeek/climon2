//! The unchanged Alacritty engine behind a small private seam: feeding steps,
//! draining events, history capacity and reading grids.

use crate::geometry::Geometry;
use alacritty_terminal::{
    Term,
    event::{Event, EventListener},
    grid::Dimensions,
    term::{Config, Osc52, TermMode},
    vte::ansi::{Processor, Timeout},
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
struct Unbuffered;

impl Timeout for Unbuffered {
    fn set_timeout(&mut self, _: Duration) {}

    fn clear_timeout(&mut self) {}

    fn pending_timeout(&self) -> bool {
        false
    }
}

struct Size(Geometry);

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

fn config(history: usize) -> Config {
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
    title: Option<String>,
}

impl Engine {
    pub(crate) fn new(geometry: Geometry, history: usize) -> Self {
        let listener = Listener::default();
        let events = listener.0.clone();
        Self {
            term: Term::new(config(history), &Size(geometry), listener),
            processor: Processor::new(),
            events,
            title: None,
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
            other => self.retitle(other),
        }
    }

    fn retitle(&mut self, event: Event) {
        match event {
            Event::Title(title) => self.title = Some(title),
            Event::ResetTitle => self.title = None,
            _ => {}
        }
    }

    /// The application's title; `None` means the default title.
    pub(crate) fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// `ESC c` clears the engine's title without an event.
    pub(crate) fn reset_title(&mut self) {
        self.title = None;
    }

    pub(crate) fn mode(&self) -> TermMode {
        *self.term.mode()
    }
}
