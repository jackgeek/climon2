//! The on-path proxy (spec #488; acceptance, Evidence policy): a test-only
//! TCP proxy between a hub monitor and a relay on loopback. It stands for
//! every attacker on the path of a monitor link. Each chunk it reads passes
//! through its tamper before it is sent on, so a test can drop, delay,
//! reorder, replay, edit or inject bytes; it uses no operating-system
//! network tooling. It records every connection's bytes in each direction
//! as it sent them on.

use std::{
    io::{ErrorKind, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

/// Which way a chunk is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    ToRelay,
    ToHub,
}

/// Where a chunk stands: its connection, counted from 0 in the order they
/// were made, its direction, and how many bytes went that way before it.
#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    pub connection: usize,
    pub direction: Direction,
    pub offset: usize,
}

/// What the proxy does with each chunk: the bytes it sends on in its place.
pub type Tamper = dyn FnMut(Chunk, &[u8]) -> Vec<u8> + Send;

/// One connection's bytes, as the proxy sent them on.
#[derive(Clone, Debug, Default)]
pub struct Carried {
    pub to_relay: Vec<u8>,
    pub to_hub: Vec<u8>,
}

impl Carried {
    fn side(&mut self, direction: Direction) -> &mut Vec<u8> {
        match direction {
            Direction::ToRelay => &mut self.to_relay,
            Direction::ToHub => &mut self.to_hub,
        }
    }
}

pub struct Proxy {
    address: SocketAddr,
    carried: Arc<Mutex<Vec<Carried>>>,
    stopping: Arc<AtomicBool>,
    acceptor: Option<JoinHandle<()>>,
}

impl Proxy {
    /// A proxy to `relay` that carries every byte unchanged.
    pub fn faithful(relay: SocketAddr) -> Self {
        Self::tampering(relay, |_, bytes| bytes.to_vec())
    }

    /// A proxy to `relay` that sends on what `tamper` makes of each chunk.
    pub fn tampering(
        relay: SocketAddr,
        tamper: impl FnMut(Chunk, &[u8]) -> Vec<u8> + Send + 'static,
    ) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let carried = Arc::new(Mutex::new(Vec::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let tamper: Arc<Mutex<Box<Tamper>>> = Arc::new(Mutex::new(Box::new(tamper)));
        let acceptor = {
            let (carried, stopping) = (Arc::clone(&carried), Arc::clone(&stopping));
            thread::spawn(move || accept(&listener, relay, &carried, &stopping, &tamper))
        };
        Self {
            address,
            carried,
            stopping,
            acceptor: Some(acceptor),
        }
    }

    /// Where a hub monitor dials the proxy.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Every connection so far, in the order they were made.
    pub fn carried(&self) -> Vec<Carried> {
        self.carried.lock().unwrap().clone()
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(acceptor) = self.acceptor.take() {
            acceptor.join().unwrap();
        }
    }
}

fn accept(
    listener: &TcpListener,
    relay: SocketAddr,
    carried: &Arc<Mutex<Vec<Carried>>>,
    stopping: &AtomicBool,
    tamper: &Arc<Mutex<Box<Tamper>>>,
) {
    while !stopping.load(Ordering::Relaxed) {
        let hub = match listener.accept() {
            Ok((hub, _)) => hub,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(error) => panic!("the proxy cannot accept: {error}"),
        };
        hub.set_nonblocking(false).unwrap();
        let connection = {
            let mut carried = carried.lock().unwrap();
            carried.push(Carried::default());
            carried.len() - 1
        };
        let Ok(to_relay) = TcpStream::connect(relay) else {
            continue;
        };
        for (from, to, direction) in [
            (
                hub.try_clone().unwrap(),
                to_relay.try_clone().unwrap(),
                Direction::ToRelay,
            ),
            (to_relay, hub, Direction::ToHub),
        ] {
            let (carried, tamper) = (Arc::clone(carried), Arc::clone(tamper));
            thread::spawn(move || pump(from, to, connection, direction, &carried, &tamper));
        }
    }
}

/// Carries one direction of one connection until its sender closes, then
/// closes that direction on the other side.
fn pump(
    mut from: TcpStream,
    mut to: TcpStream,
    connection: usize,
    direction: Direction,
    carried: &Mutex<Vec<Carried>>,
    tamper: &Mutex<Box<Tamper>>,
) {
    let mut buffer = [0; 16_384];
    let mut offset = 0;
    while let Ok(count @ 1..) = from.read(&mut buffer) {
        let chunk = Chunk {
            connection,
            direction,
            offset,
        };
        offset += count;
        let bytes = (tamper.lock().unwrap())(chunk, &buffer[..count]);
        carried.lock().unwrap()[connection]
            .side(direction)
            .extend(&bytes);
        if to.write_all(&bytes).is_err() {
            break;
        }
    }
    let _ = to.shutdown(Shutdown::Write);
}
