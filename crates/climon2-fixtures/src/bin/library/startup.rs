//! Caller-owned startup memory. Every byte `create` reads (the startup struct
//! and each of its buffers) lives in one anonymous mapping the host scrambles
//! and unmaps once `create` returns: an instance that kept a pointer into it
//! would fault, and a library that freed any of it with its allocator would
//! abort, because no allocator owns this memory.
//!
//! Tests describe a startup as JSON overrides of [`default`]. A buffer is a
//! UTF-8 string, `{"hex": ...}` bytes, `{"null": length}` (a null pointer
//! with that length), `{"dangling": true}` (a nonnull pointer and length
//! zero), or `{"prefix": ..., "fill": ..., "length": n}` (exactly `n` bytes).

use climon2_core::native::{self, Bytes};
use serde_json::{Value, json};
use std::ptr::{self, NonNull};

pub const FILL: u8 = super::abi::FILL;
const SIZE: usize = 1 << 20;

/// One mapping of caller memory; nothing in it is ever freed by an allocator.
pub struct Arena {
    base: NonNull<u8>,
    used: usize,
}

// SAFETY: the mapping is plain memory the host hands to one thread at a time.
unsafe impl Send for Arena {}

impl Arena {
    pub fn new() -> Self {
        // SAFETY: an anonymous private mapping takes no pointer arguments.
        let base = unsafe {
            libc::mmap(
                ptr::null_mut(),
                SIZE,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        assert_ne!(base, libc::MAP_FAILED, "caller memory is mapped");
        Self {
            base: NonNull::new(base.cast()).expect("mmap never returns null"),
            used: 0,
        }
    }

    fn place(&mut self, bytes: &[u8], align: usize) -> *mut u8 {
        let start = self.used.next_multiple_of(align);
        assert!(start + bytes.len() <= SIZE, "caller memory is large enough");
        // SAFETY: the range is inside the mapping and unused.
        let target = unsafe { self.base.as_ptr().add(start) };
        // SAFETY: as above; the source is a distinct host buffer.
        unsafe { ptr::copy_nonoverlapping(bytes.as_ptr(), target, bytes.len()) };
        self.used = start + bytes.len().max(1);
        target
    }

    pub fn value<T>(&mut self, value: T) -> *mut T {
        // SAFETY: `value` is a plain value viewed as its bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts((&raw const value).cast::<u8>(), size_of::<T>()) };
        self.place(bytes, align_of::<T>()).cast()
    }

    /// `values` in caller memory, or null for none.
    pub fn slice<T>(&mut self, values: &[T]) -> *const T {
        if values.is_empty() {
            return ptr::null();
        }
        // SAFETY: the values are plain data viewed as their bytes.
        let bytes = unsafe {
            std::slice::from_raw_parts(values.as_ptr().cast::<u8>(), size_of_val(values))
        };
        self.place(bytes, align_of::<T>()).cast()
    }

    pub fn bytes(&mut self, spec: &Value) -> Bytes {
        if let Some(length) = spec.get("null") {
            return Bytes {
                data: ptr::null(),
                length: length.as_u64().expect("a null buffer's length"),
            };
        }
        if spec.get("dangling").is_some() {
            return Bytes {
                data: self.place(&[FILL], 1),
                length: 0,
            };
        }
        let content = content(spec);
        if content.is_empty() {
            return Bytes {
                data: ptr::null(),
                length: 0,
            };
        }
        Bytes {
            data: self.place(&content, 1),
            length: content.len() as u64,
        }
    }

    /// Overwrites every byte the library was given, then unmaps it.
    pub fn scramble_and_unmap(self) {
        // SAFETY: the whole mapping is writable and owned by this arena.
        unsafe { self.base.as_ptr().write_bytes(FILL, SIZE) };
        // SAFETY: unmapping exactly the mapping this arena made.
        let result = unsafe { libc::munmap(self.base.as_ptr().cast(), SIZE) };
        assert_eq!(result, 0, "caller memory is unmapped");
    }
}

/// The bytes a buffer spec describes.
pub fn content(spec: &Value) -> Vec<u8> {
    if let Some(text) = spec.as_str() {
        return text.as_bytes().to_vec();
    }
    if let Some(hex) = spec.get("hex") {
        let hex = hex.as_str().expect("hex text");
        return (0..hex.len())
            .step_by(2)
            .map(|index| u8::from_str_radix(&hex[index..index + 2], 16).expect("hex byte"))
            .collect();
    }
    let prefix = spec["prefix"].as_str().expect("a sized buffer's prefix");
    let fill = spec["fill"]
        .as_str()
        .expect("a sized buffer's fill")
        .as_bytes()[0];
    let length = spec["length"].as_u64().expect("a sized buffer's length") as usize;
    let mut bytes = prefix.as_bytes().to_vec();
    bytes.resize(length, fill);
    bytes
}

/// A valid startup for `release` under `root`: the documented default
/// `monitor.limits`, both client keys enabled, no log file.
pub fn default(root: &str, release: &str) -> Value {
    json!({
        "session_id": "00112233445566778899aabbccddeeff",
        "generation": 1,
        "expected_release": release,
        "metadata_path": path(&format!("{root}/sessions/session.pty")),
        "home_path": path(root),
        "runtime_path": path(&format!("{root}/runtime")),
        "settings": {
            "escape_enabled": 1,
            "escape_byte": 0x1d,
            "take_control_enabled": 1,
            "take_control_byte": 0x14,
            "reserved": 0,
        },
        "diagnostics": {
            "log_path": { "present": 0, "reserved": 0, "value": path("") },
            "filter": "info",
        },
        "limits": {
            "client_bytes": 67_108_864u64,
            "request_bytes": 262_144,
            "event_bytes": 1_048_576,
            "progress_bytes": 65_536,
            "drain_payload_bytes": 65_536,
            "request_records": 64,
            "event_records": 1024,
            "progress_records": 32,
            "deliveries": 8,
            "operations": 16,
            "chunk_bytes": 65_536,
            "drain_records": 64,
            "wire_frame_bytes": 1_048_576,
            "metadata_record_bytes": 16_384,
            "title_bytes": 4096,
            "notification_title_bytes": 4096,
            "notification_body_bytes": 8192,
            "max_workers": 2,
            "operation_ms": 5000,
            "kill_ms": 10_000,
            "restoration_stall_ms": 5000,
            "restoration_total_ms": 60_000,
            "delivery_stall_ms": 5000,
            "leave_ms": 5000,
            "cleanup_ms": 10_000,
            "start_ms": 10_000,
            "reserved": 0,
        },
    })
}

fn path(bytes: &str) -> Value {
    json!({ "encoding": native::NATIVE_UNIX, "reserved": 0, "bytes": bytes })
}

/// `base` with `overrides` merged in: objects field by field, anything else
/// replaced.
pub fn merged(base: &Value, overrides: &Value) -> Value {
    match (base, overrides) {
        (Value::Object(base), Value::Object(overrides)) => {
            let mut result = base.clone();
            for (key, value) in overrides {
                let merged = result
                    .get(key)
                    .map_or_else(|| value.clone(), |base| merged(base, value));
                result.insert(key.clone(), merged);
            }
            Value::Object(result)
        }
        _ => overrides.clone(),
    }
}

fn number<T: TryFrom<u64>>(spec: &Value, field: &str) -> T {
    let value = spec[field]
        .as_u64()
        .unwrap_or_else(|| panic!("{field} is a number"));
    T::try_from(value).unwrap_or_else(|_| panic!("{field} fits its native type"))
}

fn native_path(arena: &mut Arena, spec: &Value) -> native::Path {
    native::Path {
        encoding: number(spec, "encoding"),
        reserved: number(spec, "reserved"),
        bytes: arena.bytes(&spec["bytes"]),
    }
}

fn limits(spec: &Value) -> native::Limits {
    native::Limits {
        client_bytes: number(spec, "client_bytes"),
        request_bytes: number(spec, "request_bytes"),
        event_bytes: number(spec, "event_bytes"),
        progress_bytes: number(spec, "progress_bytes"),
        drain_payload_bytes: number(spec, "drain_payload_bytes"),
        request_records: number(spec, "request_records"),
        event_records: number(spec, "event_records"),
        progress_records: number(spec, "progress_records"),
        deliveries: number(spec, "deliveries"),
        operations: number(spec, "operations"),
        chunk_bytes: number(spec, "chunk_bytes"),
        drain_records: number(spec, "drain_records"),
        wire_frame_bytes: number(spec, "wire_frame_bytes"),
        metadata_record_bytes: number(spec, "metadata_record_bytes"),
        title_bytes: number(spec, "title_bytes"),
        notification_title_bytes: number(spec, "notification_title_bytes"),
        notification_body_bytes: number(spec, "notification_body_bytes"),
        max_workers: number(spec, "max_workers"),
        operation_ms: number(spec, "operation_ms"),
        kill_ms: number(spec, "kill_ms"),
        restoration_stall_ms: number(spec, "restoration_stall_ms"),
        restoration_total_ms: number(spec, "restoration_total_ms"),
        delivery_stall_ms: number(spec, "delivery_stall_ms"),
        leave_ms: number(spec, "leave_ms"),
        cleanup_ms: number(spec, "cleanup_ms"),
        start_ms: number(spec, "start_ms"),
        reserved: number(spec, "reserved"),
    }
}

/// The startup `spec` describes, entirely in `arena`.
pub fn build(arena: &mut Arena, spec: &Value) -> *const native::Startup {
    let session = content(&json!({ "hex": spec["session_id"] }));
    let settings = &spec["settings"];
    let log = &spec["diagnostics"]["log_path"];
    let startup = native::Startup {
        session_id: native::Guid {
            bytes: session.try_into().expect("a 16-byte session ID"),
        },
        generation: number(spec, "generation"),
        expected_release: arena.bytes(&spec["expected_release"]),
        metadata_path: native_path(arena, &spec["metadata_path"]),
        home_path: native_path(arena, &spec["home_path"]),
        runtime_path: native_path(arena, &spec["runtime_path"]),
        settings: native::Settings {
            escape_enabled: number(settings, "escape_enabled"),
            escape_byte: number(settings, "escape_byte"),
            take_control_enabled: number(settings, "take_control_enabled"),
            take_control_byte: number(settings, "take_control_byte"),
            reserved: number(settings, "reserved"),
        },
        diagnostics: native::Diagnostics {
            log_path: native::OptionalPath {
                present: number(log, "present"),
                reserved: number(log, "reserved"),
                value: native_path(arena, &log["value"]),
            },
            filter: arena.bytes(&spec["diagnostics"]["filter"]),
        },
        limits: limits(&spec["limits"]),
    };
    arena.value(startup)
}
