//! The status, viewing and acknowledgement suites' vocabulary (#95):
//! native constants from the header, Set viewing and Acknowledge requests,
//! the controllable status clock, and status records as the native
//! contract specifies them. Expected values are written here by hand from
//! the header, the native contract and the daemon wire reference.

use super::control::{CURRENT, INTERACTIVE};
use serde_json::{Value, json};
use std::path::Path;

pub const STATUS: u64 = 4;
pub const VIEWING_RESULT: u64 = 6;
pub const ACKNOWLEDGEMENT: u64 = 7;
pub const IDLE: u64 = 1;
pub const RUNNING: u64 = 2;
pub const NEEDS_ATTENTION: u64 = 3;
pub const ACKNOWLEDGED: u64 = 4;
pub const COMPLETED: u64 = 5;
pub const STATUS_FAILED: u64 = 6;

/// The controllable clock's start: 2027-01-15T08:00:00.000Z.
pub const T0: u64 = 1_800_000_000_000;

/// Starts the controllable clock at `clock` at [`T0`].
pub fn start_clock(clock: &Path) {
    std::fs::write(clock, T0.to_string()).unwrap();
}

/// A step that sets the controllable clock at `clock` `ms` after [`T0`]
/// and waits until the daemon's turn that read it has queued its records.
pub fn advance(clock: &Path, ms: u64) -> Value {
    json!({ "do": "clock", "path": clock.to_str().unwrap(), "value": T0 + ms })
}

/// An interactive Open with public `id` at `cols` x `rows`, requiring core
/// and asking for titles, status and focus.
pub fn interactive_status(id: u64, cols: u64, rows: u64) -> Value {
    json!({
        "tag": "open",
        "request_id": id,
        "open": {
            "mode": INTERACTIVE,
            "geometry": [cols, rows],
            "required": ["daemon-wire-v1"],
            "optional": ["title-v1", "status-v1", "focus-v1"],
        },
    })
}

fn authority(generation: u64, (period, restoration): (&str, &str)) -> Value {
    json!([generation, period, restoration])
}

/// Set viewing to `value` under the instance's `which` authority.
pub fn viewing(id: u64, generation: u64, which: (&str, &str), value: u64) -> Value {
    json!({
        "tag": "viewing",
        "request_id": id,
        "authority": authority(generation, which),
        "value": value,
    })
}

/// Acknowledge under the instance's `which` authority.
pub fn acknowledge(id: u64, generation: u64, which: (&str, &str)) -> Value {
    json!({ "tag": "acknowledge", "request_id": id, "authority": authority(generation, which) })
}

/// Set viewing to `value` under the instance's current authority.
pub fn view(id: u64, generation: u64, value: u64) -> Value {
    viewing(id, generation, CURRENT, value)
}

/// A native status: its state, episode (present exactly when given) and
/// exact change time.
pub fn status(state: u64, episode: Option<u64>, changed_at: &str) -> Value {
    json!({
        "state": state,
        "episode_present": u64::from(episode.is_some()),
        "episode_id": episode.unwrap_or(0),
        "changed_at": changed_at,
    })
}

/// A native available status.
pub fn available(status: Value) -> Value {
    json!({ "available": 1, "reserved": 0, "value": status })
}

/// The native unavailable status: availability alone.
pub fn unavailable() -> Value {
    json!({ "available": 0, "reserved": 0 })
}

/// Set viewing's native result: changed, the viewing now, no bytes.
pub fn viewed(changed: u64, value: u64) -> Value {
    json!({ "changed": changed, "value": value, "reserved": [0, 0], "accepted_bytes": 0 })
}

/// Acknowledge's native result.
pub fn acknowledged(changed: u64) -> Value {
    json!({ "changed": changed })
}
