//! Running one [`End`] over a real byte stream. The caller hands over a
//! [`Carrier`]: its byte stream and its clock. The loop here steps the end,
//! sends what it asks, and waits for bytes no longer than the end's
//! deadline, so every caller keeps the deadlines the same way. It performs
//! no input or output of its own and reads no clock: the carrier does both.

use crate::{End, Input, Output, Randomness, Time};
use std::{io, time::Duration};

/// The most bytes one wait takes from the byte stream.
const READ_SIZE: usize = 16_384;

/// The caller's byte stream and clock, as [`drive`] uses them.
pub trait Carrier {
    /// The time now on the caller's clock.
    fn now(&mut self) -> Time;
    /// Sends all of `bytes`, in order.
    fn send(&mut self, bytes: &[u8]) -> io::Result<()>;
    /// Waits no longer than `wait`, or without limit when it is `None`, for
    /// bytes, and puts what arrived at the start of `buffer`. `Ok(0)` means
    /// the other end closed the byte stream, and a `TimedOut` error that
    /// nothing arrived in time.
    fn receive(&mut self, buffer: &mut [u8], wait: Option<Duration>) -> io::Result<usize>;
}

/// What [`drive`] does after a step, as the caller decides between the
/// step and the sending of its bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Sends the step's bytes, then waits for bytes or the deadline.
    Wait,
    /// Sends the step's bytes, then steps again at once: the caller has
    /// given the end something to send.
    Step,
    /// Sends the step's bytes and stops.
    Stop,
}

/// What arrived for the next step.
enum Arrived {
    Tick,
    Bytes(usize),
    Closed,
}

impl Arrived {
    fn input(self, buffer: &[u8]) -> Input<'_> {
        match self {
            Self::Tick => Input::Tick,
            Self::Bytes(count) => Input::Bytes(&buffer[..count]),
            Self::Closed => Input::Closed,
        }
    }
}

/// Runs `end` over `carrier` until the end closes the byte stream or
/// `between` says to stop. After every step and before its bytes are sent,
/// `between` sees the end: it can act on how far the end has got, give it
/// something to send, or refuse to go on, and an error it gives stops the
/// run with nothing more sent. A byte stream that fails to send or receive
/// is closed, as far as the end is told.
pub fn drive<E: End>(
    end: &mut E,
    carrier: &mut impl Carrier,
    randomness: &mut dyn Randomness,
    mut between: impl FnMut(&mut E) -> io::Result<Next>,
) -> io::Result<()> {
    let mut buffer = vec![0; READ_SIZE];
    let mut arrived = Arrived::Tick;
    loop {
        let now = carrier.now();
        let output = end.step(now, arrived.input(&buffer), randomness);
        let next = between(end)?;
        match after(carrier, &mut buffer, now, &output, next) {
            Some(next_arrival) => arrived = next_arrival,
            None => return Ok(()),
        }
    }
}

/// Sends `output`'s bytes, then gives what the next step takes, or nothing
/// when the run is over.
fn after(
    carrier: &mut impl Carrier,
    buffer: &mut [u8],
    now: Time,
    output: &Output,
    next: Next,
) -> Option<Arrived> {
    let sent = output.bytes.is_empty() || carrier.send(&output.bytes).is_ok();
    if output.close || next == Next::Stop {
        return None;
    }
    Some(match (sent, next) {
        (false, _) => Arrived::Closed,
        (true, Next::Step) => Arrived::Tick,
        (true, _) => wait(carrier, buffer, now, output.deadline),
    })
}

/// Waits for bytes until `deadline`, which the end named at `now`. A
/// deadline already reached steps the end at once.
fn wait(
    carrier: &mut impl Carrier,
    buffer: &mut [u8],
    now: Time,
    deadline: Option<Time>,
) -> Arrived {
    let wait = match deadline.map(|deadline| remaining(now, deadline)) {
        Some(None) => return Arrived::Tick,
        Some(Some(left)) => Some(left),
        None => None,
    };
    arrival(carrier.receive(buffer, wait))
}

/// The time from `now` to `deadline`, or nothing when it has come.
fn remaining(now: Time, deadline: Time) -> Option<Duration> {
    let left = deadline.elapsed().saturating_sub(now.elapsed());
    (!left.is_zero()).then_some(left)
}

/// What one wait for bytes gave the end.
fn arrival(received: io::Result<usize>) -> Arrived {
    match received {
        Ok(0) => Arrived::Closed,
        Ok(count) => Arrived::Bytes(count),
        Err(error) if error.kind() == io::ErrorKind::TimedOut => Arrived::Tick,
        Err(_) => Arrived::Closed,
    }
}
