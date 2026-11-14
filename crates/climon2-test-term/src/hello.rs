//! A raw-socket hello peer: something at a session's socket that answers probes.
use climon2_core::protocol::{Frame, read_frame, write_frame};
use climon2_platform::runtime;
use std::{path::Path, thread};

/// Listens at `socket` and answers each of the next `probes` connections, once it has read the
/// frozen hello request `{"protocol":1}`, with one hello frame whose body is `reply`. Joining
/// it waits for every probe, and fails if any request was not the frozen hello.
pub fn hello_peer(socket: impl AsRef<Path>, reply: &str, probes: usize) -> thread::JoinHandle<()> {
    let listener = runtime::bind(socket.as_ref()).expect("bind the hello peer's socket");
    let reply = Frame::new(1, reply.as_bytes().to_vec()).expect("a hello frame");
    thread::spawn(move || {
        for _ in 0..probes {
            let (mut connection, _) = listener.accept().unwrap();
            let request = read_frame(&mut connection).unwrap();
            assert_eq!(request.body(), b"{\"protocol\":1}");
            write_frame(&mut connection, &reply).unwrap();
        }
    })
}
