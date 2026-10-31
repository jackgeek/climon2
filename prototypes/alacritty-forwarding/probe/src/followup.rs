//! THROWAWAY #48: scheduler/public-cell seam under qualification.
use alacritty_terminal::term::Term;
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::test::TermSize;

pub fn select_shared_link<T>(term: &mut Term<T>, link: Hyperlink) {
    term.grid_mut().cursor.template.set_hyperlink(Some(link));
}

pub struct PtyBuffer {
    bytes: Vec<u8>,
    cap: usize,
}
impl PtyBuffer {
    pub fn new(cap: usize) -> Self {
        Self {
            bytes: Vec::new(),
            cap,
        }
    }
    pub fn push(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        if bytes.len() > self.cap - self.bytes.len() {
            return Err("PTY buffer full");
        }
        self.bytes.extend(bytes);
        Ok(())
    }
    pub fn drain(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }
    pub fn remaining(&self) -> usize {
        self.cap - self.bytes.len()
    }
}

#[derive(Default)]
pub struct Resizes {
    pending: Option<(usize, usize)>,
    count: usize,
}
impl Resizes {
    pub fn request(&mut self, cols: usize, rows: usize) {
        self.pending = Some((cols, rows));
    }
    pub fn apply<T>(&mut self, term: &mut Term<T>) -> bool {
        let Some((cols, rows)) = self.pending.take() else {
            return false;
        };
        term.resize(TermSize::new(cols, rows));
        self.count += 1;
        true
    }
    pub fn reflows(&self) -> usize {
        self.count
    }
}
