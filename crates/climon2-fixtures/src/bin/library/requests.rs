//! Requests in caller memory (#93). Every byte `submit` may read (the
//! request and each buffer and list it names) lives in one mapping the host
//! scrambles and unmaps as soon as `submit` returns, so an accepted request
//! the library had not copied would fault, and a rejected one it retained
//! would too.
//!
//! A request is JSON: `tag` (a number, `"open"` or `"leave"`), `request_id`,
//! optional `reserved`, `fill` (a byte written over the whole request
//! first, so inactive storage holds it), and the active member: `open`
//! (`mode`, `reserved`, `geometry` `[cols, rows]`, token lists `required`
//! and `optional`) or `generation`. A token list is an array of buffers (see
//! `startup`), `{"null": count}` or `{"dangling": count}`.

use super::startup::Arena;
use climon2_core::native::{self, Bytes, Tokens};
use serde_json::Value;
use std::{mem::MaybeUninit, ptr};

fn number<T: TryFrom<u64>>(value: &Value, default: u64) -> T {
    let value = value.as_u64().unwrap_or(default);
    T::try_from(value).unwrap_or_else(|_| panic!("{value} fits its native type"))
}

fn tag(spec: &Value) -> u32 {
    match spec.as_str() {
        Some("open") => native::OPEN,
        Some("leave") => native::LEAVE,
        _ => number(spec, 0),
    }
}

fn tokens(arena: &mut Arena, spec: &Value) -> Tokens {
    if let Some(count) = spec.get("null") {
        return Tokens {
            data: ptr::null(),
            count: number(count, 0),
        };
    }
    if let Some(count) = spec.get("dangling") {
        let anchor = arena.slice(&[Bytes {
            data: ptr::null(),
            length: 0,
        }]);
        return Tokens {
            data: anchor,
            count: number(count, 0),
        };
    }
    let list: Vec<Bytes> = spec
        .as_array()
        .map(|tokens| tokens.iter().map(|token| arena.bytes(token)).collect())
        .unwrap_or_default();
    Tokens {
        data: arena.slice(&list),
        count: list.len() as u64,
    }
}

fn open(arena: &mut Arena, spec: &Value) -> native::Open {
    let geometry = spec["geometry"].as_array();
    let side = |index: usize| geometry.map_or(0, |sides| number(&sides[index], 0));
    native::Open {
        mode: number(&spec["mode"], u64::from(native::MONITOR)),
        reserved: number(&spec["reserved"], 0),
        geometry: native::Geometry {
            cols: side(0),
            rows: side(1),
        },
        required: tokens(arena, &spec["required"]),
        optional: tokens(arena, &spec["optional"]),
    }
}

/// The request `spec` describes, entirely in `arena`.
pub fn build(arena: &mut Arena, spec: &Value) -> *const native::Request {
    let fill = spec["fill"].as_u64().unwrap_or(0) as u8;
    let mut request = MaybeUninit::<native::Request>::uninit();
    // SAFETY: the pointer covers exactly one request.
    unsafe {
        request
            .as_mut_ptr()
            .cast::<u8>()
            .write_bytes(fill, size_of::<native::Request>())
    };
    // SAFETY: every byte is initialized, and a request is plain data.
    let mut request = unsafe { request.assume_init() };
    request.tag = tag(&spec["tag"]);
    request.reserved = number(&spec["reserved"], 0);
    request.request_id = number(&spec["request_id"], 0);
    match request.tag {
        native::OPEN => request.args.open = open(arena, &spec["open"]),
        native::LEAVE => request.args.generation = number(&spec["generation"], 0),
        _ => {}
    }
    arena.value(request)
}
