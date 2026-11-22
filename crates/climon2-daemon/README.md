# climon2-daemon

Real session daemon library with a thin executable on macOS, Linux and
Windows; no client implementation or descriptor passing. The daemon owns the
VT engine, bounded scrollback, terminal restoration and live presentation.

On Windows (#317) the PTY is `src/pty/windows.rs`, a registered native file
over `climon2-platform`'s `pty` seam: the command runs in a pseudoconsole of
the bundled ConPTY, `conpty.dll` loaded from the daemon's own folder beside
`OpenConsole.exe` (either missing fails startup with 125), inside a session
job. `TERM`, `COLORTERM` and `CLIMON2_SESSION_ID` are set in the daemon's
environment first. A command that cannot start is 126; there is no exec
helper or startup socket. The command's end comes from its leader's handle,
its status is kept in full and logged as `command exited status=0x...`, and
`--readiness-fd` is the inherited pipe's handle value. `tests/windows.rs`
checks the binary's own answers; `climon2-cli`'s `tests/exec_windows.rs`
runs it as `exec` does.

## CLI-to-daemon API

```text
climon2-daemon --session-id GUID --home CANONICAL_ABSOLUTE_HOME \
  --socket ABSOLUTE_SOCKET --version SEMVER --cli-pid PID \
  --lifetime ordinary|headless --readiness-fd FD --rows ROWS --cols COLS \
  --scrollback-max-rows ROWS --assembly-records COUNT --assembly-bytes BYTES \
  --assembly-ms MILLISECONDS --cwd ABSOLUTE_CWD \
  [--log FILTER] -- COMMAND [ARG...]
```

Options are private, order-independent, unique and required except `--log`.
`--scrollback-max-rows` sets the VT engine's retained normal-screen history.
`--assembly-records`, `--assembly-bytes` and `--assembly-ms` are the
notification assembly limits (`payload.assembly_records`,
`payload.assembly_bytes` and `deadlines.assembly_ms` of `monitor.limits`)
that `exec` validated, each in its setting's range; they stay fixed for the
session's lifetime (#98).
`--lifetime` is the session's startup policy, its
[lifetime policy](../../GLOSSARY.md#sessions): `ordinary` for `c2 exec`,
`headless` for `c2 exec --headless` (see the leave policy below). Any other
value is an invalid invocation. Core's `lifetime::Lifetime` owns both spellings.
All tokens after `--` are untouched OS-string command arguments. The CLI
resolves home/socket/version, computes the session environment, and supplies
initial terminal dimensions. The daemon does not reconstruct runtime values
from display metadata or modify the inherited command environment.

Use `platform::daemon::spawn(command, |command, fd| { ... })`. Append
`--readiness-fd FD`, then `-- COMMAND ARG...`, **inside that closure**: options
after `--` belong to the command. The inherited descriptor is adopted with
`ReadinessWriter::from_inherited` before PTY spawn and made close-on-exec.
`PendingDaemon::wait_ready(&[metadata_path, socket_path])` receives:

- `Readiness::Ready` only after successful command exec, usable PTY/socket,
  and atomic 0600 `<home>/sessions/<guid>.pty` publication under 0700 sessions.
- `Readiness::Failed { code, message }`: 127 not found, 126 cannot execute or
  inaccessible cwd, 125 daemon failure. Messages are useful, static,
  single-line diagnostics without command/argument/environment content.

`--log` is the CLI's already-read `CLIMON2_LOG` value. Unset means the shared
debug/release default; the daemon does not reread that environment variable.
The command still inherits its original `CLIMON2_LOG` unchanged. Shared
`LogFile::session`, dispatcher and capped stderr capture provide daemon logging.
The panic hook intentionally omits panic payloads, which may contain secrets.

### Exact environment and launch confirmation

portable-pty 0.9.0 unconditionally supplies `SHELL`, even after `env_clear`.
The daemon spawns its own private `--exec-command` entry point on the PTY,
passing the original OS strings, explicit cwd, and an exact copied environment.
That entry point removes `SHELL` **only when originally absent** and directly
execs the command. Empty and non-UTF8 inherited values remain unchanged.

A short-lived authenticated `<private runtime>/<guid>.exec` socket confirms the exec:
the helper sends an attempt marker, and successful exec closes its CLOEXEC
connection; failed exec sends 126/127. Its path comes from the shared bounded
runtime layout, not from home, executable, checkout or the supplied session
socket path. A valid 103-byte session socket must not grow when naming the
helper socket. The helper adds no environment variables.
Both sockets are bound under the shared private-directory/umask seam before
starting stderr's capture thread. The startup socket is removed on every
controlled startup path. The outer CLI readiness deadline remains five seconds.

## Ownership and lifecycle

One event loop owns the master, and reads output and writes input through one
owned duplicate of it. It never takes portable-pty's writer, whose drop types a
newline and the end-of-file key into the command's terminal.
`Session::read_pty` is the sole PTY reader, through `Pty::read_output`;
there are no other cloned readers or master handles, so closing the master and
that duplicate hangs the terminal up.
**ADR 0003 / #27 review rule:** reviewers must preserve this single-reader
call graph. A textual ownership assertion would not prove it, so no vacuous
architecture test is claimed.

The daemon owns the session's virtual terminal (`climon2-vt`, ADR 0007). Every
loop iteration reads at most 32 KiB from the PTY and feeds it to the VT, whether
or not any client is attached; a slow or absent client never stops the command's
output. Between iterations the loop waits up to 2 ms for PTY readability (the
platform waiting seam, no read). The VT's terminal replies go to the PTY; its
filtered live output goes to the controller only.

Authenticated connections use core's typed protocol messages and connection
states. Incremental nonblocking decoding enforces an absolute two-second hello
deadline and bounded frames, and reads no further than the frame it is
decoding. Up to 64 concurrent connections are admitted;
a further connection is closed unanswered. Each connection's outgoing queue is
capped at 16 MiB, each queued frame charging its allocation and its place in
the queue until it is written whole. Restoration prefixes are produced incrementally, whenever less
than 1 MiB is queued, while live output waits behind them; a controller whose
pending live output plus queued bytes would exceed 16 MiB, or any connection
that accepts none of its queued bytes for five seconds, is treated as lost. A
controller that does not send `Ready` within 30 s plus one second per 4 MiB of
restoration prefix is also lost. Probes remain serviceable throughout.

### The frozen hello, then daemon wire revision 1

A connection's first frame must be a valid frozen hello of protocol 1
(`core::protocol::HelloRequest`) within two seconds of accept: a frame
length outside 1 to 1,048,576 bytes, any other type (refused as soon as its
type byte arrives), malformed JSON, another protocol or the deadline closes
the connection without a byte. Hello advertises the frozen tokens and
`state`, then `daemon-wire-v1`, `monitor-v1`, `title-v1`, `status-v1` and
`focus-v1`, and `daemon_wire_v1` with 1,048,576-byte request and server
frame limits (`core::hello::advertise_revision1`). Every frame after hello is
[daemon wire revision 1](../../docs/protocol/daemon-client/core-v1.md), the
only protocol after hello since the version-private one was retired (#104):
a type revision 1 does not define, a second hello or a retired private type
included, is `unknown_message_type`. Every revision-1 frame
is read and written by core's `wire` codec: a frame its prefix declares
oversized is refused before its body arrives, request IDs must strictly
increase (a rejected request still consumes its ID), and a stream ending inside
a frame is `truncated_frame`. A connection-fatal failure queues a best-effort
Fatal error, replacing unsent output, and closes the connection; a stream
ending between frames just closes. Frames read before the end are still
served, so a peer may send Leave and half-close.

A monitor Open registers a **passive monitor connection**. It is not an
attachment: the leave policy and the hello `state` (so `c2 list`'s
STATE and CLIENTS) never count it, and registering never acquires control,
though the registration may inherit it (see below).
Open selects the requested families this daemon supports (required ones must
all be; unknown or unsupported optional ones are dropped) under a fresh
connection GUID. Its success Response is followed by one Initial metadata
record captured in the same loop step: the title by value when `title-v1` is
selected (unavailable otherwise), the stored effective status when
`status-v1` is selected (unavailable otherwise), the retained progress when
`progress-v1` is selected (unavailable otherwise), and the command's result
once the daemon knows it (null before). `notification-v1` is selected only
with `status-v1`: optional without it, it is dropped; required without it,
the Open is rejected with `unsupported_capability`.
Every later change of the application title that the VT reports
(`Fed::titles`, one per change, by value) goes as a Title changed to each
registration that selected `title-v1`, whatever the control state; titles are
inert text, never logged. A monitor Open is accepted during committed
termination too, since that rejects new interactive work only.

Leave removes the registration in the same step, before any later
departure can promote it, answers `left` with `attachment_detached: false`,
and closes the stream; nothing sent after it is served. Leave before any Open, and a later Open or Kill on a registered
connection, are `invalid_state` and the stream goes on. A rejected first role
(an unsupported revision, or an unknown or unsupported required family) has
no effects, carries the command's result as `known_final` once the daemon
knows it, and ends the stream. On a connection without an attachment, Input,
Resize and Ready are rejected without effects and the stream goes on, with
any registration kept: `invalid_state` before any Open, `not_attached` on a
registration or a standalone Kill's connection. Focus is
`unsupported_capability` unless the connection's Open selected `focus-v1`,
and Set viewing and Acknowledge unless it selected `status-v1`, then
`not_attached` on a registration. Acquire is `invalid_state` before any
Open or on a standalone Kill's connection, which leave nothing to convert.
Registrations share the
connection cap, the 16 MiB queue and the five-second no-progress loss.

Every revision-1 connection shares these bounds (#90): it counts toward
the 64 connections, and output that would take its queue past 16 MiB (each
queued frame charges its allocation and its 32-byte place in the queue
until it is written whole, so even empty frames are bounded), or a
queue that accepts nothing for five seconds, loses it at once: its stream
just closes, so a slow peer reads a gap-free prefix of its records and then
the end, while every other connection, the PTY and control go on. The
largest frame either way is the advertised 1,048,576 bytes
(`connection::frame_limit`), and titles are at most 4,096 bytes, so a
metadata record stays within 16,384 bytes. Request IDs, control periods and
restoration IDs never wrap: the last request ID,
18,446,744,073,709,551,615, can be served but no request follows it, and
once the last period or restoration ID is issued (`control::Issued`, which
hands out a `Fresh` pair only while both can advance), work that needs a
new one is refused before its effects with `resource_limit`: an
interactive Open, an Acquire by anyone but the controller and a Resize
whose restoration would need a new ID. No registration is then promoted,
and a departing controller hands back to nobody: each remaining attachment
is explicitly failed (its connection closes), since the hello `state`
cannot describe attachments without a controller.

An interactive Open attaches an ordinary **interactive attachment**, under
the attachment, control and leave policy every attachment shares. Its success Response and Initial
metadata come first, then, in the same loop step, Control granted (`attach`)
and Restoration begin; the restoration's Display chunks and Restoration
complete follow as the connection drains, then live Display under the same
period and restoration ID. Grants and replacement restorations are reported
at once, before their snapshot, which waits for the pending size to apply
and for any synchronized update to end. A host dimension above 65,535 is
`invalid_geometry`, and an interactive Open during committed termination is
`session_terminating`; both end the stream. Input, Resize, Ready and Focus
are checked in the
[documented order](../../docs/protocol/daemon-client/core-v1.md#implemented-authority-checks):
family, role, attachment, termination, control, period, restoration, then
readiness, and finally their own arguments (empty Input is
`invalid_argument`, a geometry beyond 65,535 `invalid_geometry`). Display
carries restoration, live output and the release of the presentation, at
most 65,535 bytes per frame and never empty; Control revoked follows the
release, with `takeover` when another attachment takes control and
`session_ending` when the session ends while it controls. A ready
controller's applied resize reports Geometry when the session geometry
changed. Acquire by a non-controlling attachment takes control at once: its
Control granted (`acquire`) and Restoration begin precede the changed
success, and its restoration follows; the controller's own Acquire is a
no-op naming its current authority. Leave is an accepted detach: the
release, then hand-back (`hand_back`) to the most recently controlling
remaining attachment, then `left` with `attachment_detached: true`; during
committed termination it still removes the attachment after releasing a
presentation it has. Focus (`focus-v1`) remembers the controller's desired
focus for its control period: a change while the application has mode 1004
on queues `ESC [ I` or `ESC [ O` with the controller's input, in request
order (3 bytes accepted), nothing otherwise, and newly enabled reporting
(`Fed::focus_enabled`) queues the known state once with the daemon's
terminal replies.

A registration's Acquire converts it into an attachment that takes control
at once, of a starting or detached session too: any previous controller gets
its release and Control revoked (`takeover`), then the registration gets
Control granted (`acquire`) and Restoration begin before the changed success;
the hello `state` then counts it, and it stays an attachment when another
client takes over. **Succession:** when a controller departs, by Leave or
any loss, control is handed back (`hand_back`) to the most
recently controlling remaining attachment; when none remains, the newest
registration is promoted (`successor`): it becomes an attachment with a
grant and a restoration. Only then does the leave policy apply, so a
registered monitor keeps a session alive through its last attachment's
departure. Committed termination refuses Acquire (`session_terminating`)
and promotes nobody; a registration promoted before the termination
committed ends with it like any attachment. A Leave read after a promotion
is that attachment's accepted detach (`attachment_detached: true`).

A converted registration has **canonical presentation** (`Effect::Convert`,
`Delivery::canonical`): each of its grants keeps the session geometry, so
acquiring or inheriting resizes nothing, and each of its restorations is
`Vt::canonical`'s, serialized at the session geometry for a fresh terminal
of that size whatever host size anyone reported, with live output Direct.
A host change that keeps the geometry keeps its restoration; once ready, a
Resize that changes the geometry is followed, when applied, by Geometry and
at once a replacement Restoration begin at the new geometry within the same
period, before any Display at it, so its model is never reflowed in place.

Activity and attention (`src/status.rs`, #91 and #92) follow the
[session status mechanics](../../docs/session-status.md) in all three
detection modes: heuristic, explicit and fallback.
A pure detector holds both state machines; every event takes the turn's
clock reading, and its random source is injected. The loop reads the clock
once at the start of each turn, so accepted input (Input from the ready
controller) comes before that turn's timer
and sample work. After reading the PTY the loop polls the detector: a due
sample copies the active viewport's characters (`Vt::viewport_characters`), its size
and the application title, never per output byte. The first baseline is
taken before any command output. Each sampling delay is a fresh draw from
500 to 1,500 ms, clamped to the hardcoded 10,000 ms agent deadline; changed
characters at the same size or a changed title start agent work and restart
that deadline, a resize replaces the baseline (restarting an armed agent
deadline) without starting work, and content is compared before the
deadline expires, which ends agent work and requests attention in the same
event. Each accepted input re-arms a 2,000 ms user window, clears pending or
acknowledged attention, and suspends sampling; its end is idle with a fresh
baseline. Focus, the daemon's terminal replies and paste cleanup are not
input. Every effective change goes as Status changed to the `status-v1`
connections, before an Input's or Acknowledge's Response; only effective
changes replace the stored UTC change time. Episodes count from 1, coalesce
while pending, survive acknowledgement and are never wrapped: a request
that needs one after the last counter starts no episode, is logged, and
every later Acknowledge is `resource_limit`. Input holding only focus
reports (`ESC [ I`, `ESC [ O`), which an ordinary client forwards from its
terminal once the application turns focus reporting on, is not user input;
query replies never arrive, since the daemon alone answers queries. The published result
gives the final status, carried by Command final, and stops observation.
Set viewing records the controller's latest viewing for its control period;
Acknowledge needs it true.

The loop also feeds the command's output through `Vt::feed_observed`, and
`status::interpret` (`src/status/signals.rs`) parses each observed sequence
once, in stream order: an OSC 7501 capability query (`?`) is answered with
`status::QUERY_REPLY` among the daemon's terminal replies and drives
nothing; a valid root report (Program Status Protocol 0.4 validation; any
report with an `id` drives nothing, so no child record is kept), an OSC 133
`A`, `B`, `C` or `D`, and a full reset (`ESC c`) are `Signal`s for
`Detector::signal`, which samples the terminal as of the signal when it
needs a fresh baseline; their changes are reported after the read's live
output. Root `working` is agent work without a deadline, which a user
window's end restores while the explicit scope's root record says so; root
`idle`, `done`, `blocked` and `error` request attention; `error` and `clear`
start the 10,000 ms fallback quiet deadline, which changed characters, a
changed title or a resize restart (never agent work), which a fresh root
report or shell boundary ends, and whose expiry resumes heuristics without
attention. `A`, `C` and `D` retire the scope to idle heuristics with a fresh
baseline; `A` excludes prompt rendering from heuristics until `B` or `C`;
`B` only rebases. A full reset rebases heuristics and acts as `clear` in
explicit or fallback detection; a soft reset is not a signal. The VT strips
every OSC from host output.

Notifications and progress (#98) come from the same observed sequences
(`status::signals`): OSC 9 text (a numbered first parameter is a control
operation, OSC 9;4 progress included), OSC 777 `notify` and OSC 99 chunks
(`status::notifications`). `status::assembly` assembles OSC 99 chunks under
the exec-provided limits, discarding an assembly that passes a bound and
expiring one at its fixed lifetime with a logged diagnostic and nothing
completed, and makes text inert UTF-8 within the release-constant bounds
(titles 4,096 bytes, bodies 8,192, records 16,384), ignoring oversized
notifications. A completed notification requests attention through
`Detector::notify`, so its Status changed, if any, precedes its Notification,
which carries the pending episode. `status::progress` normalizes OSC 9;4
against the cached number and reports only changes. Notification and
Progress changed reach every connection that selected their family, in
stream order with the status changes and whatever the control state.

The daemon keeps the command's result as its wait status gives it
(`core::wire::CommandResult`): a normal exit's raw status, or a signal's
number, never folded together. It publishes the result
once, when it is final, so that no more of the command's output follows
Command final: for a command that exited by itself, when the PTY drain after
the reap ends (end of output, `PTY_DRAIN_IDLE` 100 ms without output, or
`PTY_DRAIN_CAP` 1 s after the reap), at the end of an output pass that
delivered the drained output, once nothing it produced is held back from a
connection, and before the exit report waits for `Ready` or the initial
attachment. Held back means a restoration in progress (its frames, then the
live backlog behind it) or a pending synchronized update (which holds a
derived update and every grant's snapshot); each ends within its own bound,
by completing or by the loss of a reader without progress for 5 s, and by
the 150 ms latch (#203). A grant after Command final still replays the
screen. A termination (`Kill`, final unexpected departure, authoritative
failure such as a failed PTY read, or initial deadline) that begins after the
reap does not cut that drain short: `begin_shutdown` first observes the
command, and if it has exited the hang-up waits until the drain ends and
Command final is published (owner decision on #88, 2026-11-14). A failed
read during the drain counts as idle, so the drain goes on and ends as usual.
After a hang-up of a running command, which stops PTY reads, the result is
published when the end of the command is learned at the end of the grace
(`HANGUP_GRACE`), with any result already reaped. Every passive monitor connection and interactive
attachment then receives Command final, with the final status when it
selected `status-v1` and a null one otherwise; the controller receives the command's last output
before it, then its release and Control revoked (`session_ending`). Later Opens get the result in their Initial metadata,
then their one Command final as the session ends, however it ends (#259),
and later first-role rejections carry it as
`known_final`. A hang-up the daemon cannot confirm publishes the result
only if the command was already reaped, and nothing otherwise.

A valid standalone Kill takes the Kill role (never an attachment) and
commits termination, and later kills join that one hang-up grace without
restarting it. It is
answered when the session ends: `termination` with the result and
`cleanup: complete` once the result was confirmed and cleanup completed,
even after an authoritative session failure; otherwise `failed` with
`cleanup_failed`, `cleanup: incomplete` and the result the daemon knew,
confirmed or not (null if it never learned it). The stream then ends. A
later Open or Kill on its connection is `invalid_state`, as are Leave and
Acquire; Input, Resize and Ready are `not_attached` and Focus
`unsupported_capability`. Standalone Kills wait in their own map, never
among the opened connections that receive core metadata.

Every PTY byte goes through one ordered write queue (`PtyWrites`): terminal
replies, the Focus report on newly enabled reporting and paste cleanup bytes
ahead of queued input, which holds a controller's Input and Focus reports in
request order, and a partly written unit always completes before the next
starts, so sequences never interleave across partial writes. Input is admitted only from the
controller of the current control period after its `Ready`; when admission
ends, its unwritten input is discarded and, if an application paste is open
while mode 2004 is set, closing-marker cleanup bytes are queued. A PTY write
failure is an authoritative session failure.

Control is a pure state machine (`control.rs`): each attach, Acquire,
hand-back and promotion is granted at once under a new authority, a control
period with its first restoration ID (attaches in one iteration are each
granted and the last wins), the previous controller gets its release then
Control revoked, and a departing controller hands back to the most recently
controlling attachment still present, otherwise promotes the newest passive
registration, which it keeps in acceptance order. A host change before readiness replaces the restoration
within its period under a new restoration ID, and Ready needs the complete
current restoration. Only the controller's size drives the
PTY and VT; the session geometry is its host size clamped to 2..=500 columns
and 1..=200 rows, and a host larger than that gets Derived presentation.
Resizes are coalesced to one per iteration.

Early exits and PTY output remain available for the first attachment of an
ordinary session: the exit of a command that ends first is reported once that
attachment's restoration is confirmed, or after the five-second initial attach
window. However the session then ends, a revision-1 connection that
opened after the exit, and so learned the result only from its Initial
metadata, gets that result as Command final as it ends, so each opened
connection gets exactly one (#259). A headless session has no initial attach window, so its exit is reported
at once unless an attachment is restoring. Final status comes
exclusively from shared exact `waitpid`, never portable-pty waiting. After
reaping, drain ends at EOF/EIO, 100 ms of actual PTY idle, or one second.
Metadata and the socket are then removed while connections stay open, the
controller gets its release bytes, every attachment's control ends with
Control revoked (`session_ending`), and final flushing is bounded
to one further second. A receiver that
never reads cannot be guaranteed delivery after this bound.

The leave policy applies after succession: while another attachment remains,
or a passive registration is promoted, the session continues; after an
accepted final detach it keeps running with no idle timeout; a final
unexpected departure, `Kill`, daemon TERM, an authoritative failure or no
attachment within five seconds of readiness begins shutdown. The headless
startup policy (`--lifetime headless`) changes only the zero-attachment cases:
the session advertises `detached` whenever it has no attachment, from readiness
on (never `starting`), has no initial attach window, and stays `detached` after
a final unexpected departure. It therefore ends only on command completion,
`Kill`, daemon TERM or an authoritative failure. Shutdown closes every master
handle without writing to it, then runs a real five-second hang-up grace as a loop
deadline: probes see `terminating`, attaches are refused, repeated kills join
without restarting it, and the grace is not shortened if the command exits
meanwhile. A command already reaped first finishes its PTY drain, so the
hang-up and its grace begin when the drain ends. Pending PTY writes and resizes are dropped after hang-up. When the
grace ends, the daemon SIGKILLs every process still in the command's terminal
session (all job-control process groups, per the #19/#27 owner decision) and
reaps the command. Only after metadata and socket removal does the controlling
attachment get its release then Control revoked (`session_ending`), and each standalone
Kill its `termination` answer; a failed cleanup never confirms a kill. If the loop
itself fails, the same hang-up, grace and kill run without serving connections.

Processes that started their own session are left alone. The PTY slave's
device is captured at spawn; candidates come only from that terminal's and the
session's recorded job groups' kernel process lists (see platform
`TerminalSession`). Because macOS detaches the terminal when the command exits,
the loop records job groups at most every 100 ms while the command runs, and
hangup records them once more before closing the master. A job started and
orphaned within that interval before a natural exit can be missed. HUP/INT/PIPE
are ignored by the daemon. Natural exit also removes surviving session members
after drain. Cleanup always attempts metadata before the session socket;
runtime directories and logs remain.

## Behavioral evidence

```sh
cargo test -p climon2-daemon --features test-hooks
cargo clippy -p climon2-daemon --features test-hooks --all-targets -- -D warnings
cargo fmt -p climon2-daemon -- --check
```

`tests/lifecycle.rs` holds the harness and the owned generic terminal
fixtures that every daemon test shares, and its own startup and invocation
tests, which speak no post-hello protocol:

- `malformed_daemon_invocation_is_own_failure_with_no_stdout`,
  `invalid_cwd_missing_command_and_unsafe_sessions_are_startup_failures`:
  safe launch diagnostics and refused permissions.
- `startup_failure_reports_cleans_up_and_exits_with_the_reported_code`,
  `an_existing_session_file_fails_publication_and_is_left_in_place` (a session
  file already present is someone else's: only the daemon's own temporary is
  removed),
  `startup_confirmation_decides_the_reported_launch_failure`,
  `an_unconfirmed_launch_fails_at_the_startup_deadline` (no helper connection,
  or a silent one, ends startup at the four-second deadline; a command already
  started behind the silent one ignores SIGHUP, so only the failed daemon's kill
  ends it),
  `repeated_unknown_zero_pid_and_relative_options_are_invalid_invocations` and
  `unsupervised_controlled_panic_ends_the_daemon_without_readiness` own the
  readiness pipe instead of using `wait_ready` (which kills a failed daemon),
  so they observe the daemon's own exit status and cleanup. The confirmation
  and deadline tests hold the shared PTY allocation lock briefly so their
  connection is the first on the startup socket, or the socket is gone before
  the helper runs.
- `controlled_panic_is_captured_and_never_ready_uses_real_startup_deadline`:
  hook-only startup faults bounded by the real readiness deadline.
- `exec_helper_confirms_launch_failure_codes_and_exits_with_them` drives the
  private `--exec-command` helper directly: 127 for a missing command in a
  usable cwd, otherwise 126.
- `an_unattached_session_ends_after_the_initial_deadline`: a command that
  ends before anything attaches keeps an ordinary session for the five-second
  initial attach window, then it ends.

Every other daemon test speaks daemon wire revision 1, in
`tests/revision1/`. The private-protocol tests, and the
`tests/revision1/coexistence.rs` tests of both protocols sharing one session
control, were deleted with that protocol
([#104](https://github.com/jackgeek/climon2/issues/104)); every behaviour
they proved is proven through revision-1 peers, as the next table shows.

### Daemon lifecycle over revision 1

The lifecycle evidence of
[#103](https://github.com/jackgeek/climon2/issues/103): each behaviour the
private-protocol tests proved before #104 deleted them, with the revision-1
tests that prove it in `tests/revision1/` (`startup.rs`, `ending.rs`,
`terminal.rs`, `headless.rs` and the scenario modules below).

| Behaviour | Revision-1 tests |
|---|---|
| Startup readiness: PID, start time and release in private (0600) metadata, then exact final bytes and exit, Command final last, and cleanup | `readiness_publishes_session_metadata_and_command_final_is_last` |
| Long home and executable paths and a 103-byte session socket | `long_paths_and_a_maximum_length_session_socket_serve_a_controller` |
| Exact argv, cwd and environment, absent/empty/non-UTF-8 SHELL, logging filter and log privacy | `runtime_argv_cwd_and_inherited_environment_are_exact_not_display_metadata` (test hooks) |
| Startup failures and invalid invocations | none needed: they speak no post-hello protocol (above) |
| The initial-attachment deadline | `an_unattached_session_ends_after_the_initial_deadline` (no post-hello protocol), `watching_does_not_stop_the_initial_attach_rule` |
| HUP, INT and PIPE ignored by the daemon | `the_daemon_ignores_hup_int_and_pipe_and_its_controller_goes_on` |
| Concurrent probes, the absolute two-second hello deadline and a second attach | `probes_and_an_unfinished_hello_never_hold_a_revision1_controller` |
| Exact exit and signal delivery | `command_final_reports_each_exit_and_signal_once` |
| An early exit, delivered after restoration, the held result's Command final as the later attachment's Ready ends the session | `an_early_exit_reaches_a_later_attachment_through_its_restoration` |
| An early exit held until the session ends: the held result's Command final, exactly once, to connections opened before and after the exit, whether the initial-attachment deadline, a Kill or a later attachment's Ready ends it ([#259](https://github.com/jackgeek/climon2/issues/259)) | `the_initial_deadline_publishes_a_held_result_to_every_monitor_once`, `a_kill_during_the_hold_publishes_the_held_result_to_every_monitor_once`, `an_attachment_confirming_ready_publishes_a_held_result_to_every_connection_once` |
| The session's end waits for the controller's Ready | `the_session_ends_only_after_its_controller_confirms_ready` |
| The bounded final drain, leaving no descendants | `the_final_drain_is_bounded_and_leaves_no_descendants`; the unit tests `an_ending_session_finishes_at_its_deadline_or_once_drained` and `final_flushing_lasts_one_further_second` in `src/session.rs` check the drain decision and the one-second final-flush bound without wall-clock bounds |
| The hang-up grace after every unexpected departure (end of stream, connection-fatal error, repeated hello, SIGTERM, unread input), log privacy | `every_unexpected_departure_hangs_up_then_kills_after_the_real_grace` |
| Joined Kills during the grace, probes seeing `terminating`, attaches refused | `a_standalone_kill_is_answered_after_termination_and_cleanup_and_later_kills_join_it`, `committed_termination_rejects_new_interactive_work_and_leave_still_works`, `a_known_result_reaches_later_opens_and_rejected_first_roles` |
| Hanging up types nothing into the command's terminal (Linux GNU, ptrace) | `hanging_up_types_nothing_into_the_command_terminal` |
| Kill with an open application paste | `a_kill_with_an_open_application_paste_still_terminates` |
| Kill after an unreported exit | `a_kill_after_the_command_exited_confirms_its_result` |
| PTY write failure | `a_pty_write_failure_releases_then_ends_control_with_the_session`, `a_kill_succeeds_after_a_session_failure_once_termination_is_confirmed` |
| Job-control cleanup on natural exit and on the controller's loss | `natural_exit_kills_jobs_left_in_the_command_session`, `losing_the_controller_kills_every_job_of_the_command_session_but_not_new_sessions` |
| Malformed, unknown, oversized and out-of-state traffic | `traffic_instead_of_a_hello_closes_only_its_connection`, `revision1_traffic_that_breaks_the_rules_is_connection_fatal`, `a_request_at_the_frame_limit_is_served_and_one_byte_more_is_fatal`, `requests_that_need_an_attachment_are_refused_and_the_stream_goes_on`, `malformed_binary_input_is_connection_fatal` |
| The connection cap | `revision1_connections_share_the_cap_and_a_stalled_monitor_is_lost_after_five_seconds` |
| Real PTY sizes: the zero-size fallback and resize | `interactive_open_is_granted_at_the_requested_geometry_in_the_documented_order`, `a_ready_controllers_resize_reports_the_actual_geometry_after_its_acceptance` (zero dimensions cannot be written in revision 1) |
| Resize before Ready | `a_resize_before_ready_replaces_the_restoration_and_leaves_the_alternate_screen_once` |
| A resize burst is one PTY resize | `a_resize_burst_is_one_pty_resize_to_the_last_size` |
| Byte transparency | `binary_input_and_display_pass_unchanged` |
| An unfinished token still restores | `an_unfinished_token_still_restores_and_a_kill_confirms_the_result` |
| Retention with an unfinished restoration, and eviction | `retention_with_an_unconfirmed_restoration_keeps_only_the_newest_rows`, `identical_rows_evict_and_cleared_history_has_no_stale_notice` |
| Derived presentation of an oversized host | `a_ready_controllers_resize_reports_the_actual_geometry_after_its_acceptance`, `monitor_mode_restorations_are_canonical_at_the_effective_geometry_whatever_the_host` |
| A waiting alternate screen restored before input | `a_waiting_alternate_screen_is_restored_before_input_is_admitted` |
| Detach (Leave) and reattachment | `leave_releases_then_reports_and_a_reattachment_resumes` |
| Takeover with release, taking control back | `takeover_releases_the_alternate_screen_and_the_exit_leaves_it` |
| Controller-driven size, input outside the control period | `the_controller_drives_the_size_and_a_hand_back_restores_the_remaining_size`, `stale_and_superseded_authority_is_rejected_and_never_re_enables_input` |
| Revoked paste closure across one-byte writes | `a_revoked_paste_is_closed_whole_across_one_byte_writes`, `bracketed_paste_is_exact_and_an_open_paste_is_closed_when_admission_ends` |
| A terminal reply never interleaves with input | `a_terminal_reply_never_interleaves_with_input_being_written` |
| Withheld Ready | `a_revision1_controller_withholding_ready_is_lost_at_the_restoration_deadline` |
| Repeated synchronized-update begins, and an open update's latch | `repeated_sync_begins_do_not_hold_a_grant`, `an_open_synchronized_update_is_not_released_before_its_deadline` |
| Stalled readers | `a_stalled_restoration_holds_command_final_only_until_its_reader_is_lost`, `revision1_connections_share_the_cap_and_a_stalled_monitor_is_lost_after_five_seconds` |
| Saturation | `saturation_backpressures_the_pty_without_blocking_probes_or_losing_bytes` |
| The headless policy of #78 | the HDL-01 rows of [Monitor acceptance scenarios](#monitor-acceptance-scenarios) |

The fixture reports any pixel size with its size, so every size assertion
also checks that none is reported.

A Kill of a fixture that reads its terminal is confirmed without checking
its result: the hang-up may end it by SIGHUP or by its own handling of the
failed read. Tests that check a hung-up command's result use fixtures that
sleep instead.

### Daemon wire revision 1: acceptance scenarios

`tests/revision1/` holds the Contract layer for revision 1 inside the same
test binary, so it shares the harness and fixtures above: raw-socket peers
that encode and decode with core's codec and check hand-written expectations
from the wire reference. The `script` fixture mode sets titles on command
(`title`, `title-empty`, `title-bytes`, `titles`, `reset`), prints `text`,
raw sequences (`print`, with `\e` for ESC) and fully coloured rows
(`colors`), and ends with `exit`, `exit N` or `signal N`;
its `long-titles A B` sets `t-A` through `t-B` each padded with `x` to 4,000
bytes; the `silent` mode runs `sleep 120`, which prints nothing at all (the
libtest fixture prints its harness lines); the `status-query-record` mode
records its input and, after the first read, prints an OSC 7501 capability
query and a device-attributes query; the `focus-record` mode records its input and turns focus reporting on and
off on command; the `close-terminal` mode closes its terminal, so the daemon
reads the end of its output, and runs on until `finish` appears, then exits
3.
Each test's doc comment cites the acceptance-matrix IDs it covers.

| Scenario | Named tests |
|---|---|
| COM-01 (Contract): hello advertises revision 1 beside the frozen tokens and `state`, exactly as the published complete reply does; Open selects only supported families; a peer that does not select an optional family receives nothing for it | `hello_advertises_revision1_beside_the_frozen_tokens_and_state`, `monitor_open_selects_supported_families_then_reports_initial_metadata`, `a_monitor_without_title_v1_receives_nothing_for_titles` |
| COM-01 (Contract, interactive case): an interactive Open selects the supported families, `focus-v1` included | `interactive_open_selects_supported_families_and_rejects_without_effects` |
| COM-01 (Contract after retirement): the daemon speaks only the frozen hello and revision 1. Traffic that never completes a valid hello of protocol 1 (a revision-1 or retired private type instead, malformed JSON, another protocol, a zero or oversized length, or the two-second deadline) is closed without a byte, and a hello at the 1,048,576-byte limit is answered; after hello every frame is revision 1, the first one too, so a retired private type or a second hello is `unknown_message_type` and an oversized or zero length is refused from its prefix | `traffic_instead_of_a_hello_closes_only_its_connection`, `probes_and_an_unfinished_hello_never_hold_a_revision1_controller`, `a_hello_at_the_frozen_limit_is_answered`, `revision1_traffic_that_breaks_the_rules_is_connection_fatal` |
| COM-02 (Contract): broken framing, non-strict JSON or UTF-8, malformed binary Input headers, server, retired private and unknown types, non-increasing request IDs and truncation are connection-fatal with their exact codes; a frame at the 1,048,576-byte limit is served and one byte more refused from its prefix; fragmented and coalesced frames are each answered in order; a Fatal error replaces a stalled stream's unstarted backlog without splitting a frame; out-of-state requests are rejected, never fatal | `revision1_traffic_that_breaks_the_rules_is_connection_fatal`, `malformed_binary_input_is_connection_fatal`, `a_request_at_the_frame_limit_is_served_and_one_byte_more_is_fatal`, `coalesced_and_fragmented_revision1_frames_are_each_answered_in_order`, `a_fatal_error_replaces_a_backlog_without_splitting_a_frame`, `requests_that_need_an_attachment_are_refused_and_the_stream_goes_on`, `requests_on_an_interactive_attachment_get_their_documented_results` |
| COM-03 (Contract, passive cases): Open, Leave and the rejections of a first role, a later Open or Kill, an early Leave or Acquire and every request that needs an attachment have their documented outcomes and effects, including during committed termination; a registration's Acquire converts it into an attachment that takes control, and is `session_terminating` during committed termination | `monitor_open_selects_supported_families_then_reports_initial_metadata`, `passive_requests_get_their_documented_outcomes`, `a_monitor_registers_and_leaves_during_committed_termination`, `requests_that_need_an_attachment_are_refused_and_the_stream_goes_on`, `a_registration_takes_control_only_by_its_own_acquire`, `a_committed_kill_overrides_succession` |
| COM-03 (Contract, interactive cases): interactive Open, Input, Resize, Ready, Acquire, Focus and Leave have their documented results and rejections, without effects when rejected; a terminating session rejects new interactive work with `session_terminating` while Leave still works | `interactive_open_is_granted_at_the_requested_geometry_in_the_documented_order`, `interactive_open_selects_supported_families_and_rejects_without_effects`, `requests_on_an_interactive_attachment_get_their_documented_results`, `committed_termination_rejects_new_interactive_work_and_leave_still_works` |
| COM-03 (Contract, Kill cases): a standalone Kill is answered only after the real hang-up grace, with its session metadata and socket removed and the command gone; a standalone Kill sent during the grace joins it without restarting it; a Kill's connection rejects a later Open or Kill, Leave and Acquire (`invalid_state`), Input, Resize and Ready (`not_attached`) and Focus (`unsupported_capability`); a Kill succeeds after a session failure once termination is confirmed, and fails with a partial result, keeping the result it knew, when its termination or cleanup did not complete | `a_standalone_kill_is_answered_after_termination_and_cleanup_and_later_kills_join_it`, `a_session_ending_without_final_evidence_sends_no_command_final`, `a_failed_kill_keeps_the_result_the_daemon_knew`, `a_failed_cleanup_fails_the_kill_but_keeps_the_confirmed_result`, `a_kill_succeeds_after_a_session_failure_once_termination_is_confirmed` |
| COM-03 and STA-06 (Contract, family dependency): `notification-v1` needs `status-v1`: required without it the Open is `unsupported_capability`, optional without it it is not selected; with it, it is; Open with `progress-v1` reports progress none | `notification_v1_is_selected_only_with_status_v1`, `monitor_open_selects_supported_families_then_reports_initial_metadata` |
| COM-04 (Contract): binary Input and Display carry `00 ff 41`, NUL and invalid UTF-8 unchanged, across Input requests and Display chunks | `binary_input_and_display_pass_unchanged` |
| COM-05 (Contract, Initial metadata case): Initial metadata is captured atomically and title changes follow it gap-free, including around a burst; it carries a command result already known, and that connection's one Command final follows only as the session ends (#259) | `title_changes_follow_initial_metadata_by_value_whatever_the_control`, `a_title_burst_is_reported_gap_free_around_an_atomic_initial_metadata`, `a_known_result_reaches_later_opens_and_rejected_first_roles` |
| COM-05 (Contract, interactive order): Response, Initial metadata, grant, Restoration begin, matching Display chunks and Restoration complete arrive in order under fragmented and coalesced I/O; a large restoration is produced as it drains, and completes before Command final when the command's output drained during it, as does the restoration of a monitor promoted after the drain; Geometry follows a Resize's acceptance | `interactive_open_is_granted_at_the_requested_geometry_in_the_documented_order`, `a_restoration_in_progress_when_the_drain_ends_completes_before_command_final`, `a_stalled_restoration_holds_command_final_only_until_its_reader_is_lost`, `ready_needs_the_complete_current_restoration`, `a_ready_controllers_resize_reports_the_actual_geometry_after_its_acceptance` |
| COM-05 (Contract, monitor-mode geometry): once a monitor-mode controller is ready, each applied Resize that changes the effective geometry is followed by Geometry, then a Restoration begin at that geometry before any Display at it, then a canonical restoration needing a new Ready; a host change that keeps the geometry is followed by nothing, before readiness too | `monitor_mode_restorations_are_canonical_at_the_effective_geometry_whatever_the_host` |
| COM-06 (Contract): exit zero, an arbitrary nonzero exit (143) and a signal (15, and 9 after a kill) each reach every opened connection as exactly one Command final, after every Display and Title changed of the command's last output even when the daemon reaps the command before reading it or a Kill arrives before that output drained, after a restoration in progress (its Restoration complete included) and any open synchronized update, which holds a derived controller's update, and still once a stalled restoration's reader is lost, after the restoration of the monitor that loss promotes (`successor`), which then confirms Ready to end the session, exit kept apart from signal; a known result reaches later Opens' Initial metadata, then their one Command final as the session ends, and rejected first roles' `known_final`, and no connection gets it twice | `command_final_reports_each_exit_and_signal_once`, `command_final_follows_the_last_output_of_a_command_that_exits`, `command_final_follows_the_commands_last_output` (Linux), `a_kill_during_the_drain_lets_the_commands_last_output_through` (Linux), `a_restoration_in_progress_when_the_drain_ends_completes_before_command_final`, `a_synchronized_update_open_at_exit_reaches_a_derived_controller_before_command_final`, `a_held_synchronized_update_reaches_a_derived_controller_before_command_final` (Linux), `a_stalled_restoration_holds_command_final_only_until_its_reader_is_lost`, `a_standalone_kill_is_answered_after_termination_and_cleanup_and_later_kills_join_it`, `a_known_result_reaches_later_opens_and_rejected_first_roles` |
| COM-06 (Fault, test hooks and fixture): a session ending without final evidence (`end-hangup`) sends no Command final, its standalone Kill fails with no result; the end of the command's output while it runs publishes nothing; a Kill keeps the result the daemon knew when the final kill (`end-hangup`) or the cleanup (`cleanup`) fails; a session failure (`pty-write`) still lets a Kill succeed | `a_session_ending_without_final_evidence_sends_no_command_final`, `the_end_of_the_commands_output_while_it_runs_is_no_final_result`, `a_failed_kill_keeps_the_result_the_daemon_knew`, `a_failed_cleanup_fails_the_kill_but_keeps_the_confirmed_result`, `a_kill_succeeds_after_a_session_failure_once_termination_is_confirmed` |
| CTL-01 (Contract): a passive registration never acquires control, of a detached session or one controlled elsewhere, and is not an attachment: hello `state` and so `list` are unchanged; core metadata flows before control; only its own Acquire takes control, at the session's geometry, with no input | `a_passive_registration_never_acquires_control_and_is_not_an_attachment`, `a_monitor_registers_and_leaves_during_committed_termination`, `a_registration_takes_control_only_by_its_own_acquire` |
| CTL-01 (Contract, starting sessions): a registration's Acquire of a starting session makes it the first attachment, in control at the session's geometry; watching alone is not attaching, so a session only watched still ends at its five-second initial deadline, and the registration gets the Command final | `a_registration_acquires_a_starting_session`, `watching_does_not_stop_the_initial_attach_rule` |
| COM-06 and CTL-01 (Contract, a held early exit, [#259](https://github.com/jackgeek/climon2/issues/259)): an ordinary session whose command exits 43 while only passive monitor connections watch stays `starting`; the monitor open at the exit gets its Command final at once, one opened later learns the result from its Initial metadata (final `failed` status) and gets its Command final, with that status, when the initial-attachment deadline ends the session; neither gets a second, and both streams end. The same holds when a standalone Kill ends the held session (the Kill confirms exit 43), when an interactive attachment that opened after the exit confirms Ready (it gets its Command final before Control revoked, `session_ending`), and when a Kill ends a session whose controller withholds Ready | `the_initial_deadline_publishes_a_held_result_to_every_monitor_once`, `a_kill_during_the_hold_publishes_the_held_result_to_every_monitor_once`, `an_attachment_confirming_ready_publishes_a_held_result_to_every_connection_once`, `a_known_result_reaches_later_opens_and_rejected_first_roles` |
| CTL-01 and COM-06 (Contract, Acquire after the Command final): until the output drained and the controller confirmed Ready, a registration that received the Command final still acquires; the previous controller gets its Command final, then Control revoked (`takeover`), and nobody gets a second Command final | `a_registration_acquires_after_the_command_final_until_the_session_ends` |
| CTL-03 (Contract, title case): titles reach monitors by value, in order and whatever the control, a promoted monitor's background control included; unset, empty, a value and unavailable stay distinct; an oversized title is ignored, never truncated | `title_changes_follow_initial_metadata_by_value_whatever_the_control`, `a_monitor_without_title_v1_receives_nothing_for_titles`, `a_title_burst_is_reported_gap_free_around_an_atomic_initial_metadata`, `an_oversized_title_is_ignored_never_truncated`, `titles_reach_an_attachment_as_inert_text` |
| CTL-04 (Contract): a departing controller, by Leave or by loss, hands control back to the most recently controlling remaining attachment first, otherwise the newest registration is promoted (`successor`) at the session's geometry, with no resize or input; the leave policy applies only after succession, so a registered monitor keeps the session alive; the promotion's restoration is canonical at the effective geometry, while an ordinary attachment on an oversized host keeps its Derived restoration; a loss after the command exited still promotes, and its restoration precedes Command final | `departures_hand_control_back_first_then_promote_the_newest_registration`, `monitor_mode_restorations_are_canonical_at_the_effective_geometry_whatever_the_host`, `a_stalled_restoration_holds_command_final_only_until_its_reader_is_lost` |
| CTL-04 (Fault, races): a Leave read after a promotion detaches the promoted attachment (`attachment_detached: true`), and one read first removes the registration, which is then never promoted; a committed Kill overrides succession, and a registration promoted before the Kill ends with the session as an attachment | `leave_racing_a_promotion_removes_the_registration_or_the_attachment_atomically`, `a_committed_kill_overrides_succession`, `a_registration_promoted_before_a_kill_ends_as_an_attachment` |
| CTL-04 (Fault, `rejection-overflow` and `CLIMON2_DAEMON_HOLD` hooks): a connection lost to its own output queue's overflow departs at once, before anything the daemon reads later in the same pass. A registration lost in the pass that reads the final attachment's Leave is never promoted, and the session stays detached. A non-controlling attachment lost in the pass that reads the controller's Leave is never handed control: an ordinary session stays detached as if it had left first, and a registration behind it is promoted under the very next control period. A controller lost in the pass that reads a registration's Leave promotes that registration first, so it leaves as an attachment | `a_registration_lost_to_an_overflow_is_never_promoted`, `an_attachment_lost_to_an_overflow_never_inherits_control`, `an_attachment_lost_to_an_overflow_yields_to_the_next_registration`, `a_controller_lost_to_an_overflow_promotes_before_a_later_leave` |
| STA-01, NOT-01 and STA-07 (Contract, controllable clock): changed output is running at its sample; ten seconds of inactivity report needs_attention with no transient idle and nothing a millisecond earlier; accepted Input clears pending and acknowledged attention before its acceptance; repeated Input re-arms the two-second window, ending idle exactly then; pending requests coalesce with hidden activity and no new time, acknowledgement keeps the episode, a later request starts the next one, and viewing never acknowledges; tied and distinct change times are exact | `inactivity_requests_attention_atomically_and_input_clears_both_attention_paths`, `repeated_input_re_arms_the_two_second_user_window`, `requests_coalesce_while_pending_and_re_arm_after_acknowledgement` |
| STA-01 (Contract, final states): the final status rides Command final, never Status changed, and a later deadline, Set viewing or Acknowledge changes nothing | `a_final_status_rejects_late_deadlines_viewing_and_acknowledgement`, `interactive_open_selects_supported_families_and_rejects_without_effects` |
| STA-02 (Contract, controllable clock): restyled characters, cursor-only moves, identical redraws, an identical screen swap and a resize are no activity; a change at the deadline is checked first; a resize during agent work restarts it; the last sample is clamped to the deadline; uninterrupted silence reports nothing | `only_changed_characters_or_title_are_activity_at_controlled_deadlines` |
| STA-07, DSC-01 and CTL-03 (Contract, status case): Initial metadata reports the stored status and time, idle since initialization, not the connection's time; status reaches monitors without control, the controller and a revoked attachment, and nothing reaches a connection without `status-v1` | `monitor_open_selects_supported_families_then_reports_initial_metadata`, `status_reaches_every_status_v1_connection_whatever_the_control`, `requests_coalesce_while_pending_and_re_arm_after_acknowledgement` |
| STA-08 (Contract and Fault, raw peer): Set viewing and Acknowledge need `status-v1` first, then a role, attachment, no committed termination (`session_terminating`), control, the current period and restoration; viewing true and Acknowledge need readiness, viewing false does not; Acknowledge needs the latest viewing true, which revocation clears; nothing pending is `changed:false`; viewing, Focus and terminal-query replies never clear attention | `set_viewing_and_acknowledge_check_authority_readiness_and_viewing`, `requests_that_need_an_attachment_are_refused_and_the_stream_goes_on`, `requests_on_an_interactive_attachment_get_their_documented_results`, `committed_termination_rejects_new_interactive_work_and_leave_still_works` |
| LIM-05 (Contract and Fault, episode case, `CLIMON2_DAEMON_FIRST_EPISODE`): the last episode identifier is issued and acknowledged; a later request starts no episode rather than wrap, and Acknowledge is then `resource_limit` | `exhausted_attention_episodes_are_reported_and_never_wrap` |
| STA-01 and STA-02 (Contract, initial silence): a command silent from startup (`silent`, `sleep 120`) reports nothing at any sample through 15,000 ms and stays idle and cleared | `a_session_silent_from_startup_stays_idle_and_cleared` |
| STA-01 (Contract, same-turn dispatch, turn barrier): an Input received while the daemon is held and served in the turn whose clock ends the user window re-arms it first: no idle, then idle two seconds later | `input_comes_before_a_timer_due_in_the_same_turn` |
| STA-08 (Contract and Fault, revision-1 controller): Input holding only focus reports leaves pending attention while reaching the application; the application's queries never reach the controller's Display, so no reply can; typed bytes clear it | `focus_reports_and_terminal_queries_never_clear_attention` |
| STA-01, STA-02, STA-07 and LIM-05 (pure, injected clock and random source): every transition and dispatch rule of both machines, initial silence, the sampling delay range and focus-report input | the 15 heuristic `status::tests` unit tests in `src/status.rs` |
| STA-03 (Contract, controllable clock): a capability query and child reports neither activate explicit detection nor aggregate; root `working` stays running silently past 40 s; root `done`/`error`/`idle` request attention, repeats coalesce, acknowledgement re-arms, and `done` is no completion; observed sequences never reach host output | `root_reports_drive_explicit_detection_while_children_and_queries_do_not` |
| STA-03 and STA-05 (Contract, controllable clock): the user window's end restores remembered root `working` only in the current explicit scope; `D` retires it | `user_window_expiry_restores_explicit_working_only_in_its_scope` |
| STA-03 (Contract, `status-query-record`): the daemon answers an OSC 7501 capability query itself, with every state and kind, before the engine's device-attributes reply | `program_status_queries_are_answered_locally_in_order` |
| STA-04 (Contract, controllable clock): root `clear` and `error` start the 10,000 ms fallback quiet deadline; characters, title and resize each restart it a millisecond before it, never as agent work; it ends at exactly ten seconds of quiet without attention and heuristics resume; the user window stays independent | `error_and_clear_start_the_fallback_quiet_deadline` |
| STA-04 (Contract, controllable clock): a full reset rebases heuristics, keeping an agent deadline, and acts as `clear` in explicit detection; a soft reset changes nothing; a shell boundary and a fresh root report end the grace; the command's exit during it is completed | `resets_reports_markers_and_the_final_exit_end_or_keep_detection_as_specified` |
| STA-05 (Contract, controllable clock): `A`, `C` and `D` (with an exit status) retire the scope with no attention, final status or deadline left; prompt rendering after `A` is no activity while input and root reports still apply; `C` and `B` end the exclusion, `B` with a fresh baseline; observed sequences never reach host output | `shell_markers_retire_scopes_and_exclude_prompt_rendering` |
| STA-06 (Contract) and CTL-03 (Contract, notifications and progress): OSC 9 text, completed OSC 99 and OSC 777 `notify` request attention and arrive in order with the episode; OSC 9;4 progress, incomplete OSC 99, auxiliary data alone, queries and other commands do not; every record reaches a passive registration, an attachment revoked by takeover and a background controller; a later Open's Initial metadata reports the retained progress | `notifications_and_progress_reach_every_subscribed_role_in_order` |
| LIM-01 (Contract, exec-provided assembly limits, controllable clock): records, bytes (identifiers included) and lifetime limits from the daemon's arguments apply at the limit and one past | `exec_provided_assembly_limits_apply_at_the_limit_and_one_past` |
| LIM-01 (Contract, release-constant bounds) and STA-06 (Contract, inert text): a 4,096-byte title and an 8,192-byte body of multi-byte characters split across OSC 99 chunks arrive whole, one byte more is ignored; base64 split inside a four-character group decodes once joined, and base64 that does not decode reports nothing; invalid UTF-8 is replaced; observed sequences never reach host output | `notification_titles_and_bodies_are_bounded_and_inert` |
| STA-06 and LIM-01 (pure): OSC 9, OSC 777 and OSC 99 parsing; progress syntax and normalization against the cached number; assembly by identifier, auxiliary data, records, bytes (identifiers and base64 as received) and lifetime at the limit and one past; base64 joined and decoded once at completion; text and record bounds, in one OSC 99 chunk and after base64 decoding too; a notification requests attention and carries the pending episode | the `status::signals::tests` OSC 9, 777 and 99 tests, the 3 `status::progress::tests`, the 11 `status::assembly::tests` and the notification test of `status::tests` |
| LIM-01 (pure, exec-provided assembly limits): the three assembly arguments are required and range-checked | the assembly test of `args::tests` in `src/args.rs` |
| STA-03, STA-04 and STA-05 (pure, injected clock and random source): root-only report effects, coalescing and remembered working; the fallback grace's restarts, exact expiry, independence from input and its ends; full resets; shell boundaries, prompt exclusion and what they keep; no signal after the final status | 9 further `status::tests` unit tests and `a_final_result_stops_observation_and_rejects_late_events` in `src/status.rs` |
| STA-03 and STA-05 (pure, upstream validation): root reports of each state, queries, child reports, malformed pairs, unknown states, every limit at the bound and one past, base64 text values, and OSC 133 markers | the first 6 `status::signals::tests` unit tests in `src/status/signals.rs` |
| CTL-05 (Contract and Fault, stale authority from a raw peer): Input, Resize and Ready under an earlier period or a superseded restoration are `stale_period` or `stale_restoration`; Ready needs the complete current restoration; late authority never re-enables input | `stale_and_superseded_authority_is_rejected_and_never_re_enables_input`, `ready_needs_the_complete_current_restoration` |
| INP-03 (Contract): bracketed-paste bytes reach the application exactly; an open application paste is closed when admission ends; a focus report never lands inside a paste that earlier Input requests carry | `bracketed_paste_is_exact_and_an_open_paste_is_closed_when_admission_ends`, `focus_reports_follow_the_input_accepted_before_them` |
| TRM-02 (Contract): the daemon alone answers session queries; titles stay inert text and title-looking output stays text | `session_queries_are_answered_by_the_daemon_alone`, `titles_reach_an_attachment_as_inert_text` |
| LIM-01 (Contract, queue bound): a monitor reading far slower than about 20 MB of titles arrive, though it makes progress throughout, is failed at its 16 MiB queue bound: its stream ends after a gap-free prefix, while another monitor receives every title, the fixture's output is read and a probe is answered; one behind by about 12 MB, within the bound, receives every title; unit cases: the queue holds exactly 16 MiB of frames and their per-frame overhead (524,288 empty frames) and refuses one byte more, and written or replaced frames release their charge | `a_monitor_falling_behind_beyond_its_queue_bound_is_failed_alone`, `a_monitor_behind_within_its_queue_bound_receives_every_title`, `connection::tests::the_output_queue_holds_sixteen_mebibytes_of_frames_and_their_overhead`, `connection::tests::written_and_replaced_frames_release_their_charge` (unit) |
| LIM-01 (Contract, existing bounds on revision 1): 64 revision-1 connections are served and a 65th is closed unanswered; a stalled monitor frees its slot only after five seconds without progress; a revision-1 controller withholding Ready is lost at the 30 s restoration deadline and promotes the registered monitor (the size-scaled part is the unit test `session::delivery::tests::the_deadline_allows_a_second_per_four_mebibytes_of_prefix`); a stalled restoration reader is lost after five seconds | `revision1_connections_share_the_cap_and_a_stalled_monitor_is_lost_after_five_seconds`, `a_revision1_controller_withholding_ready_is_lost_at_the_restoration_deadline`, `a_stalled_restoration_holds_command_final_only_until_its_reader_is_lost` |
| LIM-01 (Contract and Fault, frame and text bounds): a request at 1,048,576 bytes is served and one byte more refused; a 1,048,576-byte server frame is queued and one byte more never sent; a title of 4,096 bytes arrives whole and one past it is ignored, never truncated; the most escaped title fits a 16,384-byte metadata record as Title changed and Initial metadata | `a_request_at_the_frame_limit_is_served_and_one_byte_more_is_fatal`, `connection::tests::a_server_frame_over_the_limit_is_never_queued` (unit), `an_oversized_title_is_ignored_never_truncated`, `the_most_escaped_title_fits_a_metadata_record` |
| LIM-05 (Contract and Fault, `CLIMON2_DAEMON_ISSUED` hook): the last control period and restoration ID are issued, then an interactive Open (its stream ends) and an Acquire by a non-controlling attachment or a registration are refused with `resource_limit` without effects, while the controller's own Acquire is a no-op; its Leave hands back to nobody, so the other attachment's stream ends and the registration is not promoted; a monitor-mode Resize takes the last restoration ID and the next one needing a new ID is `resource_limit`, as is an interactive Open with periods left; the last request ID is served and nothing follows it, one past it is `invalid_request_id`; pure decisions for each refusal, hand-back and promotion | `the_last_control_period_is_issued_then_new_control_is_refused_without_effects`, `the_last_restoration_id_is_issued_then_a_resize_needing_another_is_refused`, `the_last_request_id_is_served_and_no_request_follows_it`, `control::tests::the_last_counters_are_issued_then_an_attach_is_refused_without_effects` and the other `control::tests` exhaustion cases (unit) |
| FOC-01 (Contract): Focus emits no duplicates, newly enabled reporting receives the current state, and new readiness synchronizes the latest intent without replay; a focus report follows the Input accepted before it, however the requests were read | `focus_reaches_the_application_once_per_change_and_resynchronizes_on_new_readiness`, `focus_reports_follow_the_input_accepted_before_them` |

Every wait for a daemon is bounded: `Session::finish`, its drop and
`run_unsupervised` kill and reap a daemon still running 20 seconds after it
should have ended, so a wedged daemon fails its test instead of hanging it (and
with it a whole mutation run).

On macOS, session-leader exit may revoke the PTY immediately, so the final
drain fixture permits EOF/EIO before the idle/cap clocks expire; it does not
claim those clocks necessarily expire in that OS scenario. The command's last
close of its terminal waits for the daemon to read the queued output, but the
macOS kernel discards that output when no reader takes it within its close
timeout (0.6 s), and the session leader's exit then revokes the terminal. So
on macOS the daemon reads the command's output before it can reap the
command, and the tests that stop the daemon across the exit to reap first
(`command_final_follows_the_commands_last_output`,
`a_kill_during_the_drain_lets_the_commands_last_output_through`,
`a_held_synchronized_update_reaches_a_derived_controller_before_command_final`)
run on Linux only, where a closed terminal's unread output stays readable;
`command_final_follows_the_last_output_of_a_command_that_exits` and
`a_synchronized_update_open_at_exit_reaches_a_derived_controller_before_command_final`
cover the same orders on both. The restoration tests that stop the daemon
across the exit
(`a_restoration_in_progress_when_the_drain_ends_completes_before_command_final`,
`a_stalled_restoration_holds_command_final_only_until_its_reader_is_lost`)
run on both: the daemon has read all of the command's output before they
stop it, so nothing is left to discard.

With `test-hooks` only, `CLIMON2_DAEMON_FAULT=panic|never-ready` injects named
pre-command startup faults. Their test checks capped panic capture, real
five-second startup timeout and absence of spawned command/files.
`CLIMON2_DAEMON_FAULT=pty-write` makes every PTY write fail, an authoritative
session failure: the controller receives the hung-up command's Command
final, its release bytes, then Control revoked (`session_ending`).
`CLIMON2_DAEMON_FAULT=pty-write-partial` makes every PTY write take at most one
byte after a 5 ms pause, forcing the partial writes the ordering tests need.
`CLIMON2_DAEMON_CLOCK=PATH` replaces the status detector's clock (only
that clock; every other daemon deadline stays real): PATH holds
milliseconds, read as both the monotonic reading and the UTC time since the
Unix epoch, at the start of every turn, and once a turn that read a new
value ends, the daemon writes the value to `PATH.ack`, so a test knows that
turn's records are queued. While `PATH.hold` exists the daemon does not
start its next turn and writes `PATH.held`; whatever it receives meanwhile is
served in the turn that follows, at the clock set before `PATH.hold` is
removed. A failure to write these files is a session I/O failure.
`CLIMON2_DAEMON_FIRST_EPISODE=N` starts attention
episodes at counter N, to reach exhaustion.
`CLIMON2_DAEMON_FAULT=end-hangup` makes the end of every hang-up fail, as one
the daemon cannot confirm: no Command final unless the command was reaped
before, and a failed standalone Kill. `CLIMON2_DAEMON_FAULT=cleanup` makes the
in-loop removal of the session metadata and socket fail (the daemon's final
cleanup still removes them): a failed standalone Kill that keeps its
confirmed result.
`CLIMON2_DAEMON_FAULT=rejection-overflow` makes every rejection Response
overflow its connection's output queue, so the connection is lost inside the
pass that read its request. `CLIMON2_DAEMON_HOLD=<path>` makes
the session loop read no incoming frame while `<path>` exists, announcing each
held pass by creating `<path>` with its extension replaced by `.held`; a test
releases frames it wrote on several connections into one pass
(`a_held_session_loop_reads_no_frame_until_released` shows a held loop reads
nothing: a request stays unanswered through two held passes, then is answered
once the hold is removed).
`CLIMON2_DAEMON_ISSUED=<period>,<restoration>` starts the session as if
these were the control period and restoration ID issued last, so a test
reaches their exhaustion.
These fault
branches and the extra generic fixture binary are absent from ordinary release
builds. The ignored libtest function is a subprocess fixture, not a skipped
requirement. Every controlled final path checks owned process/socket/metadata
cleanup; logs intentionally survive.

### Monitor acceptance scenarios

Contract-layer cases of the [acceptance matrix](../../docs/monitor-acceptance.md),
through the real daemon binary and raw-socket revision-1 peers in `tests/revision1/`,
supplemented by pure decisions in the control state machine and the session
loop. Each test's doc comment cites its ID and case. A headless session's
fixture ends itself after two minutes (`DAEMON_FIXTURE_LIFE`), so a test killed
before it ends its session cannot leave that session running forever.

| Scenario | Named test |
|---|---|
| HDL-01 zero clients from startup: `detached` from readiness, outlives the initial attach window that ends an ordinary session started after it, exit reported to a later attachment | `a_headless_session_advertises_detached_and_outlives_the_initial_attach_window` |
| HDL-01 command completion with zero clients and no initial attach hold: the headless session is gone while an ordinary reference session started before it still advertises `starting` | `a_headless_session_ends_with_its_command_without_the_initial_attach_hold` |
| HDL-01 no initial attach hold, pure decision: only an unattached ordinary session keeps an early exit until its initial deadline | `session::tests::only_an_unattached_ordinary_session_holds_an_early_exit_until_its_initial_deadline` (unit) |
| HDL-01 survives the deliberate Leave and the unexpected loss of its last attachment, then accepts another | `a_headless_session_survives_the_leave_and_the_loss_of_its_last_attachment` |
| HDL-01 and CTL-04 headless succession over revision-1 attachments: the last attachment's loss promotes a registered monitor (`successor`), and the promoted monitor's own loss leaves the session detached; its command still reaches a later attachment | `a_headless_session_promotes_a_monitor_and_outlives_its_loss` |
| HDL-01 pure decisions: `detached` from startup, the initial deadline ends nothing, the last attachment's detach or loss keeps it `detached`, Kill ends it | `control::tests::a_new_headless_session_is_detached_and_outlives_its_initial_deadline`, `control::tests::a_headless_session_survives_the_departure_of_its_last_attachment` (unit) |
| HDL-01 Kill with zero clients, confirmed after cleanup | `kill_ends_a_headless_session_with_zero_clients` |
| HDL-01 ordinary leave policy unchanged | `every_unexpected_departure_hangs_up_then_kills_after_the_real_grace`, `an_unattached_session_ends_after_the_initial_deadline` |
| Startup policy: a missing or unknown `--lifetime` is an invalid invocation | `repeated_unknown_zero_pid_and_relative_options_are_invalid_invocations`; `args::tests::daemon_requires_a_known_lifetime` (unit) |
