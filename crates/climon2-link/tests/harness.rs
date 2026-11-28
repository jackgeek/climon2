//! The Link-layer harness itself (#492): two ends, the hub monitor's and the
//! relay's, joined by in-memory byte streams, with the test supplying the
//! time and the randomness. The ends here are stand-ins that write down what
//! reaches them; the link code's own ends are driven the same way.

mod support;

use climon2_link::{End, Input, Output, Randomness, Time};
use std::time::Duration;
use support::{Harness, Seeded};

/// What one step handed an end.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Tick,
    Bytes(Vec<u8>),
    Closed,
}

type Behaviour = Box<dyn FnMut(&Seen, &mut dyn Randomness) -> Output>;

/// A stand-in end: it writes down the time and input of every step and
/// answers as its behaviour says.
struct Recorder {
    seen: Vec<(Duration, Seen)>,
    behaviour: Behaviour,
}

impl Recorder {
    fn new(behaviour: impl FnMut(&Seen, &mut dyn Randomness) -> Output + 'static) -> Self {
        Self {
            seen: Vec::new(),
            behaviour: Box::new(behaviour),
        }
    }

    /// An end that never sends and never closes.
    fn silent() -> Self {
        Self::new(|_, _| Output::default())
    }

    /// An end that sends `bytes` at its first step and nothing afterwards.
    fn opening_with(bytes: &'static [u8]) -> Self {
        let mut opening = Some(bytes);
        Self::new(move |_, _| send(opening.take().unwrap_or_default()))
    }

    /// Every byte that reached this end, in order.
    fn received(&self) -> Vec<u8> {
        let bytes = |(_, seen): &(Duration, Seen)| match seen {
            Seen::Bytes(bytes) => bytes.clone(),
            _ => Vec::new(),
        };
        self.seen.iter().flat_map(bytes).collect()
    }
}

impl End for Recorder {
    fn step(&mut self, now: Time, input: Input<'_>, randomness: &mut dyn Randomness) -> Output {
        let seen = match input {
            Input::Tick => Seen::Tick,
            Input::Bytes(bytes) => Seen::Bytes(bytes.to_vec()),
            Input::Closed => Seen::Closed,
        };
        let output = (self.behaviour)(&seen, randomness);
        self.seen.push((now.elapsed(), seen));
        output
    }
}

fn send(bytes: &[u8]) -> Output {
    Output {
        bytes: bytes.to_vec(),
        close: false,
    }
}

const SECOND: Duration = Duration::from_secs(1);

/// Both roles are driven: what either end sends reaches the other end, whole
/// and in order, and the harness shows what each end put on its byte stream.
#[test]
fn bytes_one_end_sends_reach_the_other_in_order() {
    let hub = Recorder::opening_with(b"from the hub monitor");
    // The relay answers the first bytes it gets, as a listener does.
    let mut answer = Some(b"from the relay");
    let relay = Recorder::new(move |seen, _| match seen {
        Seen::Bytes(_) => send(answer.take().map_or(&[][..], |answer| answer)),
        _ => Output::default(),
    });
    let mut harness = Harness::new(hub, relay);

    harness.run();

    assert_eq!(harness.relay.received(), b"from the hub monitor");
    assert_eq!(harness.hub.received(), b"from the relay");
    assert_eq!(harness.sent_by_hub(), b"from the hub monitor");
    assert_eq!(harness.sent_by_relay(), b"from the relay");
}

/// ONE-02: an end is told the time at every step, and it is the time the
/// test supplied: zero at the start, then exactly what the test advanced it by.
#[test]
fn one_02_the_test_supplies_the_time_of_every_step() {
    let mut harness = Harness::new(Recorder::opening_with(b"x"), Recorder::silent());

    harness.run();
    harness.advance(10 * SECOND);
    harness.advance(Duration::from_millis(1));

    let later = 10 * SECOND + Duration::from_millis(1);
    assert_eq!(
        harness.hub.seen,
        [
            (Duration::ZERO, Seen::Tick),
            (10 * SECOND, Seen::Tick),
            (later, Seen::Tick)
        ]
    );
    assert_eq!(
        harness.relay.seen,
        [
            (Duration::ZERO, Seen::Tick),
            (Duration::ZERO, Seen::Bytes(b"x".to_vec())),
            (10 * SECOND, Seen::Tick),
            (later, Seen::Tick)
        ]
    );
    assert_eq!(harness.now(), Time::since_start(later));
}

/// An end that sends eight bytes of the randomness it is handed, once.
fn sends_its_randomness() -> Recorder {
    let mut first = true;
    Recorder::new(move |_, randomness| {
        let mut bytes = [0; 8];
        if std::mem::take(&mut first) {
            randomness.fill(&mut bytes);
            return send(&bytes);
        }
        Output::default()
    })
}

/// ONE-02: the randomness an end uses is the randomness the test supplied,
/// each end its own, so the same seeds give the same bytes on every run.
#[test]
fn one_02_the_test_supplies_each_end_its_randomness() {
    let run = |hub_seed, relay_seed| {
        let mut harness = Harness::with_seeds(
            sends_its_randomness(),
            sends_its_randomness(),
            hub_seed,
            relay_seed,
        );
        harness.run();
        (
            harness.sent_by_hub().to_vec(),
            harness.sent_by_relay().to_vec(),
        )
    };
    let supplied = |seed| {
        let mut bytes = [0; 8];
        Seeded::new(seed).fill(&mut bytes);
        bytes.to_vec()
    };

    assert_eq!(run(7, 9), (supplied(7), supplied(9)));
    assert_eq!(run(9, 7), (supplied(9), supplied(7)));
    assert_ne!(supplied(7), supplied(9));
    assert_ne!(supplied(7), [0; 8]);
}

/// A byte stream keeps no message boundaries: the harness can hand the same
/// bytes over in fragments of any size, down to one byte at a time.
#[test]
fn a_byte_stream_arrives_in_fragments_of_the_size_the_test_sets() {
    let mut harness = Harness::new(Recorder::opening_with(b"opening"), Recorder::silent());
    harness.fragment(3);

    harness.run();

    assert_eq!(
        harness.relay.seen,
        [
            (Duration::ZERO, Seen::Tick),
            (Duration::ZERO, Seen::Bytes(b"ope".to_vec())),
            (Duration::ZERO, Seen::Bytes(b"nin".to_vec())),
            (Duration::ZERO, Seen::Bytes(b"g".to_vec())),
        ]
    );
}

/// An end that closes its byte stream is stepped no more; the other end gets
/// the bytes sent before the close, then learns of the close, once.
#[test]
fn a_close_reaches_the_other_end_after_the_last_bytes() {
    let hub = Recorder::new(|seen, _| Output {
        bytes: b"last words".to_vec(),
        close: *seen == Seen::Tick,
    });
    // The relay tries to answer everything, the close too.
    let relay = Recorder::new(|seen, _| match seen {
        Seen::Tick => Output::default(),
        _ => send(b"unheard"),
    });
    let mut harness = Harness::new(hub, relay);

    harness.run();
    harness.advance(SECOND);

    assert_eq!(harness.hub.seen, [(Duration::ZERO, Seen::Tick)]);
    assert_eq!(
        harness.relay.seen,
        [
            (Duration::ZERO, Seen::Tick),
            (Duration::ZERO, Seen::Bytes(b"last words".to_vec())),
            (Duration::ZERO, Seen::Closed),
            (SECOND, Seen::Tick),
        ]
    );
    assert!(harness.hub_closed());
    assert!(!harness.relay_closed());
    assert_eq!(harness.sent_by_relay(), b"unheardunheard");
}

/// Two ends that answer each other for ever are a defect of the test, and
/// the harness says so instead of hanging.
#[test]
#[should_panic(expected = "the two ends never fell quiet")]
fn ends_that_never_fall_quiet_fail_the_test() {
    let echo = || Recorder::new(|_, _| send(b"again"));
    Harness::new(echo(), echo()).run();
}
