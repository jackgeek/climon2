use crate::{
    classify::{Clear, Live, classify},
    engine::Engine,
    geometry::{Geometry, STEP},
    sync::Latch,
    title,
    token::{Kind, Token, Tokenizer},
};
use std::time::Instant;

/// Session settings fixed at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VtConfig {
    /// Retained normal-screen history rows (`scrollback.max_rows`).
    pub max_history_rows: u32,
    /// Shown when the application has not set a title.
    pub default_title: String,
}

/// The result of feeding PTY output.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Fed {
    /// Filtered output for a Direct controller, in order.
    pub live: Vec<u8>,
    /// The engine's answers to terminal queries, for the PTY.
    pub replies: Vec<u8>,
    /// Oversized tokens dropped from both engine and output; log their count
    /// without content.
    pub dropped: u32,
}

/// The session's virtual terminal: the front filter, the engine and the
/// adapter state around it. Every operation is infallible.
pub struct Vt {
    engine: Engine,
    tokenizer: Tokenizer,
    latch: Latch,
    default_title: String,
}

impl Vt {
    pub fn new(geometry: Geometry, config: VtConfig) -> Self {
        Self {
            engine: Engine::new(geometry, config.max_history_rows as usize),
            tokenizer: Tokenizer::new(),
            latch: Latch::default(),
            default_title: config.default_title,
        }
    }

    /// Feed PTY output through the live output policy. Every completed
    /// token updates the engine; a partial token waits for a later read.
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Fed {
        let mut fed = Fed::default();
        for token in self.tokenizer.push(bytes) {
            self.token(&token, now, &mut fed);
        }
        fed
    }

    /// True while a synchronized update is open and before its deadline;
    /// snapshots and derived updates wait for it.
    pub fn sync_pending(&self, now: Instant) -> bool {
        self.latch.pending(now)
    }

    /// Close a synchronized update whose deadline passed. True when this
    /// call closed it, so the daemon ends any update open on a host.
    pub fn tick(&mut self, now: Instant) -> bool {
        self.latch.tick(now)
    }

    /// Whether the application enabled bracketed paste (mode 2004).
    pub fn bracketed_paste(&self) -> bool {
        self.engine
            .mode()
            .contains(alacritty_terminal::term::TermMode::BRACKETED_PASTE)
    }

    fn token(&mut self, token: &Token, now: Instant, fed: &mut Fed) {
        fed.dropped += u32::from(token.kind == Kind::Oversized);
        let policy = classify(token);
        let title = self.engine.title().map(str::to_owned);
        self.execute(&token.bytes, policy.clear, &mut fed.replies);
        self.latch.observe(policy.sync, now);
        fed.live.extend_from_slice(live(token, &policy.live));
        if self.engine.title() != title.as_deref() {
            fed.live.extend(self.title());
        }
    }

    fn execute(&mut self, bytes: &[u8], clear: Clear, replies: &mut Vec<u8>) {
        for step in bytes.chunks(STEP) {
            self.engine.advance(step, replies);
        }
        if clear == Clear::Reset {
            self.engine.reset_title();
        }
    }

    fn title(&self) -> Vec<u8> {
        title::osc(self.engine.title().unwrap_or(&self.default_title))
    }
}

fn live<'a>(token: &'a Token, live: &'a Live) -> &'a [u8] {
    match live {
        Live::Forward => &token.bytes,
        Live::Strip => &[],
        Live::Replace(bytes) => bytes,
    }
}
