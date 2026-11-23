# Configuration

Every setting in the config file, its default and its allowed values.

climon2 reads an optional `<climon2 home>/config.jsonc`. These are the
defaults:

```jsonc
{
  "client": {
    // Ctrl+[ is Escape: it delays bare Escape, and in the monitor legacy Alt+s/x/j/? may trigger the shortcuts.
    "escape_prefix": "Ctrl+]",     // null disables escape interception in ordinary attach; the monitor refuses null
    "take_control_key": "Ctrl+T"   // null disables keyboard takeover
  },
  "scrollback": {
    "max_rows": 10000
  },
  "monitor": {
    "show_key_hints": true,        // false hides the monitor's implicit key hints, never a binding
    "bell": true,                  // false: no host bell when a toast's title first shows
    "limits": {
      "memory": {
        "total_bytes": 603979776,
        "client_bytes": 67108864,
        "history_bytes_per_session": 16777216,
        "history_bytes_total": 268435456,
        "queue_bytes_total": 67108864
      },
      "history": { "max_rows": 10000 },
      "resources": {
        "stack_bytes": 2097152,
        "max_workers_per_client": 2,
        "descriptors_per_client": 8,
        "descriptors_per_load": 4,
        "descriptor_headroom": 64,
        "release_bytes": 67108864
      },
      "queues": {
        "request_bytes": 262144,
        "request_records": 64,
        "event_bytes": 1048576,
        "event_records": 1024,
        "progress_bytes": 65536,
        "progress_records": 32,
        "deliveries": 8,
        "operations": 16
      },
      "transfer": { "chunk_bytes": 65536, "drain_records": 64, "drain_payload_bytes": 65536 },
      "payload": {
        "wire_frame_bytes": 1048576,
        "metadata_record_bytes": 16384,
        "title_bytes": 4096,
        "notification_title_bytes": 4096,
        "notification_body_bytes": 8192,
        "assembly_records": 32,
        "assembly_bytes": 65536
      },
      "scheduling": { "probes": 8, "loads": 2, "starts": 8, "newer_start_burst": 8 },
      "discovery": {
        "pass_ms": 30000,
        "stall_ms": 5000,
        "batch_records": 128,
        "batch_bytes": 1048576,
        "metadata_file_bytes": 262144,
        "metadata_depth": 32
      },
      "deadlines": {
        "load_ms": 10000,
        "start_ms": 10000,
        "operation_ms": 5000,
        "kill_ms": 10000,
        "restoration_stall_ms": 5000,
        "restoration_total_ms": 60000,
        "delivery_stall_ms": 5000,
        "leave_ms": 5000,
        "cleanup_ms": 10000,
        "shutdown_ms": 10000,
        "final_host_ms": 1000,
        "assembly_ms": 5000
      },
      "retry": { "initial_ms": 1000, "cap_ms": 30000, "jitter_percent": 20, "admission_ms": 1000 },
      "ui": {
        "fps": 30,
        "client_quantum_ms": 2,
        "background_quantum_ms": 8,
        "search_bytes": 4096
      }
    }
  },
}
```

**The file format.** It is JSON that also allows `//` and `/* */` comments
and trailing commas. A comment counts as a space: it can split two tokens but
never joins them. So `1/* c */0000` is an error, not `10000`.

**Missing and unknown settings.** A missing file, object or field means the
default. Fields climon2 does not know are ignored.

**Keys.** A key setting can be `Ctrl+A` to `Ctrl+Z`, `Ctrl+[`, `Ctrl+\`,
`Ctrl+]`, `Ctrl+^`, `Ctrl+_` or `Ctrl+Space`. `null` turns the binding off.
`take_control_key` cannot be `Ctrl+[` (that is Escape), and the two bindings
cannot be the same key. If `escape_prefix` is `null`, there is no detach key,
so closing the last client ends the session.

**Scrollback.** `scrollback.max_rows` (0 to 100000, default 10000) is how many
rows of normal-screen history a session keeps for redrawing. It is set when the
session starts; attaching never changes it. 0 keeps no history, but the screen
itself is still redrawn.

**Key hints.** When `monitor.show_key_hints` (default true) is false, the
[monitor](monitor.md) hides the key hints it adds on its own. It never hides a
binding or text from an app. Hints appear in the empty screen, the top row,
the placeholder, the session list footer and toasts. False also stops the key
introduction from opening at start-up. The escape prefix then `?` still opens
it. Your **Don't show again** choice is stored apart from this setting and
never changes it.

**Bell.** When `monitor.bell` (default true) is false, toasts do not ring
your terminal's bell. Toasts and the monitor's attention reports to your
terminal still appear.

**Monitor limits.** `monitor.limits` holds the
[operational limits](../behaviour/monitor/operational-limits.md):
sizes in bytes, times in milliseconds (`_ms`), and counts. The monitor checks
and reports all of them, and enforces most. That document says which ones it
does not enforce yet. `history.max_rows` here is the monitor's own history; it
never changes `scrollback.max_rows`.

Each limit is a whole number in this range:

- Times: 1 to 3600000. But `ui.client_quantum_ms` and
  `ui.background_quantum_ms` are 1 to 50.
- `history.max_rows`: 200 to 100000. `ui.fps`: 1 to 120.
  `retry.jitter_percent`: 0 to 50. `resources.max_workers_per_client`: 0 or
  more.
- Everything else: at least 1. The upper bound is 4294967295 for counts, for
  `transfer.chunk_bytes`, and for the `payload` byte limits other than
  `assembly_bytes`. Other byte limits go up to 18446744073709551615.

The limits must also fit together:

- These must each fit inside `memory.total_bytes`: `memory.client_bytes`,
  `history_bytes_total`, `queue_bytes_total`, the `resources.stack_bytes` and
  `release_bytes` reserves, and every monitor byte limit not already inside a
  smaller total (`transfer.drain_payload_bytes`, `discovery.batch_bytes`,
  `discovery.metadata_file_bytes` and `ui.search_bytes`).
- `history_bytes_per_session` must fit inside both `history_bytes_total` and
  `client_bytes`.
- `queues.request_bytes` and `event_bytes` must each fit inside both
  `memory.queue_bytes_total` and `memory.client_bytes`.
- `payload.assembly_bytes` and `assembly_records` limit how the daemon builds
  notifications. They are not monitor memory, so no monitor total includes
  them.
- `queues.progress_bytes` must be less than `event_bytes`, and
  `progress_records` less than `event_records`. This leaves room for normal
  work.
- `transfer.chunk_bytes` must fit inside `queues.request_bytes`, inside the
  normal event space (`event_bytes` minus `progress_bytes`), and inside
  `transfer.drain_payload_bytes`. `payload.metadata_record_bytes` must fit
  inside `progress_bytes`, that normal event space, and
  `drain_payload_bytes`.
- `ui.client_quantum_ms` must not be more than `ui.background_quantum_ms`, and
  `retry.initial_ms` not more than `retry.cap_ms`.

`memory.client_bytes` is shared, per session, between the client library and
the monitor's copy of the session's terminal (its **terminal model**). The
library's share is its request and event queues, one wire frame, and its
1048576-byte instance memory. That is 3407872 bytes by default, and it must fit
inside `client_bytes`. The terminal model gets the rest.

The monitor also refuses limits too small for one session's terminal model at
the largest session size, 500x200. (`exec` does not check this.) Such a model
needs 16966087 bytes for its current screen plus the history that holds its
latest page. So:

- `memory.client_bytes` must hold that plus the library's share (20373959
  bytes by default).
- `memory.total_bytes` must hold all that plus the monitor's one shared parser
  (22471111 bytes by default).
- `memory.history_bytes_total` and `history_bytes_per_session` must hold that
  history: 12115200 bytes.

The error names the first limit to raise. Limits that only just pass will let
such a session in. But to redraw it next to its current model, the model's
share of a client's memory needs at least 33932174 bytes (36029326 in the total
with the parser, plus the library's share). The monitor redraws whenever the
session changes size.

**Who reads what.** `exec` checks the whole file before it starts the session.
`attach` checks only `client`. The monitor checks `client` and `monitor`, and
refuses a `null` `escape_prefix`. `list` and `kill` do not read the file.

Any error exits 125 before anything starts. Errors include: bad JSONC,
repeated keys, wrong types, `null` for a setting that cannot be turned off,
bad key names, two bindings on one key, values out of range, limits that do
not fit together, and, for the monitor, limits too small for its terminal
model.

Each command reads the file once. There is no environment variable to
override it, it is never created for you, and changes are not picked up while
a command runs.
