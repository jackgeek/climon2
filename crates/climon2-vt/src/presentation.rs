use crate::{
    classify::private_modes, derive::Cache, geometry::Geometry, render::goto, token::Tokenizer,
};

const ALTERNATE_SCREEN: u16 = 1049;
const SYNC_UPDATE: u16 = 2026;
const END_SYNC: &[u8] = b"\x1b[?2026l";

/// The documented normal-shell baseline after the synchronized update and
/// alternate screen are left (plan section 1.7.1).
const BASELINE: &[u8] = b"\x1b[r\x1b[?6l\x1b[4l\x1b[20l\x1b[?7h\
\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[?2004l\
\x1b[?1l\x1b>\x1b[?25h\x1b[0 q\x1b[0m";

/// How live output reaches a controller's host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The host is exactly the session size: filtered tokens are forwarded.
    Direct,
    /// The host is larger than the session: output is rendered from the
    /// engine inside the session rectangle.
    Derived,
}

impl Mode {
    pub(crate) fn of(cols: u16, rows: u16, geometry: Geometry) -> Self {
        if cols > geometry.cols() || rows > geometry.rows() {
            Self::Derived
        } else {
            Self::Direct
        }
    }
}

/// One attachment's record of what its committed stream has put its host
/// into. The daemon owns one per attachment and keeps it current: every
/// byte string this crate returns for an attachment (`grant`, `produce`,
/// `derive`, `end_sync`, `release`) must be committed, and committed Direct
/// live output must be reported with [`crate::Vt::commit_live`].
#[derive(Debug)]
pub struct Presentation {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) mode: Mode,
    pub(crate) alternate: bool,
    pub(crate) sync_open: bool,
    pub(crate) cache: Cache,
    pub(crate) release_cursor: Option<(usize, usize)>,
    observer: Tokenizer,
}

impl Presentation {
    /// A host of this reported (unclamped) size, before anything was sent.
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            mode: Mode::Direct,
            alternate: false,
            sync_open: false,
            cache: Cache::default(),
            release_cursor: None,
            observer: Tokenizer::new(),
        }
    }

    /// Record a new reported host size ahead of a replacement grant, keeping
    /// what this attachment's stream already put its host into.
    pub fn resize_host(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
    }

    /// Presentation mode chosen at the latest grant or presentation resize.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Record committed Direct live output: it may enter or leave the
    /// alternate screen (`CSI ? 1049 h/l`) or open or end a synchronized
    /// update (`CSI ? 2026 h/l`).
    pub(crate) fn commit_live(&mut self, bytes: &[u8]) {
        for (set, modes) in self.observer.push(bytes).iter().filter_map(private_modes) {
            self.observe(set, &modes);
        }
    }

    fn observe(&mut self, set: bool, modes: &[u16]) {
        if modes.contains(&ALTERNATE_SCREEN) {
            self.alternate = set;
        }
        if modes.contains(&SYNC_UPDATE) {
            self.sync_open = set;
        }
    }

    /// End a synchronized update this host still has open (on the latch
    /// deadline or revocation); empty when none is open.
    pub fn end_sync(&mut self) -> Vec<u8> {
        let bytes = if self.sync_open { END_SYNC } else { b"" };
        self.sync_open = false;
        bytes.to_vec()
    }

    /// Bytes returning the host to the normal-shell baseline when this
    /// presentation ends. The alternate screen is left only if this
    /// attachment's stream entered it. The host title is not restored.
    pub fn release(&mut self) -> Vec<u8> {
        let mut bytes = END_SYNC.to_vec();
        if self.alternate {
            bytes.extend_from_slice(b"\x1b[?1049l");
        }
        bytes.extend_from_slice(BASELINE);
        if let Some((row, col)) = self.release_cursor {
            goto(&mut bytes, row, col);
        }
        self.alternate = false;
        self.sync_open = false;
        bytes
    }
}
