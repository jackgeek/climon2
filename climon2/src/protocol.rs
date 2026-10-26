use socket2::{Domain, Socket, Type, SockAddr};
use serde_json::Value;
use std::{io::{self, Read, Write}, path::Path, time::{Duration, Instant}};

pub const MAX: usize = 1_048_576;
pub struct Stream(pub Socket);
impl Stream {
    pub fn connect(path: &Path, deadline: Instant) -> io::Result<Self> {
        let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
        socket.connect_timeout(&SockAddr::unix(path)?, remaining(deadline)?)?;
        Ok(Self(socket))
    }
    pub fn clone(&self) -> io::Result<Self> { Ok(Self(self.0.try_clone()?)) }
    pub fn send(&self, value: &Value) -> io::Result<()> {
        let payload = serde_json::to_vec(value)?;
        if payload.is_empty() || payload.len() > MAX { return Err(invalid()); }
        let mut socket = &self.0;
        socket.write_all(&(payload.len() as u32).to_be_bytes())?;
        socket.write_all(&payload)
    }
    pub fn receive(&self, deadline: Option<Instant>) -> io::Result<Value> {
        let mut prefix = [0; 4];
        self.read_exact(&mut prefix, deadline)?;
        let size = u32::from_be_bytes(prefix) as usize;
        if size == 0 || size > MAX { return Err(invalid()); }
        let mut payload = vec![0; size];
        self.read_exact(&mut payload, deadline)?;
        serde_json::from_slice(&payload).map_err(|_| invalid())
    }
    fn read_exact(&self, mut buffer: &mut [u8], deadline: Option<Instant>) -> io::Result<()> {
        while !buffer.is_empty() {
            self.0.set_read_timeout(deadline.map(remaining).transpose()?)?;
            let count = (&self.0).read(buffer)?;
            if count == 0 { return Err(io::ErrorKind::UnexpectedEof.into()); }
            buffer = &mut buffer[count..];
        }
        Ok(())
    }
}
fn remaining(deadline: Instant) -> io::Result<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or_else(|| io::ErrorKind::TimedOut.into())
}
fn invalid() -> io::Error { io::Error::new(io::ErrorKind::InvalidData, "invalid protocol frame") }
pub fn listener(path: &Path) -> io::Result<Socket> {
    let socket = Socket::new(Domain::UNIX, Type::STREAM, None)?;
    let address = SockAddr::unix(path)?;
    #[cfg(windows)]
    crate::platform::bind_private(&socket, &address)?;
    #[cfg(unix)]
    socket.bind(&address)?;
    socket.listen(128)?;
    Ok(socket)
}
pub fn hello() -> Value { serde_json::json!({"type":"hello", "protocol":1}) }
pub fn live(path: &Path, sid: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    let result = (|| -> io::Result<bool> {
        let stream = Stream::connect(path, deadline)?;
        stream.0.set_write_timeout(Some(remaining(deadline)?))?;
        stream.send(&hello())?;
        let value = stream.receive(Some(deadline))?;
        Ok(value["type"] == "hello" && value["protocol"] == 1 && value["session_id"] == sid)
    })();
    result.unwrap_or(false)
}
