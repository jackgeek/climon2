//! #501: `drive` runs one end over a caller's byte stream and clock. These
//! tests give it a scripted end and a scripted carrier, so they see exactly
//! what the end was handed, what was sent and how long each wait lasted.

mod support;

use climon2_link::{Carrier, End, Input, Next, Output, Randomness, Time, drive};
use std::{collections::VecDeque, io, time::Duration};
use support::Seeded;

fn at(seconds: u64) -> Time {
    Time::since_start(Duration::from_secs(seconds))
}

/// What a scripted end was handed at one step.
#[derive(Debug, PartialEq, Eq)]
enum Seen {
    Tick,
    Bytes(Vec<u8>),
    Closed,
}

/// An end that answers each step with the next output of its script.
struct Scripted {
    outputs: VecDeque<Output>,
    seen: Vec<(Time, Seen)>,
}

impl Scripted {
    fn new(outputs: impl IntoIterator<Item = Output>) -> Self {
        Self {
            outputs: outputs.into_iter().collect(),
            seen: Vec::new(),
        }
    }
}

impl End for Scripted {
    fn step(&mut self, now: Time, input: Input<'_>, _: &mut dyn Randomness) -> Output {
        let seen = match input {
            Input::Tick => Seen::Tick,
            Input::Bytes(bytes) => Seen::Bytes(bytes.to_vec()),
            Input::Closed => Seen::Closed,
        };
        self.seen.push((now, seen));
        self.outputs
            .pop_front()
            .expect("the script has a step left")
    }
}

/// A carrier whose clock and arrivals come from a script.
#[derive(Default)]
struct Line {
    times: VecDeque<Time>,
    arrivals: VecDeque<io::Result<Vec<u8>>>,
    sends_fail: bool,
    sent: Vec<Vec<u8>>,
    waits: Vec<Option<Duration>>,
}

impl Line {
    fn new(times: impl IntoIterator<Item = Time>) -> Self {
        Self {
            times: times.into_iter().collect(),
            ..Self::default()
        }
    }

    fn arriving(mut self, arrivals: impl IntoIterator<Item = io::Result<Vec<u8>>>) -> Self {
        self.arrivals = arrivals.into_iter().collect();
        self
    }
}

impl Carrier for Line {
    fn now(&mut self) -> Time {
        self.times
            .pop_front()
            .expect("the clock has a reading left")
    }

    fn send(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.sends_fail {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        self.sent.push(bytes.to_vec());
        Ok(())
    }

    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize> {
        self.waits.push(wait);
        let bytes = self.arrivals.pop_front().expect("an arrival is scripted")?;
        buffer[..bytes.len()].copy_from_slice(&bytes);
        Ok(bytes.len())
    }
}

fn output(bytes: &[u8], deadline: Option<Time>) -> Output {
    Output {
        bytes: bytes.to_vec(),
        close: false,
        deadline,
    }
}

fn closing(bytes: &[u8]) -> Output {
    Output {
        bytes: bytes.to_vec(),
        close: true,
        deadline: None,
    }
}

fn run(end: &mut Scripted, line: &mut Line) {
    let between = |_: &mut Scripted| Ok(Next::Wait);
    drive(end, line, &mut Seeded::new(1), between).unwrap();
}

#[test]
fn the_end_starts_with_a_tick_and_its_bytes_are_sent_before_the_wait() {
    let mut end = Scripted::new([output(b"ab", Some(at(10))), closing(b"cd")]);
    let mut line = Line::new([at(0), at(10)]).arriving([Err(io::ErrorKind::TimedOut.into())]);
    run(&mut end, &mut line);
    assert_eq!(end.seen, [(at(0), Seen::Tick), (at(10), Seen::Tick)]);
    assert_eq!(line.sent, [b"ab".to_vec(), b"cd".to_vec()]);
    assert_eq!(line.waits, [Some(Duration::from_secs(10))]);
}

#[test]
fn a_wait_lasts_only_until_the_deadline_and_no_wait_has_no_limit() {
    let mut end = Scripted::new([output(b"", Some(at(10))), output(b"", None), closing(b"")]);
    let mut line =
        Line::new([at(3), at(4), at(5)]).arriving([Ok(b"x".to_vec()), Ok(b"y".to_vec())]);
    run(&mut end, &mut line);
    assert_eq!(line.waits, [Some(Duration::from_secs(7)), None]);
    assert!(line.sent.is_empty(), "nothing empty is sent");
}

#[test]
fn bytes_that_arrive_are_handed_over_and_the_end_of_the_stream_is_a_close() {
    let mut end = Scripted::new([output(b"", None), output(b"", None), closing(b"")]);
    let mut line = Line::new([at(0), at(1), at(2)]).arriving([Ok(b"xyz".to_vec()), Ok(Vec::new())]);
    run(&mut end, &mut line);
    assert_eq!(
        end.seen,
        [
            (at(0), Seen::Tick),
            (at(1), Seen::Bytes(b"xyz".to_vec())),
            (at(2), Seen::Closed)
        ]
    );
}

#[test]
fn a_failed_receive_is_a_close_and_a_timed_out_one_a_tick() {
    let mut end = Scripted::new([output(b"", None), output(b"", None), closing(b"")]);
    let mut line = Line::new([at(0), at(1), at(2)]).arriving([
        Err(io::ErrorKind::TimedOut.into()),
        Err(io::ErrorKind::ConnectionReset.into()),
    ]);
    run(&mut end, &mut line);
    let seen: Vec<_> = end.seen.into_iter().map(|(_, seen)| seen).collect();
    assert_eq!(seen, [Seen::Tick, Seen::Tick, Seen::Closed]);
}

#[test]
fn a_deadline_already_reached_steps_the_end_at_once() {
    let mut end = Scripted::new([output(b"", Some(at(5))), closing(b"")]);
    let mut line = Line::new([at(5), at(6)]);
    run(&mut end, &mut line);
    assert_eq!(end.seen, [(at(5), Seen::Tick), (at(6), Seen::Tick)]);
    assert!(line.waits.is_empty());
    let mut end = Scripted::new([output(b"", Some(at(5))), closing(b"")]);
    let mut line = Line::new([at(7), at(8)]);
    run(&mut end, &mut line);
    assert!(line.waits.is_empty(), "a deadline passed is reached");
}

#[test]
fn bytes_that_cannot_be_sent_close_the_stream_for_the_end() {
    let mut end = Scripted::new([output(b"ab", Some(at(10))), closing(b"cd")]);
    let mut line = Line::new([at(0), at(1)]);
    line.sends_fail = true;
    run(&mut end, &mut line);
    assert_eq!(end.seen, [(at(0), Seen::Tick), (at(1), Seen::Closed)]);
    assert!(line.waits.is_empty());
}

#[test]
fn a_close_sends_its_bytes_and_ends_the_run() {
    let mut end = Scripted::new([closing(b"bye")]);
    let mut line = Line::new([at(0)]);
    run(&mut end, &mut line);
    assert_eq!(line.sent, [b"bye".to_vec()]);
    assert!(line.waits.is_empty());
}

#[test]
fn the_caller_between_steps_can_step_again_at_once_or_stop() {
    let mut end = Scripted::new([
        output(b"a", Some(at(10))),
        output(b"b", Some(at(10))),
        output(b"c", Some(at(10))),
    ]);
    let mut line = Line::new([at(0), at(1), at(2)]);
    let mut plan = VecDeque::from([Next::Step, Next::Step, Next::Stop]);
    drive(&mut end, &mut line, &mut Seeded::new(1), |_| {
        Ok(plan.pop_front().unwrap())
    })
    .unwrap();
    assert_eq!(end.seen.len(), 3);
    assert_eq!(line.sent, [b"a".to_vec(), b"b".to_vec(), b"c".to_vec()]);
    assert!(line.waits.is_empty(), "no step waited");
}

#[test]
fn the_caller_sees_each_step_before_its_bytes_go_and_its_error_stops_the_run() {
    let mut end = Scripted::new([output(b"a", Some(at(10))), output(b"secret", None)]);
    let mut line = Line::new([at(0), at(1)]).arriving([Ok(b"x".to_vec())]);
    let mut steps = 0;
    let error = drive(&mut end, &mut line, &mut Seeded::new(1), |end| {
        steps += 1;
        assert_eq!(end.seen.len(), steps, "the step has happened");
        match steps {
            1 => Ok(Next::Wait),
            _ => Err(io::Error::other("refused")),
        }
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "refused");
    assert_eq!(
        line.sent,
        [b"a".to_vec()],
        "nothing of the refused step is sent"
    );
}
