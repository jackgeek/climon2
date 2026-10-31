//! PROTOTYPE: the frozen frame layout (ADR 0001) and the prototype's post-hello messages.
use serde_json::{Value, json};
use std::io::{self, Read, Write};

pub const HELLO: u8 = 0x01;
pub const ATTACHED: u8 = 0x20;
/// Prototype-only post-hello request; version-specific, never sent cross-version.
pub const NEST_ATTACH: u8 = 0x30;
pub const ERROR: u8 = 0x7f;

pub fn write(writer: &mut impl Write, code: u8, body: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(body)?;
    writer.write_all(&((body.len() + 1) as u32).to_be_bytes())?;
    writer.write_all(&[code])?;
    writer.write_all(&body)
}

pub fn read(reader: &mut impl Read) -> io::Result<(u8, Value)> {
    let mut prefix = [0u8; 4];
    reader.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if !(1..=1_048_576).contains(&length) {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"));
    }
    let mut frame = vec![0u8; length];
    reader.read_exact(&mut frame)?;
    let body = serde_json::from_slice(&frame[1..]).map_err(io::Error::other)?;
    Ok((frame[0], body))
}

pub fn hello_request() -> Value {
    json!({ "protocol": 1 })
}
