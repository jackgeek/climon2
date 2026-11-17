//! The frozen hello probe (ADR 0001): nonblocking I/O under one absolute deadline.
use climon2_core::{
    hello::HelloExtensions,
    metadata::SessionMetadata,
    protocol::{HelloReply, HelloRequest, PROTOCOL_VERSION, read_frame, write_frame},
};
use climon2_platform::{connection, runtime::SocketStream};
use std::{
    io::{self, Read, Write},
    time::{Duration, Instant},
};

struct DeadlineStream {
    stream: SocketStream,
    deadline: Instant,
}

impl DeadlineStream {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "session probe timed out"))
    }

    fn wait_for(&self, interest: connection::Interest) -> io::Result<()> {
        connection::wait(&self.stream, interest, self.remaining()?)
    }

    fn transfer(
        &mut self,
        interest: connection::Interest,
        mut operation: impl FnMut(&mut SocketStream) -> io::Result<usize>,
    ) -> io::Result<usize> {
        loop {
            self.remaining()?;
            let result = operation(&mut self.stream);
            if !would_block(&result) {
                return result;
            }
            self.wait_for(interest)?;
        }
    }
}

fn would_block(result: &io::Result<usize>) -> bool {
    result
        .as_ref()
        .is_err_and(|error| error.kind() == io::ErrorKind::WouldBlock)
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.transfer(connection::Interest::Read, |stream| stream.read(bytes))
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(connection::Interest::Write, |stream| stream.write(bytes))
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

/// `Ok(None)` when something answered, but not as this session.
pub(crate) fn hello(
    stream: SocketStream,
    deadline: Instant,
    metadata: &SessionMetadata,
) -> io::Result<Option<HelloExtensions>> {
    let (reply, extensions) = exchange_hello(stream, deadline)?;
    Ok(reply
        .is_live_session(&metadata.session_id)
        .then_some(extensions))
}

/// Nonblocking I/O under one absolute deadline; never socket timeouts (Darwin EINVAL after FIN).
fn exchange_hello(
    stream: SocketStream,
    deadline: Instant,
) -> io::Result<(HelloReply, HelloExtensions)> {
    stream.set_nonblocking(true)?;
    let mut stream = DeadlineStream { stream, deadline };
    send_hello(&mut stream)?;
    receive_hello(&mut stream)
}

fn send_hello(stream: &mut DeadlineStream) -> io::Result<()> {
    let hello = HelloRequest {
        protocol: PROTOCOL_VERSION,
    };
    write_frame(stream, &hello.to_frame()?)
}

fn receive_hello(stream: &mut DeadlineStream) -> io::Result<(HelloReply, HelloExtensions)> {
    let frame = read_frame(stream)?;
    let extensions = HelloExtensions::from_body(frame.body());
    Ok((HelloReply::from_frame(&frame)?, extensions))
}
