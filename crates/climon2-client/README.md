# climon2-client

The version-matched terminal bridge. Its frozen invocation is:

```text
<executable home>/<version>/climon2-client attach --metadata <absolute path>
```

`run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error>`
excludes argv[0]. `Error` exposes `code`, `message`, and optional absolute `logs`.
The thin binary prints one diagnostic after all terminal guards have dropped.
`CLIMON2_HOME` and `CLIMON2_LOG` are configuration inputs from the environment;
the command's environment is never used for internal descriptors.

Every invocation writes its own session log,
`<home>/logs/<session-id>/client-<UTC YYYYMMDDTHHMMSSZ>-<pid>.log`, created
`0600` with `create_new` and `O_NOFOLLOW`. No client truncates, deletes, shares
or reopens another client's log, so concurrent and later clients of one session
never collide; a collision would be an ordinary I/O failure. In kill mode the
client library instance writes its own diagnostics beside it, in
`<home>/logs/<session-id>/library-<UTC YYYYMMDDTHHMMSSZ>-<pid>.log` (the
client's pid), which the library creates the same way, under the client's
`CLIMON2_LOG` filter or the default. Failures after the log exists carry the
` (logs: <session-log directory>)` suffix, the directory holding both.

The operation comes from the `CLIMON2_CLIENT_STARTUP` startup envelope (core
`startup_envelope`, at most 4096 bytes), which the CLI sets: `attach` with the
exact validated client settings, or `kill`. A present envelope is used exactly
or rejected. Without one, a directly started client reads and validates the
`client` section of `config.jsonc` and attaches.

Both operations go through this release's own
[client library](#client-library-libclimon2_clientdylib-libclimon2_clientso)
over daemon wire revision 1, the only protocol session daemons speak after
hello since the version-private one was retired (#104): ordinary
attach since #101 (`src/attachment.rs`, `src/bridge.rs`) and kill since #100
(`src/kill.rs`); `src/embedded.rs` loads and drives the library for both.
The executable loads `<executable home>/<version>/<library file>`, beside
itself, through the shared loader (`climon2_platform::library`), with eager,
local resolution; its release is the version folder it runs from, which the
library must describe. `current`, the working directory and search paths never
choose the library, and no other version's library is ever loaded. A missing,
other-release or faulty library, or an executable outside a version folder,
fails with status 125 and the loader's diagnostic before any connection.

The executable then creates one instance on an owner thread of its own
(generation 1, the session's metadata, home and socket directory, no client
keys, its own log file beside the client's under the client's filter, the
fixed limits below). **Kill mode** (#100) opens no terminal and no
attachment: it submits one Kill (public
ID 1, requiring nothing beyond core support) and drains until the instance's
stream ends. The library sends the Kill on its own hello-validated connection
over daemon wire revision 1 and gives it one definitive outcome. A confirmed
termination returns 0 whatever the command's status. Every failure is status
125 with the diagnostic kill gave before #100 (owner decision on #100):
- a Kill sent and then outliving `kill_ms`, or never answered:
  `termination of session <id> was not confirmed`, the CLI's message for its
  own ten-second bound;
- a hello the daemon leaves unanswered, which fails at the frozen two-second
  hello deadline and never sends the Kill: `session <id> did not respond`;
- a connection that cannot be made, closes (also inside a frame) or fails,
  during the hello or after the Kill (the library's transport `io_error` or
  `truncated_frame`): `lost connection to session <id>`;
- anything else, such as a refusal or a partial termination: the library's
  message, which for a daemon error, even one coded `io_error`, is the
  daemon's own.

HUP/TERM/INT while waiting return
128+signal at once, without waiting for the Kill's answer; the daemon finishes
a committed termination anyway. Otherwise the executable releases its handles,
and the owner thread destroys the instance and is joined. The CLI still bounds
the whole operation.

The executable never reads `monitor.limits`; every instance it creates gets
this fixed `cm1_limits` table (`embedded::LIMITS`), the operational-limits
per-client defaults except `restoration_total_ms` at its maximum:

| Field | Value |
|---|---|
| `client_bytes` | 67,108,864 |
| `request_bytes` / `request_records` | 262,144 / 64 |
| `event_bytes` / `event_records` | 1,048,576 / 1,024 |
| `progress_bytes` / `progress_records` | 65,536 / 32 |
| `deliveries` / `operations` | 8 / 16 |
| `chunk_bytes` | 65,536 |
| `drain_records` / `drain_payload_bytes` | 64 / 65,536 |
| `wire_frame_bytes` | 1,048,576 |
| `metadata_record_bytes` | 16,384 |
| `title_bytes` / `notification_title_bytes` / `notification_body_bytes` | 4,096 / 4,096 / 8,192 |
| `max_workers` | 2 |
| `operation_ms` / `kill_ms` | 5,000 / 10,000 |
| `restoration_stall_ms` / `restoration_total_ms` | 5,000 / 3,600,000 |
| `delivery_stall_ms` / `leave_ms` / `cleanup_ms` / `start_ms` | 5,000 / 5,000 / 10,000 / 10,000 |
| `reserved` | 0 |

`attach_passes_exactly_the_published_limits_table` compares the limits the
executable passes, recorded by a stand-in library, with this table, parsed from
this README; kill mode's tests observe `kill_ms`, and the attach tests observe
`operations` (unanswered Input stops the terminal being read),
`wire_frame_bytes` and `metadata_record_bytes` (frames beyond them fail) and
the backpressure of `event_bytes`.

**Attach mode** (#101): the instance's one Open (public ID 1) is interactive,
at the terminal's size, requires `daemon-wire-v1` and asks for `status-v1`
alone, optionally (#102): never title, notification, progress or focus. The
library names `status-v1` in the Open only when the daemon advertises it, so
against a daemon without it the Open and everything after it are unchanged.
The library checks the
session's metadata and hello (session, protocol, release, revision-1
advertisement) before any request; its refusal, or the daemon's, is one
diagnostic before raw mode. Raw mode begins only after the Open succeeds. The
executable then submits, in order and with increasing public IDs, Input under
the held authority, Resize, Acquire (the take-control key), Ready, Set
viewing, Acknowledge and Leave (detach); a command the library has no room
for waits, and so does the
terminal. The library never interprets Input as keys: the executable owns the
host terminal, raw mode, escape-prefix routing, detach, the take-control key,
the placeholder and presentation release. A controller waits for Control
granted, writes the restoration and live output, drains and discards input
typed during restoration (a replacement Restoration begin included), then
sends Ready once typed-ahead keys are drained and the restoration is written;
application input is sent only after that. An observer holds no authority to
resize under, so the session hears its size once it holds control again. A revoked client shows the
placeholder: the first line `This session is being viewed elsewhere.` and, when
a take-control key is configured, `Press <key> to take control`. It is redrawn
on resize. That key takes control; observer input is never sent. The escape
prefix (default Ctrl+]) followed by `d` requests an explicit detach (Leave)
from either role; after a confirmed Leave of the attachment the client prints
`climon2: detached from session <id>; reattach with: c2 attach <id>` on the
restored terminal and returns 0. Bracketed paste spanning a role change is
discarded through its closing marker, never replayed or read as keys.

**Viewing and acknowledgement** (#102, a derived decision of spec #73): the
status mechanics list an explicit ordinary attach as an acknowledgement
trigger. After each Ready the executable, now the ready controller, reports
Set viewing true under the same authority. After the Ready that follows an
explicit attach or takeover (a grant with reason `attach` or `acquire`) it
then submits one Acknowledge, which moves pending attention to acknowledged;
the daemon judges whether anything was pending. A hand-back (`hand_back`)
is not an explicit action and never acknowledges, nor does a replacement
restoration within the same period. Nothing is reported before readiness.
Detaching while holding an authority reports viewing false before the Leave.
A revocation sends nothing: the revoked client is no longer the controller,
and the daemon starts every control period not viewing, so revocation clears
viewing there. When `status-v1` was not selected the library refuses Set
viewing and Acknowledge locally (`unsupported_capability`) and sends
nothing; the executable ignores those answers, as it ignores every answer
but the Open's. Results never change what the executable shows.

**Focus is assumed, best effort.** The executable does not track host
terminal focus. Focus reports, when the application enables them, reach the
application as input and are never interpreted here, so a ready controller
counts as viewing even while its terminal window is in the background, and
an attach or takeover the user is not looking at still acknowledges
attention.

The event loop polls the library's notification descriptor, the terminal and
stdout. Keys are read only while every earlier command was accepted, so a
full library stops reading the terminal. The client receives one delivery at
a time while less than 65,536 bytes of output wait for the terminal, so
backpressure reaches the library, which stops reading the session once its
fixed event allowance is full, while the end of the session is still seen. A
pending detach, the end of the stream or a hung-up session (one that no
longer accepts commands) has a bounded five-second closing deadline (`detach
not confirmed` or `session result not confirmed`), serviced even while
terminal output is blocked.

SIGWINCH is coalesced and unchanged sizes are suppressed. HUP/TERM/INT end the
process without a Leave, closing the connection, and return 128+signal. Stdin
EOF/EIO silently returns 129. Ctrl-C, Ctrl-Z, binary bytes and all terminal
control sequences are forwarded unchanged. The command's status comes from its
Command final, or from the Initial metadata when it ended before the attach,
and is returned once the stream ends and pending output has drained: the exit
code, or 128 plus the signal's number, as before. A transport loss (a close
between or inside frames without a known result) is
`lost connection to session <id>`; a daemon's fatal error is its own message. Termios and nonblocking flags
are restored on normal exit, errors, signals, and unwinding. Leave policy
belongs exclusively to the daemon.

The daemon's release bytes return the terminal to its normal-shell baseline. A
shown placeholder is ended below itself (cursor to the last row, CR LF) when the
stream ends (the command's status, a detach or an error), and with one
non-blocking attempt on a termination signal. A session-ending revocation
takes control away without a placeholder.
Only when no daemon response arrives (unexpected connection loss, or the
five-second deadline expiring) does the client write its conservative reset:
synchronized update ended, `CSI 0 m`, cursor shown, mouse, focus and bracketed
paste off, application cursor keys and keypad off. It never leaves the
alternate screen blindly, so alternate-screen and other unknown state (for
example the title) cannot be restored without a daemon response. On expiry
pending output is discarded and the reset gets one non-blocking attempt.

## Client library (`libclimon2_client.dylib`, `libclimon2_client.so`)

This package's library target is also a cdylib (`crate-type = ["lib",
"cdylib"]`), so every release ships the client library in its version folder,
published atomically by the installer: `libclimon2_client.dylib` (universal
arm64/x86_64) on macOS and `libclimon2_client.so` on x86-64 GNU/Linux. The
file name comes from `climon2_core::layout::client_library_name`. It is an
artifact, not another crate. It exports only the frozen
`climon2_client_get_interface` bootstrap of
[native client interface revision 1](../../docs/protocol/monitor-client/core-v1.md),
using the shared `#[repr(C)]` declarations in `climon2_core::native`; Rust
layouts, unwinding and allocators never cross it. Every entry contains panics
and reports `CM1_INTERNAL_ERROR` instead; the bootstrap installs a silent panic
hook in the library's own image (not the host's), so nothing reaches the
standard streams.

Requesting revision 1 returns one static, immutable description that lives
until process exit, and the same pointer on every call:

- `release`: the version folder name, compiled in (`CARGO_PKG_VERSION`). A copy
  placed in another version's folder still reports its own release.
- `capabilities`: `daemon-wire-v1`, `monitor-v1`, `title-v1`, `status-v1`,
  `notification-v1`, `progress-v1` and `focus-v1`, exactly the families the
  library relays (#93, #94, #95 and #98, `src/library/families.rs`): every
  revision-1 family.
- `requirements`: provisional bounded values until the measurement job (#121)
  records qualified ones. Each equals its default `monitor.limits` reservation
  where one exists, plus 1 MiB of instance-local allocation, so a
  default-configured monitor admits the release without raising a reservation:

  | Field | Value | Bound |
  |---|---:|---|
  | `release_bytes` | 67,108,864 | `resources.release_bytes` |
  | `instance_bytes` | 1,048,576 | nonzero, within `memory.client_bytes` |
  | `owner_stack_bytes` / `worker_stack_bytes` | 2,097,152 each | nonzero, within `resources.stack_bytes` |
  | `workers` | 2 | `resources.max_workers_per_client` |
  | `instance_descriptors` | 8 | nonzero, within `resources.descriptors_per_client` |
  | `load_descriptors` | 4 | `resources.descriptors_per_load` |

- `functions`: all eleven entries; the instance lifecycle below (#85),
  passive monitor connections (#93), control and display (#94), and status,
  viewing and acknowledgement (#95). Failed calls report library
  origin, a stable error code and a static message (so error text outlives
  every borrower), zero their nonnull output structs (`run` keeps its report),
  and `notification` writes -1. A null handle or output pointer, the one
  invalid address the library can detect, is `CM1_INVALID_ARGUMENT`.

Any other revision returns `CM1_UNSUPPORTED_REVISION` and writes a null
description; null output storage returns `CM1_INVALID_ARGUMENT`. The bootstrap
carries the `test_hook_marker!`, so hooked and release builds stay
distinguishable.

### Instance lifecycle (#85)

Each instance is independent: its own state and lock, its own diagnostics, no
process-global lock, and no change to the environment, working directory,
signal handlers or standard streams.

- `create` runs on the thread that becomes the instance's owner. It parses
  every startup field, copying the session ID that Open checks (#93), before
  creating anything (`src/library/startup.rs`): the
  expected release must be this library's (else `CM1_RELEASE_MISMATCH`); the
  generation is nonzero; metadata, home and runtime paths are absolute native
  Unix paths, copied byte for byte (invalid UTF-8 is kept; NUL, another
  encoding or `PATH_MAX` bytes or more are rejected); keys are enabled with a
  C0 byte or disabled with zero, through core's `ClientSettings`; the
  diagnostics filter is nonempty `CLIMON2_LOG` grammar, parsed once into
  core's `diagnostics::Filter` even without a log file;
  reserved fields are zero. Limits are effective values: every field but
  `max_workers` is nonzero, `client_bytes` covers the described
  `instance_bytes`, `max_workers` the described `workers`, and
  `drain_payload_bytes` at least `metadata_record_bytes`, else
  `CM1_UNSUPPORTED_LIMIT`; accepted limits are returned unchanged. Malformed
  input is `CM1_INVALID_ARGUMENT`. Then it creates the notification descriptor,
  the waker and, last, the optional log file at the caller's absolute path (`create_new`,
  mode `0600`, `O_NOFOLLOW`; a path that exists or whose directory is missing
  is `CM1_INVALID_ARGUMENT`), so any failure returns no handles and leaves no
  descriptor (no descriptor left, for the notification descriptor, the waker
  or the log file: `CM1_RESOURCE_LIMIT`). No daemon I/O, no worker.
- The log uses core's subscriber with the instance's filter and the
  session-log cap (`CappedWriter`), through the instance's own dispatcher;
  every record carries a `client_instance{generation=N}` span. Records are
  lifecycle facts only; `client instance running` (debug) names the copied
  paths and keys.
- `run` is owner-thread only (`CM1_WRONG_THREAD`) and once (a repeated run is
  `CM1_CLOSED`). The instance is only ever borrowed shared, so a call from
  another thread is refused safely while the owner thread runs. It executes
  accepted requests and the connection until the stream ends (below),
  publishes terminal evidence after any queued record,
  then waits up to `cleanup_ms` for its workers. The
  report is separate from terminal evidence: complete cleanup is
  `CM1_CLEANUP_COMPLETE`; a worker still running is `CM1_CLEANUP_INCOMPLETE`
  with `CM1_TIMEOUT`, and that instance can never be destroyed. Without Open,
  terminal evidence is stopped with no retry after shutdown, Leave is not
  required and every connection is closed.
- `shutdown` is idempotent intent from any thread, and may precede `run`. It
  records the intent before anything that can fail, so even a failed shutdown
  ends the stream with no retry.
- `notification` returns the same borrowed descriptor every time: on Linux one
  `eventfd` (close-on-exec, nonblocking), on macOS the read end of a
  close-on-exec pipe (two descriptors), both from climon2-platform's
  `notifier` seam. It is readable while a record or terminal evidence is
  ready, and `drain` clears it when neither is. `destroy` closes it. Each instance also holds a private waker of
  the same kind, which `submit`, `shutdown` and `release_delivery` raise for
  the owner thread.
- `drain` returns one delivery, empty, or terminal evidence once earlier
  records are drained, repeatedly. `release_delivery` releases a delivery
  once. `release_communication` is `CM1_BUSY` until the stream has ended for
  its consumer and every delivery is released; `release_control` until the
  stream has ended. `destroy` (owner thread) is `CM1_BUSY` until `run` returned
  with complete cleanup and both handles are released; a failed release or
  destroy leaves its object valid.
- `snapshot` reports the instance's allocation, queued requests and their
  bytes, outstanding operations, queued and delivered records and their bytes,
  deliveries, workers and descriptors (the notification and waker descriptors,
  plus one for a log file and one for an open connection), with peaks, and the
  drained and released counters. No shared-release allocation is tracked;
  input and display measurements stay zero. Records and deliveries are
  accounted in one place: a record from its queueing until its delivery's
  release.
- Allocation is measured at the allocator
  ([#180](https://github.com/jackgeek/climon2/issues/180),
  `src/library/allocation.rs`). The library image's global allocator prefixes
  every block with the ledger it is charged to: an instance's `create` opens
  its ledger first, and each call's work on the instance runs charged to it,
  so its structures, copied paths, queued requests and records, deliveries,
  waker, connection and its stream, diagnostics dispatcher (subscriber,
  registry and filter) and any dependency's internal allocations are counted,
  prefix included, until freed on any thread. The ledger counts its own
  block, measured once by the first `create`, outside any charge. A block
  resized while working for its own instance stays charged; resized for
  another instance, or for none, it moves to the shared-release allocation,
  which is not tracked yet. Workers start through one helper that charges the
  whole thread, its thread-wide state included, to its instance; only its
  thread handle, which the standard library allocates from the system
  allocator directly, is in no ledger.
- Process- and thread-wide dependency state is never an instance's
  (`src/library/warm.rs`). Bootstrap creates tracing's process-wide
  dispatcher list through a dispatcher of the library's own that writes
  nowhere, and enters one span through it, which takes the span registry's
  process-wide thread-ID locks first (on macOS a `std::sync::Mutex` allocates
  its pthread mutex when first locked); with `test-hooks`, it also reserves
  the injected-panic list. Before a
  caller's thread first works for an instance, it is warmed outside any
  charge: the record-formatting buffer the thread keeps is created.
- `#[global_allocator]` replaces the allocator of every artifact that links
  the crate: the library, the `climon2-client` executable and the package's
  own test binaries. Outside any instance nothing is charged; each block
  still carries its prefix.
- A panic in an instance call, `submit` included, stops that instance: the
  call fails with
  `CM1_INTERNAL_ERROR`, one library `CM1_DIAGNOSTIC` record (`internal_error`)
  is queued when it fits `event_bytes`, and terminal evidence is failed with
  that error, retried passively, or not at all when shutdown was requested
  first or the panic is in `shutdown` itself.
  After terminal evidence a panic changes nothing else.

With `test-hooks` only (`src/library/hooks.rs`), `CLIMON2_LIBRARY_FAULT`
selects faults: `panic:<call>[,<call>...]` panics once at each named export's
boundary (`bootstrap` names `climon2_client_get_interface`), or at `receive`,
as a running connection handles a frame from its daemon; `abort:<call>`
aborts the whole embedding process at the same points, a native crash
nothing contains (#118); `worker` makes
`run` start a worker that ends with its stream, `stall-cleanup` one whose
cleanup never ends, and `joined-worker` one that briefly holds a block of
`chunk_bytes` and ends, and that `run` joins before going on. Release builds
start no workers: an instance's connection runs on its owner thread, inside
`run`.

### Passive monitor connections (#93)

`submit` validates and copies a request once (`src/library/request.rs`),
without daemon I/O or waiting, reading only the active member: a known tag
(`CM1_OPEN` to `CM1_FOCUS`, else `CM1_INVALID_ARGUMENT`), reserved zero, a
nonzero ID (`CM1_INVALID_REQUEST_ID`), then the operation. Open, Leave,
the control requests below (#94), Set viewing and Acknowledge (#95) and Kill
(#96) are accepted. A monitor Open has
zero geometry and reserved fields; each token list is null and zero, or a
pointer and a count; tokens are nonempty UTF-8, each named once across both
lists. A required family the library does not relay is
`unsupported_capability`; an optional one is dropped. A Leave names a nonzero
generation. The request's footprint, `sizeof(cm1_request)` plus
`sizeof(cm1_bytes)` and the bytes of each token, must fit `request_bytes`
before any token is read. Admission then needs an open instance
(`CM1_CLOSED`), an ID above every accepted one, and room in
`request_records`, `request_bytes`, `operations` and, one record plus
`metadata_record_bytes` reserved for each outstanding request's outcome, in
`event_records` and `event_bytes` (`CM1_FULL`, `queue_full`). A rejection
consumes no ID and keeps nothing.

`run` executes requests in order on the owner thread, in one poll loop over
the connection's socket and the waker (`src/library/connection.rs`). Open
reads the session metadata (at most `client_bytes`), which must name the
startup's session and this release; connects to its socket, checking the
peer's UID; and exchanges the frozen hello within `start_ms` and its 2 s
limit. The reply must name the same session, protocol 1 and this release,
and advertise revision 1 validly with `monitor-v1` (monitor mode only), every required family and
server frames within `wire_frame_bytes`. Only then is the revision-1 Open
sent, with the required families and the optional ones the daemon
advertises. An Open refused before it is sent is answered `CM1_REJECTED`
with the refusal, and the stream ends. The daemon's selection must hold
every required family and nothing unrequested.

Records are Responses (opened: the connection GUID's canonical bytes and the
selected tokens; left; a known final command result), the Initial metadata
(the title unavailable unless `title-v1` is selected, the status unless
`status-v1` is (#95); progress unavailable; the command result), Title changes (only after the Initial
metadata, only with `title-v1`) and diagnostics. A frame over
`metadata_record_bytes` (type byte included) or a title over `title_bytes`
fails the connection (`resource_limit`); nothing is truncated. While the
event queue is full, the library stops reading the socket, so daemon
records wait instead of being dropped. A delivery takes the oldest records,
at most `drain_records`, stopping before `drain_payload_bytes`; create
refuses `drain_payload_bytes` below `metadata_record_bytes`
(`unsupported_limit`), so a record never exceeds it alone. Each record is
charged to `event_bytes` twice `sizeof(cm1_record)`, for the `cm1_record` and the library's
own copy (a compile-time check keeps that copy no larger), plus the bytes it
owns once: copied text and token arrays; the library's own error text is
static and costs nothing. Each outstanding request reserves one record's
charge plus `metadata_record_bytes` for its outcome.

A wire violation, a closed or broken socket, a daemon Fatal error or an
exceeded limit ends the connection: a diagnostic when it fits the event
queue, then failed terminal evidence with the error. Transport origin covers
the socket, framing and the wire's rules (`io_error`, `truncated_frame`, the
wire's framing codes, `invalid_record`, `invalid_request_id`); daemon origin
the daemon's own errors, copied; library origin the library's judgments
(`timeout`, `cancelled`, `invalid_state`, `stale_generation`,
`resource_limit`, `unsupported_limit`, `invalid_configuration`,
`release_mismatch`, `unsupported_revision`, `unsupported_capability`, and
`io_error` for unreadable metadata). No Response is fabricated for a request
lost with the connection: the caller marks its still-unanswered IDs
indeterminate. A request outliving its deadline (`start_ms` until the Initial
metadata, `leave_ms` for a Leave) is answered `CM1_INDETERMINATE` `timeout`,
and the connection then fails. Requests still queued when the stream ends are
`CM1_REJECTED` `cancelled`.

A Leave must name the startup generation (`stale_generation`); it waits for
the Initial metadata, and without a role or while another Leave is
unanswered it is `invalid_state`, as is a second Open. The daemon's answer
ends the stream, except a rejection, which keeps the registration. Shutdown
rejects queued work, waits for an Open under way, then sends one Leave of
its own; a lost, late, rejected or failed one is reported, never replayed.
An Open whose hello completes after shutdown is never sent: it is
`CM1_REJECTED` `cancelled`.

### Control and display (#94)

An interactive Open names a positive geometry and reserved zero; the daemon
attaches and grants at once, after the Initial metadata. Acquire names a
nonzero generation; Input, Resize, Ready and Focus a nonzero generation,
period and restoration ID, parsed once into core's wire types
(`src/library/request.rs`). Input carries nonempty exact bytes, at most
`chunk_bytes` (`invalid_argument`), charged `sizeof(cm1_request)` plus its
bytes against `request_bytes` (`queue_full`); Resize a positive geometry;
Focus 0 or 1 with its reserved bytes zero. Like a Leave, each runs after the
Initial metadata: another generation is `stale_generation`, no role or a
Leave under way `invalid_state`, and Focus without `focus-v1` selected
`unsupported_capability`, all locally and never sent; a frame over the
daemon's request ceiling is `unsupported_limit`, never sent. The period and
restoration are the daemon's to judge (`stale_period`, `stale_restoration`,
`not_ready`, `not_controller`, copied with daemon origin). Each command is
bounded by `operation_ms` (`timeout`, indeterminate, and the connection
fails); its success must carry its own result (`invalid_record` otherwise),
and Input's every byte.

Control granted (attach, take control, hand-back or successor), Control
revoked (takeover or session ending), Restoration begin, Display,
Restoration complete and Geometry are delivered as they arrive, each
naming the authority the role holds (`src/library/stream.rs`): a grant only
when none is held, a revocation, Display or complete only of the held
period and restoration, a beginning (which makes its restoration current)
or a Geometry only of the held period. Anything else, a record before the
Initial metadata, and an empty Display (owner decision on #81) fail the
connection with transport `invalid_record`. A Display's bytes are its
record's own, exact (`00 ff 41`, NUL, invalid UTF-8), and must fit
`chunk_bytes` (`resource_limit`); create also refuses `drain_payload_bytes`
below `chunk_bytes`, so a Display never exceeds a delivery alone. A
departure or revocation reaches the caller after the release Display the
daemon sends under the held authority; an attachment's confirmed Leave
reports the attachment detached.

Terminal evidence is stopped, without an error, after a confirmed departure
or a shutdown before any role; otherwise failed with the first failure's
error, or the departure's. Retry is none once shutdown is requested, once a
Leave the caller asked for is sent (answered or lost), and once the
command's final result is known (a known-final rejection or the Initial
metadata's command result); otherwise permanent for missing support,
identity, configuration or frame limits, and passive.
Leave evidence is not required when no Leave was attempted, since closing
the connection ends a passive registration; otherwise it is confirmed,
rejected, failed or, when lost or late, indeterminate. Only a confirmed
departure can report a detached attachment. Every connection is closed.
Sends never raise `SIGPIPE`: Linux sends with `MSG_NOSIGNAL`, macOS sets
`SO_NOSIGPIPE`.

### Status, viewing and acknowledgement (#95)

With `status-v1` selected, the Initial metadata's status is available and
each Status changed after it is delivered as a `CM1_STATUS` record whatever
the role holds (passive, background or revoked), with the daemon's state,
episode and stored change time copied exactly (`src/library/records.rs`),
inline, so a status record is charged its record alone. The status is final
exactly when the command's result is known: a final status without the
result, a working status with it, a Status changed before the Initial
metadata, without `status-v1` or after the final result, or a Command final
whose status presence differs from the selection fail the connection with
transport `invalid_record` (`src/library/stream.rs`). The Command final's
final status reaches `CM1_FINAL`; without `status-v1` it stays unavailable.

Set viewing (a 0/1 flag, reserved bytes zero) and Acknowledge name the
authority like the control requests and follow their rules; without
`status-v1` selected both are `unsupported_capability`, locally and never
sent. The daemon judges period, restoration, readiness and the latest
viewing (`not_ready`, `not_viewing`, copied with daemon origin). Set
viewing's success must report the viewing it asked for and maps to
`CM1_VIEWING_RESULT` (changed, value, zero bytes); Acknowledge's to
`CM1_ACKNOWLEDGEMENT` (changed, zero for the explicit no-change success).
Timeouts, losses and cancellation are as for the control requests.

### Notifications and progress (#98)

With `notification-v1` selected, each Notification after the Initial
metadata is delivered as a `CM1_NOTIFICATION` record whatever the role holds
(passive, background or revoked): its source code, its episode, present
exactly when the daemon gave one, and its source identifier, title and body
as optional text owned by the record and charged with it
(`src/library/records.rs`). A title over `notification_title_bytes` or a
body over `notification_body_bytes` fails the connection with library
`resource_limit`, never truncated; a notification with neither title nor
body fails it with transport `invalid_record`. With `progress-v1` selected,
the Initial metadata's progress is available and each Progress changed is
delivered as `CM1_PROGRESS`, its state code and its percentage, absent and
zero when null, charged its record alone. A Notification or Progress changed
before the Initial metadata or without its family selected, and an Initial
metadata whose progress availability differs from the selection, fail the
connection with transport `invalid_record` (`src/library/stream.rs`).

Loaders open the absolute `<executable home>/<version>/<library file>` with
eager, local resolution (`RTLD_NOW | RTLD_LOCAL`); `current`, the working
directory, `LD_LIBRARY_PATH` and `DYLD_FALLBACK_LIBRARY_PATH` do not change
which file loads. On macOS `DYLD_LIBRARY_PATH` can, so the shared loader
(#84) refuses an image loaded from any other file than the one requested, as
[library embedding](../../docs/protocol/library-embedding.md#artifact-and-dependency-lookup)
specifies. The macOS library's install name is its build path; nothing links
against it.

On Linux the library target links the GCC unwinder (`gcc_eh`) statically, from
the Linux-only `src/library/linux.rs`, because Rust's standard library would
otherwise need the shared `libgcc_s.so.1`; unwinding never leaves the library.
The `.so` therefore needs only `libc.so.6` and the dynamic loader, exports only
the bootstrap, and has no soname, RPATH or RUNPATH, as library embedding
requires. Linking the crate has the same effect on the client executable.

`cargo test` builds the hooked library beside the test executables in
`target/debug/deps/` (Cargo uplifts only binaries), where tests and the
installer's `Payload::from_siblings` find it without nested builds. Release
builds (`cargo build --release -p climon2-client`) uplift the unhooked library.

No `cargo test` builds the library plain, unlike the monitor's binary: this
package's tests enable `test-hooks` through their `climon2-test-term`
dev-dependency, and because the library target is a cdylib, Cargo names its
outputs without a hash, so a plain test build elsewhere would replace the
hooked library at the same path. The plain library is therefore the
per-package release build, which `cargo xtask release-check` builds in a
private target directory and checks.

| Scenario | Named test |
|---|---|
| LIB-01 Library: absolute exact-release load, one stable description | `installed_library_describes_revision_one_with_its_version_identity` (`climon2-e2e/tests/library.rs`) |
| LIB-01 Library: with copies named by `current`, in the working directory and on ambient search paths, the absolute path maps the requested file (the loader's lookup, which no library change can alter); the library answers from its own image with its own release | `installed_library_loads_by_absolute_version_path_whatever_current_cwd_and_search_paths` (`climon2-e2e`) |
| LIB-02 Library: a misplaced copy describes its compiled-in release, so the shared loader refuses it as the wrong release | `installed_library_of_another_release_or_none_is_refused_without_substitution` (`climon2-e2e`) |
| LIB-02 Library: unsupported revisions and null output storage | `installed_library_rejects_other_revisions_without_a_description` (`climon2-e2e`) |
| DOC-01: null handles and outputs are rejected, `submit`'s null request included, and failed calls clear their outputs, by the frozen call rules | `installed_library_table_rejects_null_arguments` (`climon2-e2e`) |
| LIB-03, LIB-04 and LIB-05 Library: create, run, shutdown, drain to terminal evidence, release, destroy and join; run and destroy on the wrong thread, a repeated run, destroy and handle release while busy; terminal evidence apart from the cleanup report | `installed_library_instance_runs_create_run_shutdown_drain_release_destroy_join` (`climon2-e2e/tests/instance.rs`) |
| LIB-03 Library: a failed create returns no handles and leaves no descriptor: null arguments, a mismatched release, the generation, native paths | `installed_library_create_rejects_invalid_identity_and_paths_without_handles` (`instance.rs`) |
| LIB-03 Library: client keys, log path and filter | `installed_library_create_rejects_invalid_settings_and_diagnostics_without_handles` (`instance.rs`) |
| LIB-03 Library: limits it cannot honour, each at its bound, including `drain_payload_bytes` below `metadata_record_bytes` or `chunk_bytes`; no descriptor left for the notification and waker descriptors or for the log file, one more is enough | `installed_library_create_rejects_limits_it_cannot_honour_and_exhausted_descriptors` (`instance.rs`) |
| LIB-03 Library and Fault: caller startup memory scrambled and unmapped after create; lossless native paths; two instances' homes, settings and logs stay their own | `installed_library_instances_keep_their_own_copies_paths_settings_and_logs` (`instance.rs`) |
| LIB-03 Library and Fault: environment, working directory, signals and standard streams unchanged | `installed_library_leaves_environment_cwd_signals_and_streams_unchanged` (`instance.rs`) |
| LIB-04 Library and Fault, Linux `eventfd`: the descriptor stays borrowed and counted once; in a scheduled race, polling sees the stream end while the handle is used before shutdown and after the end, and every call succeeds | `installed_library_notification_descriptor_is_borrowed_and_safe_to_poll_racing_the_end` (`instance.rs`) |
| LIB-04 Fault (allocator pairing): a delivery stays charged until released through the library, which returns exactly what the drain charged | `installed_library_charges_a_delivery_until_it_is_released` (`instance.rs`) |
| LIM-02 and LIB-03 Library, x86-64 GNU/Linux: `snapshot`'s current and peak allocation equal the host's own count of the library's heap, with logging on (its dispatcher included) and off, and with a joined worker whose `chunk_bytes` block sets the peak (the peaks differ by at most the std thread handle) | `installed_library_snapshot_counts_every_allocation_its_instance_holds` (`instance.rs`) |
| LIM-02 and LIB-03 Library, x86-64 GNU/Linux: a passive monitor connection's allocation is its instance's: once run returned with both records still delivered, current and peak equal the host's count, with logging on and off | `installed_library_snapshot_counts_a_passive_connections_allocation` (`instance.rs`) |
| LIB-03 Library: a whole second instance's life leaves the first's current and peak allocation exactly unchanged | `installed_library_instances_never_change_each_others_allocation` (`instance.rs`) |
| LIB-03 Library: successive instances on one owner thread differ by exactly their homes' length; nothing process- or thread-wide is charged to the first | `installed_library_charges_successive_instances_only_for_their_own_differences` (`instance.rs`) |
| Allocator unit seam: a ledger holds its own block; a block resized for its own instance stays charged in one step, for another or none moves off | `a_new_ledger_holds_its_own_block`, `a_block_stays_charged_only_while_resized_for_its_own_instance` (`src/library/allocation.rs`) |
| Warm unit seam: a caller's thread keeps its formatting buffer uncharged; a thread an instance owns is charged for its own | `a_callers_thread_is_warmed_uncharged_and_an_instances_own_is_not` (`src/library/warm.rs`) |
| LIB-04 Fault: the failure diagnostic is queued only within `event_bytes` | `installed_library_queues_the_failure_diagnostic_only_within_event_bytes` (`instance.rs`) |
| LIB-05 Fault: a panic at the bootstrap or `create` fails only that call | `installed_library_contains_a_panic_at_the_bootstrap_or_create` (`instance.rs`) |
| LIB-05 and LIB-04 Fault: a panic in a running instance, at `submit` too, stops it, with a diagnostic delivery and failed terminal evidence (no retry from `shutdown` on); a delivery held past terminal evidence alone keeps communication and destroy busy | `installed_library_contains_a_panic_in_a_running_instance_and_stops_it` (`instance.rs`) |
| LIB-04 and LIB-05 Fault: a panic in a release or destroy keeps the object valid | `installed_library_contains_a_panic_in_a_release_and_keeps_the_object` (`instance.rs`) |
| LIB-05 Fault (`worker` hook): worker completion reported apart from terminal evidence | `installed_library_reports_worker_completion_apart_from_terminal_evidence` (`instance.rs`) |
| LIB-06 Fault (`stall-cleanup` hook): stalled worker cleanup is incomplete, never complete, and the instance is never destroyed | `installed_library_reports_stalled_worker_cleanup_as_incomplete_and_never_destroys` (`instance.rs`) |
| CTL-01, CTL-03, COM-01, COM-03, LIB-04 and EXT-01 Library: passive registration with a real daemon, controlled or detached, never acquires control; Response, atomic Initial metadata and gap-free titles; explicit and shutdown Leave confirmed once | `installed_library_registers_passively_while_controlled_elsewhere_and_relays_titles`, `installed_library_registration_never_acquires_a_detached_session`, `installed_library_explicit_leave_departs_once_and_is_answered` (`climon2-e2e/tests/passive.rs`) |
| EXT-02 Library: a Leave a stopped real daemon never answers is unconfirmed (`timeout`, Leave indeterminate, failed) | `installed_library_reports_a_lost_leave_to_a_stopped_daemon_as_unconfirmed` (`passive.rs`) |
| COM-01 Library: frames over `wire_frame_bytes` rejected before any role | `installed_library_rejects_a_daemon_whose_frames_exceed_wire_frame_bytes` (`passive.rs`) |
| COM-01 and COM-02 Library: an Open naming no family selects none, as a null, zero-count list | `installed_library_opens_naming_no_family_and_selects_none` (`passive.rs`) |
| LIB-05 Library and Fault: a stalled instance never stops another; `submit`, `drain` and release return at once | `installed_library_instance_stalled_by_its_daemon_never_stops_another` (`passive.rs`) |
| COM-01 Fault: missing support, mismatched identity, unreachable or silent daemons rejected before any Open is sent; families intersect | `installed_library_rejects_an_open_the_session_cannot_honour_before_any_role`, `installed_library_selects_only_families_both_sides_support` (`climon2-e2e/tests/passive_faults.rs`) |
| COM-02 Fault: forbidden, malformed, truncated, oversized and out-of-order traffic fails the connection with its origin and code; limits exactly met are accepted | `installed_library_fails_the_connection_on_traffic_revision_one_forbids`, `installed_library_accepts_records_exactly_at_their_limits` (`passive_faults.rs`) |
| COM-03, EXT-01, EXT-02 and REC-01 Fault: timeouts, lost answers without fabrication or replay, refusals and their retry, known final results (a later close a clean stop), shutdown and explicit Leave evidence | `installed_library_reports_lost_and_late_answers_truthfully_without_replay`, `installed_library_reports_a_known_final_result_and_never_retries`, `installed_library_shutdown_leaves_once_and_reports_unconfirmed_departures` (`passive_faults.rs`) |
| LIB-04 and LIB-05 Fault: a panic inside a running connection leaves no descriptor or operation counted | `installed_library_panic_in_a_connection_leaves_nothing_counted` (`passive_faults.rs`) |
| LIB-05 Fault: no `SIGPIPE` in an embedder that left it at its default | `installed_library_never_raises_sigpipe_in_its_embedder` (`passive_faults.rs`) |
| COM-02 and COM-03 Library: native admission, every queue limit at and one past its bound, no ID consumed by a rejection | `installed_library_admits_only_valid_requests_and_rejections_cost_nothing`, `installed_library_bounds_requests_by_every_queue_limit` (`climon2-e2e/tests/passive_admission.rs`) |
| LIB-04 Library: delivery lifetimes and limits | `installed_library_holds_each_delivery_until_released` (`passive_admission.rs`) |
| LIB-04 Fault: drain batching, a full event queue holding daemon records, per-record charges | `installed_library_batches_records_within_drain_payload_bytes`, `installed_library_holds_daemon_records_while_the_event_queue_is_full`, `installed_library_charges_each_record_its_header_and_the_bytes_it_owns` (`passive_admission.rs`), `installed_library_charges_presentation_records_only_the_display_bytes_they_own` (`control_faults.rs`) |
| COM-06 and REC-01 Library: exit zero, exit 143 and a signal from real daemons each arrive as one Command final of their own kind; the daemon's close then stops cleanly, retry none | `installed_library_relays_each_real_command_final_apart_from_signals` (`climon2-e2e/tests/termination.rs`) |
| COM-03 Kill, DEL-02, COM-06 and REC-01 Library: a Kill alongside a passive Open and a Kill-only instance succeed only after the real hang-up grace and cleanup, on their own connections; a second pending Kill `invalid_state`; one Command final of the same result | `installed_library_kills_a_real_session_only_after_termination_and_cleanup` (`termination.rs`) |
| COM-03 Kill, DEL-02 and REC-01 Library: a Kill that cannot reach a stopped daemon is rejected `timeout` and leaves the opened role, whose Leave is later confirmed | `installed_library_keeps_a_real_role_when_its_kill_cannot_reach_the_daemon` (`termination.rs`) |
| REC-01 Library: transport loss and a contained panic call for passive retry, another session's startup for permanent | `installed_library_classifies_real_failures_for_retry` (`termination.rs`) |
| REC-01 Library: startup failure calls for passive retry; destroy is busy until handles are released; a fresh instance and generation open only after destroy and join | `installed_library_recovers_only_through_a_fresh_instance_after_join` (`termination.rs`) |
| COM-06 and REC-01 Fault: exit 0 and 143 as Command final records, a contradictory repeat, closure without final evidence or meeting a Leave (beside #94's Command final cases) | `installed_library_relays_each_command_final_once_and_closure_invents_nothing` (`climon2-e2e/tests/termination_faults.rs`) |
| COM-03 Kill, DEL-02, COM-06 and REC-01 Fault: confirmed, known-final, partial, refused, lost, late, malformed and Fatal Kill answers; refusals before sending; one pending Kill; cancellation and shutdown; never replayed | `installed_library_reports_each_kill_outcome_truthfully_and_never_replays_it` (`termination_faults.rs`) |
| COM-06 and COM-03 Fault: a partial termination on any request but a Kill is `invalid_record`; a Leave so answered stays indeterminate | `installed_library_refuses_a_partial_termination_on_any_other_request` (`termination_faults.rs`) |
| COM-03 Kill, DEL-02, COM-06 and REC-01 Fault: each Kill failure on an opened role only answers its request; the role reads on to its own end | `installed_library_keeps_an_opened_role_through_each_kill_failure` (`termination_faults.rs`) |
| DOC-01 exported interface: only the bootstrap is exported, from the real release library and from a defective stand-in | `release_check_builds_unhooked_binaries_in_a_private_target` (`xtask/tests/release.rs`), `release_checks_reject_extra_exports` (`xtask/src/library.rs`) |
| DOC-01 layout case on x86-64 GNU/Linux and both macOS architectures | `rust_native_declarations_match_the_published_header` (`climon2-core/tests/native_layout.rs`) |
| #75 risk proof: `cargo test` builds the hooked library as a sibling artifact without nested builds (ELF on Linux, Mach-O on macOS); the plain library is the release build | `workspace_client_library_is_a_hooked_sibling_of_the_test_executables` (`climon2-fixtures/tests/hooks.rs`); plain: `release_check_builds_unhooked_binaries_in_a_private_target` (`xtask/tests/release.rs`) |
| #75 packaging, Linux ([library embedding](../../docs/protocol/library-embedding.md#artifact-and-dependency-lookup)'s release-build rules): release checks reject a soname, RPATH, RUNPATH or a dependency beyond glibc | `release_checks_reject_a_soname_rpath_or_runpath`, `release_checks_reject_a_dependency_beyond_glibc` (`xtask/src/library.rs`) |
| #75 packaging: release checks accept the exact library and reject a wrong architecture, another release's version or hook markers | `release_checks_accept_the_exact_library`, `release_checks_reject_a_wrong_architecture`, `release_checks_reject_a_wrong_version`, `release_checks_reject_test_hook_markers` (`xtask/src/library.rs`) |
| #84 packaging: release checks load the library through the shared loader, so a description it refuses (an incomplete function table) fails them | `release_checks_reject_a_description_the_loader_refuses` (`xtask/src/library.rs`) |
| LIB-01 Library, mixed releases (#97; private-dependency resolution not exercised, releases ship none): this library and the second-release fixture's, loaded side by side by exact version folder whatever `current`, the working directory and search paths name, each instance registering with a session of its own release's daemon; one shared load per release | `mixed_releases_load_by_exact_version_and_run_side_by_side_whatever_current_cwd_and_search_paths` (`climon2-e2e/tests/mixed.rs`) |
| LIB-03 Library (identity separation) and Fault (fault containment), mixed releases (#97): each release's identity checks stay its own beside the other's running instances, and a fault injected into the process fails only the hooked release's instance | `mixed_release_instances_keep_their_own_release_identity_side_by_side`, `a_fault_in_one_release_never_reaches_the_other_releases_instances` (`mixed.rs`) |
| #97 build strategy: the second-release fixture, this library's source built by the same Cargo run as release 0.0.97, passes the single-architecture release checks as 0.0.97's library (only the bootstrap exported; on Linux glibc only, no soname, RPATH or RUNPATH; no hook marker) and describes the wrong release as this version's | `release_checks_accept_the_second_release_fixture_as_its_own_release` (`xtask/src/library.rs`) |
| Installed and packaged layouts carry the library | `fresh_install_publishes_complete_version_launcher_and_current` (`climon2-install`), `install_builds_production_artifacts_and_preserves_existing_versions` (`xtask/tests/install.rs`) |

LIB-06 Library is not covered yet: without the fault hook this release starts
no worker that could stall. Open
([#93](https://github.com/jackgeek/climon2/issues/93)) brings the first.

## Behavioral evidence

`tests/bridge.rs` drives the real binary on real PTYs (#20, #101). Each test
installs the client executable and the workspace's client library in a
private version folder and answers as a revision-1 peer whose frames it writes
by hand from the wire reference. They are the COM-01 Monitor parity case
(ordinary attach, input, resize, escape, detach and Kill parity), beside the
packaged scenarios in `climon2-e2e`.

| Scenario | Named test |
|---|---|
| COM-01 parity: byte transparency (binary, Ctrl-C/Ctrl-Z, paste), raw mode only after the Open succeeds, the Open requiring only `daemon-wire-v1`, status from Command final | `raw_bridge_preserves_bytes_and_restores_termios_after_final_output`, `one_way_descriptors_are_bridged_and_restored`, `fragmented_frames_and_coalesced_resize_preserve_the_stream` |
| COM-01 parity: keys typed during a restoration, or a replacement restoration, are dropped; Ready waits for typed-ahead keys and the written restoration | `input_typed_before_restore_end_is_drained_and_never_replayed`, `a_replacement_restoration_drops_typed_keys_until_ready`, `ready_waits_until_the_restoration_is_written`, `ready_waits_until_typed_ahead_keys_are_drained` |
| COM-01 parity: the placeholder, its redraw on resize, the take-control key (Acquire), and an observer's size reaching the session once it holds control | `a_revoked_client_shows_the_placeholder_until_the_session_ends`, `a_resized_observer_tells_the_session_its_size_once_it_holds_control` |
| COM-01 parity: paste discard across role changes | `a_paste_spanning_role_changes_is_never_replayed_or_read_as_keys` |
| COM-01 parity: detach, confirmed or unconfirmed within the five-second deadline, from either role | `a_confirmed_leave_detaches_with_the_notice`, `unconfirmed_detach_exits_after_five_seconds_and_restores_terminal`, `pending_detach_while_stdout_is_blocked_ends_within_the_bound`, `an_observer_detaching_ignores_resizes_and_ends_the_placeholder` |
| COM-01 parity: signals (HUP/TERM/INT, before and after the Open's answer, while observing) and terminal EOF/EIO | `raw_client_signals_restore_termios_close_connection_and_return_signal_status`, `termination_before_attached_does_not_enter_raw_mode_or_report_error`, `a_signal_while_observing_ends_the_placeholder_and_keeps_the_signal_status`, `terminal_eof_or_eio_is_silent_status_129_and_closes_connection`, `terminal_eio_is_silent_status_129_and_restores_termios`, `input_consumed_by_another_reader_after_poll_is_not_a_hang_up` |
| Exit statuses unchanged: the exit code, 128 plus a signal's number (125 on overflow), a result known at the Open | `a_signalled_command_ends_the_client_with_128_plus_the_signal`, `a_result_known_at_the_open_ends_the_client_with_that_status`, `separate_stdin_and_stdout_descriptions_both_get_their_flags_back` |
| COM-01 parity: output and input backpressure, blocked-terminal deadlines and the bounded read-ahead | `bounded_output_backpressure_drains_every_byte_before_exit`, `input_backpressure_is_bounded_and_lossless`, `hung_up_session_during_blocked_output_sleeps_then_delivers_everything`, `hang_up_behind_blocked_output_and_input_keeps_output_and_leaves_keys`, `a_hang_up_behind_blocked_output_and_held_input_ends_within_the_closing_deadline`, `lost_session_while_stdout_is_blocked_ends_within_the_closing_deadline`, `exit_read_while_stdout_is_blocked_ends_within_the_closing_deadline`, `output_read_ahead_of_a_blocked_terminal_is_bounded`, `an_idle_client_sleeps` |
| COM-01 parity: failures restore before one diagnostic; only a lost connection (between or inside frames) resets conservatively | `protocol_error_lost_connection_and_panic_restore_before_one_line_diagnosis`, `an_error_while_observing_ends_the_placeholder_before_the_diagnostic`, `a_lost_connection_while_observing_resets_conservatively_and_ends_the_placeholder`, `a_close_inside_a_frame_is_a_lost_connection`, `invalid_length_prefix_closes_immediately_without_a_type_byte` |
| COM-01 parity: hello and Open refusals before raw mode (session, protocol, release, no revision 1, malformed or late hello, a refused Open, an early record, a disconnect, an unreachable socket) | `hello_rejects_mismatched_session_protocol_version_and_unexpected_frames`, `hello_deadline_is_absolute_across_partial_reply`, `attach_refusal_or_unexpected_reply_fails_without_raw_mode`, `disconnect_before_attached_names_the_session`, `nonexistent_socket_and_bad_metadata_fail_without_touching_termios` |
| LIB-02 executable case, attach: a missing library fails before any connection | `a_missing_library_fails_attach_before_any_connection` |
| Derived: the executable passes exactly the published `cm1_limits` table | `attach_passes_exactly_the_published_limits_table` |
| Derived viewing and acknowledgement (#102), STA-08 ordinary-attach case: the Open asks for `status-v1` when advertised; nothing before Ready; after an explicit attach's and a takeover's Ready, viewing true then one Acknowledge; a replacement restoration and a hand-back report viewing but never acknowledge; a revocation sends nothing; detach reports viewing false, then Leave | `ordinary_attach_reports_viewing_and_acknowledges_once_ready_after_attach_and_takeover` |
| Derived (#102): against a daemon without `status-v1` the Open names no optional family and nothing comes between Ready and the next Input or Leave | `raw_bridge_preserves_bytes_and_restores_termios_after_final_output`, `a_confirmed_leave_detaches_with_the_notice` |
| Derived (#102) against a real daemon, through the packaged CLI: an explicit attach and a takeover acknowledge pending attention once ready; a viewing client never does on its own | `ordinary_attach_and_takeover_acknowledge_pending_attention_once_ready` (`climon2-e2e/tests/status.rs`) |
| #25 logs: environment-only filtering, one private log per client | `log_filter_reaches_client_through_environment_without_cli_flags`, `concurrent_clients_on_one_session_each_write_their_own_log` |
| Attach gives the library its own `library-<second>-<pid>.log` beside the client's session log, under the client's `CLIMON2_LOG` filter (its creation record by default, none under `warn`) | `attach_gives_the_library_its_own_log_beside_the_clients_session_log` |

`tests/termination.rs` covers kill mode through the library (#100). Each test
installs the client executable and the workspace's hooked client library in a
private version folder and answers as a revision-1 peer whose frames it
writes by hand from the wire reference:

| Scenario | Named test |
|---|---|
| LIB-01 executable case and COM-01 Fault (Kill parity): a confirmed termination (exit 143, complete cleanup) is silent status 0, through the library beside the executable; one Kill, wire ID 1, revision 1, requiring nothing | `confirmed_termination_is_silent_success_whatever_the_command_status` |
| LIB-01 executable case: stand-ins describing this release in the folder `current` names, the working directory, `LD_LIBRARY_PATH` and `DYLD_FALLBACK_LIBRARY_PATH` are never loaded | `the_library_beside_the_executable_is_used_whatever_current_cwd_and_search_paths` |
| LIB-02 executable case: a missing library, another release's, one without a bootstrap and one with an incomplete function table, and an executable outside a version folder, fail with 125 and the loader's diagnostic before anything connects | `a_missing_wrong_release_or_faulty_library_fails_before_any_connection` |
| COM-01 Fault (Kill parity) and COM-03 Fault, executable case: a refusal (also one coded `io_error`) and a partial termination give the daemon's message, a connection closed during the hello or after the Kill `lost connection to session <id>`, a Kill outliving `kill_ms` the CLI's unconfirmed message, each with 125; the Kill is never sent again | `a_refused_partial_lost_or_unconfirmed_kill_is_a_failed_kill` |
| COM-01 and COM-03 Fault, executable case: a hello the daemon accepts but never answers fails at the two-second hello deadline, not `kill_ms`, with `session <id> did not respond`, and the Kill is never sent | `a_silent_hello_fails_at_the_hello_deadline_without_sending_the_kill` |
| The library's diagnostics land in its own `library-<second>-<pid>.log` beside the client's session log, under the client's `CLIMON2_LOG` filter (its creation record by default, none under `warn`) | `the_library_logs_into_its_own_file_beside_the_clients_session_log` |
| COM-01 Fault, executable case: a daemon of this release without daemon wire revision 1 is refused before any request | `kill_needs_daemon_wire_revision_1_and_never_sends_a_request_without_it` |
| COM-01 Fault (Kill parity), executable case: SIGTERM while waiting is status 143 at once, silently | `a_signal_while_waiting_for_confirmation_is_the_signal_status` |

Against real daemons, `c2 kill` through the CLI and the packaged
launcher is unchanged: `kill_terminates_a_detached_session_silently_after_cleanup`,
`kill_while_attached_gives_the_client_the_signal_status` and
`an_unconfirmed_kill_fails_at_the_deadline_and_reaps_only_its_helper`
(`climon2-cli/tests/exec.rs`, whose private installation now carries the
library), and `installed_kill_ends_detached_and_attached_sessions` with the
other packaged kills (`climon2-e2e`), including a 0.0.97 session killed from
the second-release fixture's folder (`climon2-e2e/tests/mixed.rs`).
`tests/scaffold.rs` covers the frozen invocation `attach --metadata <absolute
path>`: `invocations_outside_the_frozen_contract_are_one_line_usage_failures_without_stdout`
(no arguments, a missing or relative path, another flag or operation, or an
extra argument).

The client executable's only fault injection, `CLIMON2_CLIENT_FAULT=panic`
(with `test-hooks` only), panics after entering raw mode; the client library's
`CLIMON2_LIBRARY_FAULT` faults are described with its instance lifecycle. The client has no attach-hold hook (the
plan's `hold-attach` was not implemented); the CLI's `CLIMON2_CLI_HOLD` hook
holds before the client starts instead.
The panic hook logs a static event, never payloads, argv or environment values.
Tests create isolated files under the project and never run nested Cargo builds.
Restoration assertions compare all configuration flags, control characters and
speeds; Darwin's kernel-managed transient `PENDIN` flag is not configuration.
They also compare the terminal's file-status flags (`F_GETFL`, e.g. `O_NONBLOCK`)
for stdin's and stdout's open-file descriptions; Darwin's kernel-private
`FWASWRITTEN` bit (set by any write, not settable) is not configuration.
