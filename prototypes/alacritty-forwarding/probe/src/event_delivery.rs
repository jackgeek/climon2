//! THROWAWAY #48: readiness-driven socket delivery, no retry timer.
use crate::delivery::{Disconnect, Outbound};
use std::io::{self, ErrorKind, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

#[derive(Debug)]
pub struct DeliveryReport {
    pub bytes: usize,
    pub writes: usize,
    pub waits: usize,
    pub worst_gap: Duration,
}

pub fn deliver(
    socket: &mut UnixStream,
    outbound: &mut Outbound,
) -> Result<DeliveryReport, Disconnect> {
    let mut report = DeliveryReport {
        bytes: 0,
        writes: 0,
        waits: 0,
        worst_gap: Duration::ZERO,
    };
    let mut last = Instant::now();
    loop {
        let now = Instant::now();
        if let Some(reason) = outbound.poll(now) {
            return Err(reason);
        }
        let Some(chunk) = outbound.next_chunk() else {
            return Ok(report);
        };
        match socket.write(chunk) {
            Ok(0) => panic!("socket write made no progress"),
            Ok(written) => {
                let now = Instant::now();
                report.worst_gap = report.worst_gap.max(now - last);
                last = now;
                report.bytes += written;
                report.writes += 1;
                outbound.wrote(written, now);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                let until = outbound
                    .next_deadline()
                    .expect("pending output has a deadline");
                report.waits += 1;
                wait_writable(socket, until.saturating_duration_since(Instant::now())).unwrap();
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => panic!("socket delivery failed: {error}"),
        }
    }
}

pub fn wait_writable(socket: &UnixStream, timeout: Duration) -> io::Result<()> {
    let until = Instant::now() + timeout;
    let mut descriptor = libc::pollfd {
        fd: socket.as_raw_fd(),
        events: libc::POLLOUT,
        revents: 0,
    };
    loop {
        let left = until.saturating_duration_since(Instant::now());
        let millis = left
            .as_millis()
            .saturating_add(u128::from(!left.is_zero()))
            .min(i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
        if result >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
