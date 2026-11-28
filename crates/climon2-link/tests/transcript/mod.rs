//! Protocol transcripts (REV-07, #498): the complete bytes of one exchange,
//! both directions, made with fixed test keys and fixed randomness, and the
//! replay that holds each end to them.
//!
//! A [`Scenario`] says how the exchange is made: the two ends, their
//! randomness and a script of [`Cue`]s, each a moment or something one
//! end's caller does. Recording runs both ends through the script and
//! writes down, in order, every moment, every caller's act, every batch of
//! bytes one end handed the other and every close. Replay runs one end
//! alone through the same script: it hands the end the other end's
//! recorded bytes and closes at the points they were recorded, and requires
//! the end's own bytes and close to be the recorded ones exactly.
//!
//! An end is stepped only at a moment of the script, after its own
//! caller's act, when bytes or a close arrive, and again at once while it
//! has sent bytes and named the present moment as its deadline. Recording
//! and replay follow that rule alike, so an end sees the same steps in
//! both, with no clock and no other end behind them.

// Each test binary compiles this module for itself and uses its own part.
#![allow(dead_code)]

use climon2_link::{End, HubEnd, Input, Randomness, RelayEnd, Time};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::Duration;

/// The folder that holds link revision `revision`'s transcripts.
pub fn folder(revision: u16) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("transcripts")
        .join(format!("revision-{revision}"))
}

/// The committed transcript called `name` of link revision `revision`.
pub fn load(revision: u16, name: &str) -> Transcript {
    let path = folder(revision).join(format!("{name}.txt"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{}: {error}; record a new one with `cargo test -p climon2-link --test transcripts -- --ignored`",
            path.display()
        )
    });
    Transcript::parse(&text)
}

/// Bytes per line of hex in a transcript file.
const PER_LINE: usize = 16;

/// One end of the byte stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Hub,
    Relay,
}

impl Side {
    fn word(self) -> &'static str {
        match self {
            Side::Hub => "hub",
            Side::Relay => "relay",
        }
    }

    fn parse(word: &str) -> Self {
        match word {
            "hub" => Side::Hub,
            "relay" => Side::Relay,
            other => panic!("no side is called {other:?}"),
        }
    }
}

/// One line of a transcript.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry {
    /// The time moved to this moment, and both ends were stepped.
    At(Duration),
    /// One end's caller did the act of this name, and the end was stepped.
    Acts(Side, String),
    /// One end's bytes, handed to the other end in one step.
    Sends(Side, Vec<u8>),
    /// One end closed its byte stream, and the other end was told.
    Closes(Side),
}

/// The entries of one exchange, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Transcript(pub Vec<Entry>);

impl Transcript {
    /// Reads a transcript file. Lines that start with `#` and blank lines
    /// are notes; an indented line holds hex bytes of the `sends` above it.
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        let mut declared = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if line.starts_with(char::is_whitespace) {
                let Some(Entry::Sends(_, bytes)) = entries.last_mut() else {
                    panic!("hex outside a sends entry: {line:?}");
                };
                bytes.extend(hex(trimmed));
                continue;
            }
            let words: Vec<&str> = trimmed.split_whitespace().collect();
            entries.push(match words.as_slice() {
                ["at", millis, "ms"] => At(Duration::from_millis(millis.parse().expect("ms"))),
                [side, "sends", count, "bytes"] => {
                    declared.push((entries.len(), count.parse::<usize>().expect("a count")));
                    Entry::Sends(Side::parse(side), Vec::new())
                }
                [side, "closes"] => Entry::Closes(Side::parse(side)),
                [side, "acts:", ..] => {
                    let name = trimmed.split_once("acts:").expect("acts").1.trim();
                    Entry::Acts(Side::parse(side), name.to_owned())
                }
                _ => panic!("not a transcript line: {line:?}"),
            });
        }
        for (at, count) in declared {
            let Entry::Sends(_, bytes) = &entries[at] else {
                unreachable!()
            };
            assert_eq!(bytes.len(), count, "entry {at} declares {count} bytes");
        }
        Self(entries)
    }

    /// The transcript file: `notes` as `#` lines, then one entry a line.
    pub fn render(&self, notes: &str) -> String {
        let mut text = String::new();
        for note in notes.lines() {
            writeln!(text, "{}", format!("# {note}").trim_end()).unwrap();
        }
        for entry in &self.0 {
            match entry {
                At(at) => writeln!(text, "at {} ms", at.as_millis()).unwrap(),
                Entry::Acts(side, name) => writeln!(text, "{} acts: {name}", side.word()).unwrap(),
                Entry::Closes(side) => writeln!(text, "{} closes", side.word()).unwrap(),
                Entry::Sends(side, bytes) => {
                    writeln!(text, "{} sends {} bytes", side.word(), bytes.len()).unwrap();
                    for line in bytes.chunks(PER_LINE) {
                        let digits: Vec<String> = line.iter().map(|b| format!("{b:02x}")).collect();
                        writeln!(text, "    {}", digits.join(" ")).unwrap();
                    }
                }
            }
        }
        text
    }

    /// Every byte `side` sent, in order.
    pub fn sent_by(&self, side: Side) -> Vec<u8> {
        let mut sent = Vec::new();
        for entry in &self.0 {
            if let Entry::Sends(by, bytes) = entry
                && *by == side
            {
                sent.extend(bytes);
            }
        }
        sent
    }
}

use Entry::At;

fn hex(text: &str) -> Vec<u8> {
    let text: String = text.split_whitespace().collect();
    assert_eq!(text.len() % 2, 0, "whole bytes: {text:?}");
    let digit = |at: usize| u8::from_str_radix(&text[at..at + 2], 16).expect("hex");
    (0..text.len()).step_by(2).map(digit).collect()
}

/// One cue of a scenario's script.
pub enum Cue {
    /// The time moves to this moment.
    At(Duration),
    /// The hub monitor's caller does the act of this name.
    Hub(&'static str, fn(&mut HubEnd)),
    /// The relay's caller does the act of this name.
    Relay(&'static str, fn(&mut RelayEnd)),
}

impl Cue {
    /// The entry this cue makes in a transcript.
    fn entry(&self) -> Entry {
        match self {
            Cue::At(at) => At(*at),
            Cue::Hub(name, _) => Entry::Acts(Side::Hub, (*name).to_owned()),
            Cue::Relay(name, _) => Entry::Acts(Side::Relay, (*name).to_owned()),
        }
    }
}

/// How one exchange is made.
pub struct Scenario {
    /// The file's name in the link revision's folder, without `.txt`.
    pub name: &'static str,
    /// What the exchange is, for the file's notes.
    pub notes: &'static str,
    pub hub: fn() -> HubEnd,
    pub relay: fn() -> RelayEnd,
    pub hub_randomness: fn() -> Box<dyn Randomness>,
    pub relay_randomness: fn() -> Box<dyn Randomness>,
    /// The script after the start, which is the moment zero.
    pub cues: Vec<Cue>,
}

impl Scenario {
    /// The script, its start included.
    fn script(&self) -> impl Iterator<Item = &Cue> {
        std::iter::once(&Cue::At(Duration::ZERO)).chain(&self.cues)
    }
}

/// One end as recording and replay hold it.
struct Seat<E> {
    end: E,
    randomness: Box<dyn Randomness>,
    /// What the end has sent and the other end has not been handed.
    sent: Vec<u8>,
    /// The end asked to close; it is stepped no more.
    closed: bool,
    /// The other end has been told of the close.
    told: bool,
}

impl<E: End> Seat<E> {
    fn new(end: E, randomness: Box<dyn Randomness>) -> Self {
        Self {
            end,
            randomness,
            sent: Vec::new(),
            closed: false,
            told: false,
        }
    }

    /// Steps the end at `now` with `input`, then with nothing for as long
    /// as it sends bytes and names `now` or earlier as its deadline.
    fn step(&mut self, now: Duration, input: Input<'_>) {
        let now = Time::since_start(now);
        let mut input = input;
        while !self.closed {
            let output = self.end.step(now, input, &mut *self.randomness);
            let more = !output.bytes.is_empty() && output.deadline.is_some_and(|at| at <= now);
            self.sent.extend(output.bytes);
            self.closed = output.close;
            if !more {
                return;
            }
            input = Input::Tick;
        }
    }
}

/// Hands the other end what `from` has sent, or else its close, and
/// writes it down. Whether there was anything to hand.
fn hand<E: End, F: End>(
    side: Side,
    from: &mut Seat<E>,
    to: &mut Seat<F>,
    now: Duration,
    log: &mut Vec<Entry>,
) -> bool {
    if !from.sent.is_empty() {
        let bytes = std::mem::take(&mut from.sent);
        to.step(now, Input::Bytes(&bytes));
        log.push(Entry::Sends(side, bytes));
        return true;
    }
    if from.closed && !from.told {
        from.told = true;
        to.step(now, Input::Closed);
        log.push(Entry::Closes(side));
        return true;
    }
    false
}

/// Runs both ends of `scenario` through its script and writes down the
/// exchange. Gives the transcript and the two ends as they finished.
pub fn record(scenario: &Scenario) -> (Transcript, HubEnd, RelayEnd) {
    let mut hub = Seat::new((scenario.hub)(), (scenario.hub_randomness)());
    let mut relay = Seat::new((scenario.relay)(), (scenario.relay_randomness)());
    let mut log = Vec::new();
    let mut now = Duration::ZERO;
    for cue in scenario.script() {
        log.push(cue.entry());
        match cue {
            Cue::At(at) => {
                now = *at;
                hub.step(now, Input::Tick);
                relay.step(now, Input::Tick);
            }
            Cue::Hub(_, act) => {
                act(&mut hub.end);
                hub.step(now, Input::Tick);
            }
            Cue::Relay(_, act) => {
                act(&mut relay.end);
                relay.step(now, Input::Tick);
            }
        }
        loop {
            let to_relay = hand(Side::Hub, &mut hub, &mut relay, now, &mut log);
            let to_hub = hand(Side::Relay, &mut relay, &mut hub, now, &mut log);
            if !to_relay && !to_hub {
                break;
            }
        }
    }
    (Transcript(log), hub.end, relay.end)
}

/// Replays `transcript` to the hub monitor's end of `scenario`: see
/// [`replay`].
pub fn replay_hub(scenario: &Scenario, transcript: &Transcript) -> HubEnd {
    let seat = Seat::new((scenario.hub)(), (scenario.hub_randomness)());
    replay(Side::Hub, seat, scenario, transcript, |cue, end| {
        if let Cue::Hub(_, act) = cue {
            act(end);
        }
    })
}

/// Replays `transcript` to the relay's end of `scenario`: see [`replay`].
pub fn replay_relay(scenario: &Scenario, transcript: &Transcript) -> RelayEnd {
    let seat = Seat::new((scenario.relay)(), (scenario.relay_randomness)());
    replay(Side::Relay, seat, scenario, transcript, |cue, end| {
        if let Cue::Relay(_, act) = cue {
            act(end);
        }
    })
}

/// Runs `side`'s end alone through the script of `scenario`, handing it
/// the other end's bytes and close where `transcript` holds them. Panics
/// unless the end sends exactly its own recorded bytes, each batch before
/// the entries recorded after it, and closes exactly when it was recorded
/// to. Gives the end as it finished.
fn replay<E: End>(
    side: Side,
    mut seat: Seat<E>,
    scenario: &Scenario,
    transcript: &Transcript,
    act: impl Fn(&Cue, &mut E),
) -> E {
    let mut script = scenario.script();
    let mut now = Duration::ZERO;
    for (at, entry) in transcript.0.iter().enumerate() {
        let place = format!(
            "{} entry {at}, replayed to the {}",
            scenario.name,
            side.word()
        );
        match entry {
            At(_) | Entry::Acts(..) => {
                let cue = script
                    .next()
                    .unwrap_or_else(|| panic!("{place}: no cue left"));
                assert_eq!(&cue.entry(), entry, "{place}: the script's next cue");
                if let Cue::At(moment) = cue {
                    now = *moment;
                }
                let own = matches!(entry, Entry::Acts(by, _) if *by == side);
                act(cue, &mut seat.end);
                if own || matches!(cue, Cue::At(_)) {
                    seat.step(now, Input::Tick);
                }
            }
            Entry::Sends(by, bytes) if *by == side => {
                let sent = seat.sent.get(..bytes.len());
                assert_eq!(sent, Some(&bytes[..]), "{place}: the end's own bytes");
                seat.sent.drain(..bytes.len());
            }
            Entry::Sends(_, bytes) => seat.step(now, Input::Bytes(bytes)),
            Entry::Closes(by) if *by == side => {
                assert!(
                    seat.closed && seat.sent.is_empty(),
                    "{place}: the end closes"
                );
                seat.told = true;
            }
            Entry::Closes(_) => seat.step(now, Input::Closed),
        }
    }
    assert!(script.next().is_none(), "{}: cues left", scenario.name);
    assert!(
        seat.sent.is_empty(),
        "{}: the {} sent more",
        scenario.name,
        side.word()
    );
    assert_eq!(
        seat.closed,
        seat.told,
        "{}: the {} closed",
        scenario.name,
        side.word()
    );
    seat.end
}
