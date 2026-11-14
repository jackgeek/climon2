# climon2-daemon

Real session daemon library with a thin executable on macOS and Linux; no client
implementation or descriptor passing. The daemon owns the VT engine, bounded
scrollback, terminal restoration and live presentation.

## CLI-to-daemon API

```text
climon2-daemon --session-id GUID --home CANONICAL_ABSOLUTE_HOME \
  --socket ABSOLUTE_SOCKET --version SEMVER --cli-pid PID \
  --lifetime ordinary|headless --readiness-fd FD --rows ROWS --cols COLS \
  --scrollback-max-rows ROWS --cwd ABSOLUTE_CWD \
  [--log FILTER] -- COMMAND [ARG...]
```

Options are private, order-independent, unique and required except `--log`.
`--scrollback-max-rows` sets the VT engine's retained normal-screen history.
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
output. Between iterations the loop waits up to 2 ms for PTY readability (poll,
no read). The VT's terminal replies go to the PTY; its filtered live output goes
to the controller only.

Authenticated connections use core's typed protocol messages and connection
states. Incremental nonblocking decoding enforces an absolute two-second hello
deadline and bounded frames, and reads no further than the frame it is
decoding. Up to 64 concurrent connections are admitted;
a further connection is closed unanswered. Each connection's outgoing queue is
capped at 16 MiB. Restoration prefixes are produced incrementally, whenever less
than 1 MiB is queued, while live output waits behind them; a controller whose
pending live output plus queued bytes would exceed 16 MiB, or any connection
that accepts none of its queued bytes for five seconds, is treated as lost. A
controller that does not send `Ready` within 30 s plus one second per 4 MiB of
restoration prefix is also lost. Probes remain serviceable throughout.

### Daemon wire revision 1 beside the private protocol

Hello advertises the frozen tokens and `state`, then `daemon-wire-v1`,
`monitor-v1`, `title-v1` and `focus-v1`, and `daemon_wire_v1` with
1,048,576-byte request and server frame limits
(`core::hello::advertise_revision1`). The first frame after hello selects
the protocol by its type: a revision-1 type (`0x30`–`0x39` or
`0x40`–`0x4d`, `core::wire::defines`) selects
[daemon wire revision 1](../../docs/protocol/daemon-client/core-v1.md) for the
rest of the connection; any other type continues the private protocol, which
still serves attach and kill as before. That frame's length is checked
under the frozen hello limit before its type is known. Every revision-1 frame
is read and written by core's `wire` codec: a frame its prefix declares
oversized is refused before its body arrives, request IDs must strictly
increase (a rejected request still consumes its ID), and a stream ending inside
a frame is `truncated_frame`. A connection-fatal failure queues a best-effort
Fatal error, replacing unsent output, and closes the connection; a stream
ending between frames just closes. Frames read before the end are still
served, so a peer may send Leave and half-close.

A monitor Open registers a **passive monitor connection**. It is not an
attachment: control, the leave policy and the hello `state` (so `climon2
list`'s STATE and CLIENTS) never count it, and it never acquires control.
Open selects the requested families this daemon supports (required ones must
all be; unknown or unsupported optional ones are dropped) under a fresh
connection GUID. Its success Response is followed by one Initial metadata
record captured in the same loop step: the title by value when `title-v1` is
selected (unavailable otherwise), status and progress explicitly unavailable,
and the command's result once the daemon knows it (null before).
Every later change of the application title that the VT reports
(`Fed::titles`, one per change, by value) goes as a Title changed to each
registration that selected `title-v1`, whatever the control state; titles are
inert text, never logged. A monitor Open is accepted during committed
termination too, since that rejects new interactive work only.

Leave removes the registration in the same step, answers `left` with
`attachment_detached: false`, and closes the stream; nothing sent after it is
served. Leave before any Open, and a later Open or Kill on a registered
connection, are `invalid_state` and the stream goes on. A rejected first role
(an unsupported revision, or an unknown or unsupported required family) has
no effects, carries the command's result as `known_final` once the daemon
knows it, and ends the stream. On a connection without an attachment, Input,
Resize and Ready are rejected without effects and the stream goes on, with
any registration kept: `invalid_state` before any Open, `not_attached` on a
registration or a standalone Kill's connection. Focus is
`unsupported_capability` unless the connection's Open selected `focus-v1`,
then `not_attached` on a registration. Set viewing and Acknowledge are
`invalid_state` on every connection until #91 serves them, and Acquire is
`internal_error` on a registration until #89 serves it, and `invalid_state`
before any Open or on a standalone Kill's connection, which leave nothing to
convert. Registrations share the
connection cap, the 16 MiB queue and the five-second no-progress loss.

An interactive Open attaches an ordinary **interactive attachment**: the same
attachment, control and leave policy as a private-protocol attach, which it
can take control from and hand back to. Its success Response and Initial
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

The daemon keeps the command's result as its wait status gives it
(`core::wire::CommandResult`): a normal exit's raw status, or a signal's
number, never folded together; only the private protocol's `Exit` and
`Terminated` fold a signal into 128 plus its number. It publishes the result
once, when it is final, so that no more of the command's output follows
Command final: for a command that exited by itself, when the PTY drain after
the reap ends (end of output, `PTY_DRAIN_IDLE` 100 ms without output, or
`PTY_DRAIN_CAP` 1 s after the reap), in the pass that delivered the drained
output and before the exit report waits for `Ready` or the initial
attachment. A termination (`Kill`, final unexpected departure, authoritative
failure such as a failed PTY read, or initial deadline) that begins after the
reap does not cut that drain short: `begin_shutdown` first observes the
command, and if it has exited the hang-up waits until the drain ends and
Command final is published (owner decision on #88, 2026-11-14). A failed
read during the drain counts as idle, so the drain goes on and ends as usual.
After a hang-up of a running command, which stops PTY reads, the result is
published when the end of the command is learned at the end of the grace
(`HANGUP_GRACE`), with any result already reaped. Every passive monitor connection and interactive
attachment then receives Command final, with a null status while
`status-v1` is not served; the controller receives the command's last output
before it, then its release and Control revoked (`session_ending`). A
restoration still in progress when the drain ends, or a derived update held
by a synchronized update, can still deliver after it (follow-up #203). Later Opens get the result in their Initial metadata
and no Command final, and later first-role rejections carry it as
`known_final`. A hang-up the daemon cannot confirm publishes the result
only if the command was already reaped, and nothing otherwise; the private
protocol then reports 125 as before.

A valid standalone Kill takes the Kill role (never an attachment) and
commits termination through the same control as the private `Kill`, so
kills of either protocol join one hang-up grace without restarting it. It is
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

Control is a pure state machine (`control.rs`): each attach, take-control and
hand-back is granted at once under a new authority, a control period with its
first restoration ID (attaches in one iteration are each granted and the last
wins), the previous controller gets its release then `Revoked`, and a
departing controller hands back to the most recently controlling attachment
still present. A host change before readiness replaces the restoration
within its period under a new restoration ID, and Ready needs the complete
current restoration. Both protocols share these counters; the private
protocol names each restoration by its ID in `Granted`, `RestoreEnd` and
`Ready`, which its client only echoes. Only the controller's size drives the
PTY and VT; the session geometry is its host size clamped to 2..=500 columns
and 1..=200 rows, and a host larger than that gets Derived presentation.
Resizes are coalesced to one per iteration.

Early exits and PTY output remain available for the first attachment of an
ordinary session: the exit of a command that ends first is reported once that
attachment's restoration is confirmed, or after the five-second initial attach
window. A headless session has no initial attach window, so its exit is reported
at once unless an attachment is restoring. Final status comes
exclusively from shared exact `waitpid`, never portable-pty waiting. After
reaping, drain ends at EOF/EIO, 100 ms of actual PTY idle, or one second.
Metadata and the socket are then removed while connections stay open, the
controller gets its release bytes, every private attachment gets `Exit` (a
revision-1 controller gets Control revoked), and final flushing is bounded
to one further second. A receiver that
never reads cannot be guaranteed delivery after this bound.

The leave policy: while another attachment remains, the session continues;
after an accepted final detach it keeps running with no idle timeout; a final
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
reaps the command. Only after metadata and socket removal does each attachment
get its release then `Exit` (or `Error` after a failure), and each kill
requester `Terminated` (a standalone Kill its `termination` answer); a failed
cleanup never confirms a kill. If the loop
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

`tests/lifecycle.rs` exercises the real daemon and PTY using raw Unix socket
clients and owned generic terminal fixtures:

- `ready_publishes_private_metadata_and_exit_is_last`: PID/start/version,
  initial size, permissions, final bytes, exact status, cleanup.
- `long_home_and_executable_with_maximum_session_socket_start_and_clean_up`:
  home/executable paths longer than 103 bytes, a maximum-length session socket,
  readiness, exact command exit and owned startup/socket/process cleanup.
- `runtime_argv_cwd_and_inherited_environment_are_exact_not_display_metadata`:
  literal/empty/option-looking/non-UTF8 argv, cwd, whole environment,
  absent/empty/non-UTF8 SHELL, explicit logging filter and diagnostic privacy.
- `exact_signal_exit_is_delivered`,
  `early_exit_is_delivered_after_restore_readiness`,
  `an_exit_waits_for_the_controllers_ready`,
  `final_drain_eof_or_deadline_is_bounded_and_leaves_no_descendants`:
  real signal mapping, early exit (held while the controller withholds
  `Ready`), final drain ordering and process groups. Once the command has
  ended, the session finishes at the one-second drain cap even with output
  still queued, or sooner when every connection has drained: the unit test
  `an_ending_session_finishes_at_its_deadline_or_once_drained` in
  `src/session.rs` checks that decision without wall-clock bounds.
- `concurrent_probes_absolute_hello_deadline_and_second_attach`,
  `malformed_unknown_oversized_and_out_of_state_messages_close_only_the_probe`:
  frozen hello, concurrent probes, one Attach per connection and terminal
  protocol errors, each with its exact `Error` text: decoding failures name
  themselves, other framing failures say `invalid session protocol`. An
  unfinished hello that writes a byte every 50 ms still ends two seconds after
  connecting, however long the rest of the test takes.
- `a_connection_beyond_the_cap_is_closed_unanswered`: 64 connections are served
  and a 65th is closed unanswered while the session goes on.
- `initial_fallback_and_resize_use_the_real_pty`,
  `raw_input_output_are_byte_transparent`: real dimensions and binary framing.
  The fixture reports any pixel size with the size, so every size assertion
  also checks that none is reported.
- `saturation_backpressures_pty_without_blocking_probes_or_losing_bytes`:
  an 8 MiB burst to a client that is not reading yet arrives exactly (the whole
  stream is compared, then the release) within 15 s, which a one-read-per-tick
  pump cannot meet, while probes stay serviceable.
- `every_controlling_leave_hangs_up_then_kills_after_real_grace`:
  EOF, Error, repeated hello, unknown message, TERM and unwritten input all use
  the real five seconds and leave no command process.
- Continuity:
  `attached_client_restores_a_waiting_alternate_screen_before_input_is_ready`,
  `detach_releases_then_confirms_and_a_reattachment_resumes`,
  `takeover_revokes_with_release_and_exit_leaves_the_alternate_screen`,
  `the_controller_drives_the_size_and_input_outside_its_period_is_dropped`,
  `resize_before_ready_replaces_the_restoration_period`,
  `a_resize_burst_is_one_pty_resize_to_the_last_size`,
  `an_unfinished_token_still_restores_and_kill_confirms_termination`,
  `repeated_sync_begins_do_not_hold_a_grant`,
  `an_oversized_host_is_served_a_derived_presentation`,
  `retention_with_an_unfinished_prefix_keeps_only_the_newest_rows` and
  `identical_rows_evict_and_cleared_history_has_no_stale_notice`.
- PTY write ordering:
  `a_revoked_paste_is_closed_before_the_next_controllers_input` and
  `a_terminal_reply_never_interleaves_with_input_being_written` (both under
  `pty-write-partial`), and
  `a_kill_with_an_open_application_paste_still_terminates`.
- Shutdown and departures: `attach_during_termination_is_refused`,
  `hanging_up_types_nothing_into_the_command_terminal` (Linux: ptrace records
  every write to the master from `Kill` until its last handle closes),
  `the_hang_up_grace_keeps_serving_and_a_second_kill_joins_it`,
  `kill_after_an_unreported_exit_confirms_that_status`,
  `an_unattached_session_ends_after_the_initial_deadline`,
  `a_stalled_reader_is_dropped_after_five_seconds_without_progress`,
  `withheld_ready_is_an_unexpected_departure_after_the_restore_deadline` and
  `a_pty_write_failure_releases_then_reports_error`.
- `leave_kills_every_job_of_the_command_session_but_not_new_sessions`: under
  job-control `bash -i`, HUP-ignoring foreground and background jobs (their own
  process groups) are gone after the real grace; a daemonized new-session
  process survives and is killed by the test.
- `natural_exit_kills_jobs_left_in_the_command_session`: the shell's
  HUP-ignoring background job, orphaned off the terminal when the shell exits,
  is killed after the final status.
- Startup failure/invalid invocation and daemon-ignore-signal tests
  (`invalid_cwd_missing_command_and_unsafe_sessions_are_startup_failures`,
  `malformed_daemon_invocation_is_own_failure_with_no_stdout`,
  `daemon_ignores_hup_int_pipe_without_ending_the_command`) cover safe launch
  diagnostics, refused permissions and ignored HUP/INT/PIPE.
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
- `exec_helper_confirms_launch_failure_codes_and_exits_with_them` drives the
  private `--exec-command` helper directly: 127 for a missing command in a
  usable cwd, otherwise 126.

### Daemon wire revision 1: acceptance scenarios

`tests/revision1/` holds the Contract layer for revision 1 inside the same
test binary, so it shares the harness and fixtures above: raw-socket peers
that encode and decode with core's codec and check hand-written expectations
from the wire reference. The `script` fixture mode sets titles on command
(`title`, `title-empty`, `title-bytes`, `titles`, `reset`), prints `text` and
fully coloured rows (`colors`), and ends with `exit`, `exit N` or `signal N`;
the `focus-record` mode records its input and turns focus reporting on and
off on command; the `close-terminal` mode closes its terminal, so the daemon
reads the end of its output, and runs on until `finish` appears, then exits
3.
Each test's doc comment cites the acceptance-matrix IDs it covers.

| Scenario | Named tests |
|---|---|
| COM-01 (Contract): hello advertises revision 1 beside the frozen tokens and `state`, exactly as the published complete reply does; Open selects only supported families; a peer that does not select an optional family receives nothing for it | `hello_advertises_revision1_beside_the_frozen_tokens_and_state`, `monitor_open_selects_supported_families_then_reports_initial_metadata`, `a_monitor_without_title_v1_receives_nothing_for_titles` |
| COM-01 (Contract, interactive and parity cases): an interactive Open selects the supported families, `focus-v1` included; takeover, hand-back and the leave policy match the private protocol, across both protocols | `interactive_open_selects_supported_families_and_rejects_without_effects`, `takeover_hand_back_and_leave_policy_match_the_private_protocol` |
| COM-02 (Contract): broken framing, non-strict JSON or UTF-8, malformed binary Input headers, server, private and unknown types, non-increasing request IDs and truncation are connection-fatal with their exact codes; a frame at the 1,048,576-byte limit is served and one byte more refused from its prefix; fragmented and coalesced frames are each answered in order; a Fatal error replaces a stalled stream's unstarted backlog without splitting a frame; out-of-state requests are rejected, never fatal | `revision1_traffic_that_breaks_the_rules_is_connection_fatal`, `malformed_binary_input_is_connection_fatal`, `a_request_at_the_frame_limit_is_served_and_one_byte_more_is_fatal`, `coalesced_and_fragmented_revision1_frames_are_each_answered_in_order`, `a_fatal_error_replaces_a_backlog_without_splitting_a_frame`, `requests_that_need_an_attachment_are_refused_and_the_stream_goes_on`, `requests_on_an_interactive_attachment_get_their_documented_results` |
| COM-03 (Contract, passive cases): Open, Leave and the rejections of a first role, a later Open or Kill, an early Leave or Acquire and every request that needs an attachment have their documented outcomes and effects, including during committed termination | `monitor_open_selects_supported_families_then_reports_initial_metadata`, `passive_requests_get_their_documented_outcomes`, `a_monitor_registers_and_leaves_during_committed_termination`, `requests_that_need_an_attachment_are_refused_and_the_stream_goes_on`, `acquire_is_refused_without_effects_until_it_is_served` |
| COM-03 (Contract, interactive cases): interactive Open, Input, Resize, Ready, Acquire, Focus and Leave have their documented results and rejections, without effects when rejected; a terminating session rejects new interactive work with `session_terminating` while Leave still works | `interactive_open_is_granted_at_the_requested_geometry_in_the_documented_order`, `interactive_open_selects_supported_families_and_rejects_without_effects`, `requests_on_an_interactive_attachment_get_their_documented_results`, `committed_termination_rejects_new_interactive_work_and_leave_still_works`, `takeover_hand_back_and_leave_policy_match_the_private_protocol` |
| COM-03 (Contract, Kill cases): a standalone Kill is answered only after the real hang-up grace, with its session metadata and socket removed and the command gone; standalone and private kills sent during the grace join it without restarting it; a Kill's connection rejects a later Open or Kill, Leave and Acquire (`invalid_state`), Input, Resize and Ready (`not_attached`) and Focus (`unsupported_capability`); a Kill succeeds after a session failure once termination is confirmed, and fails with a partial result, keeping the result it knew, when its termination or cleanup did not complete | `a_standalone_kill_is_answered_after_termination_and_cleanup_and_later_kills_join_it`, `a_session_ending_without_final_evidence_sends_no_command_final`, `a_failed_kill_keeps_the_result_the_daemon_knew`, `a_failed_cleanup_fails_the_kill_but_keeps_the_confirmed_result`, `a_kill_succeeds_after_a_session_failure_once_termination_is_confirmed` |
| COM-04 (Contract): binary Input and Display carry `00 ff 41`, NUL and invalid UTF-8 unchanged, across Input requests and Display chunks | `binary_input_and_display_pass_unchanged` |
| COM-05 (Contract, Initial metadata case): Initial metadata is captured atomically and title changes follow it gap-free, including around a burst; it carries a command result already known, and no Command final follows it | `title_changes_follow_initial_metadata_by_value_whatever_the_control`, `a_title_burst_is_reported_gap_free_around_an_atomic_initial_metadata`, `a_known_result_reaches_later_opens_and_rejected_first_roles` |
| COM-05 (Contract, interactive order): Response, Initial metadata, grant, Restoration begin, matching Display chunks and Restoration complete arrive in order under fragmented and coalesced I/O; a large restoration is produced as it drains; Geometry follows a Resize's acceptance | `interactive_open_is_granted_at_the_requested_geometry_in_the_documented_order`, `ready_needs_the_complete_current_restoration`, `a_ready_controllers_resize_reports_the_actual_geometry_after_its_acceptance` |
| COM-06 (Contract): exit zero, an arbitrary nonzero exit (143) and a signal (15, and 9 after a kill) each reach every opened connection as exactly one Command final, after every Display and Title changed of the command's last output even when the daemon reaps the command before reading it or a Kill arrives before that output drained, exit kept apart from signal, while the private protocol keeps `Exit` 0, 143 and 143 and `Terminated` 137; a known result reaches later Opens' Initial metadata and rejected first roles' `known_final`, and is never published twice | `command_final_reports_each_exit_and_signal_once`, `command_final_follows_the_commands_last_output`, `a_kill_during_the_drain_lets_the_commands_last_output_through`, `a_standalone_kill_is_answered_after_termination_and_cleanup_and_later_kills_join_it`, `a_known_result_reaches_later_opens_and_rejected_first_roles` |
| COM-06 (Fault, test hooks and fixture): a session ending without final evidence (`end-hangup`) sends no Command final, its standalone Kill fails with no result and the private kill keeps `Terminated` 125; the end of the command's output while it runs publishes nothing; a Kill keeps the result the daemon knew when the final kill (`end-hangup`) or the cleanup (`cleanup`, private `Error`) fails; a session failure (`pty-write`) still lets a Kill succeed | `a_session_ending_without_final_evidence_sends_no_command_final`, `the_end_of_the_commands_output_while_it_runs_is_no_final_result`, `a_failed_kill_keeps_the_result_the_daemon_knew`, `a_failed_cleanup_fails_the_kill_but_keeps_the_confirmed_result`, `a_kill_succeeds_after_a_session_failure_once_termination_is_confirmed` |
| CTL-01 (Contract): a passive registration never acquires control, of a detached session or one controlled elsewhere, and is not an attachment: hello `state` and so `list` are unchanged; core metadata flows before control | `a_passive_registration_never_acquires_control_and_is_not_an_attachment`, `a_monitor_registers_and_leaves_during_committed_termination` |
| CTL-03 (Contract, title case): titles reach monitors by value, in order and whatever the control; unset, empty, a value and unavailable stay distinct; an oversized title is ignored, never truncated | `title_changes_follow_initial_metadata_by_value_whatever_the_control`, `a_monitor_without_title_v1_receives_nothing_for_titles`, `a_title_burst_is_reported_gap_free_around_an_atomic_initial_metadata`, `an_oversized_title_is_ignored_never_truncated`, `titles_reach_an_attachment_as_inert_text` |
| CTL-05 (Contract and Fault, stale authority from a raw peer): Input, Resize and Ready under an earlier period or a superseded restoration are `stale_period` or `stale_restoration`; Ready needs the complete current restoration; late authority never re-enables input | `stale_and_superseded_authority_is_rejected_and_never_re_enables_input`, `ready_needs_the_complete_current_restoration` |
| INP-03 (Contract): bracketed-paste bytes reach the application exactly; an open application paste is closed when admission ends; a focus report never lands inside a paste that earlier Input requests carry | `bracketed_paste_is_exact_and_an_open_paste_is_closed_when_admission_ends`, `focus_reports_follow_the_input_accepted_before_them` |
| TRM-02 (Contract): the daemon alone answers session queries; titles stay inert text and title-looking output stays text | `session_queries_are_answered_by_the_daemon_alone`, `titles_reach_an_attachment_as_inert_text` |
| FOC-01 (Contract): Focus emits no duplicates, newly enabled reporting receives the current state, and new readiness synchronizes the latest intent without replay; a focus report follows the Input accepted before it, however the requests were read | `focus_reaches_the_application_once_per_change_and_resynchronizes_on_new_readiness`, `focus_reports_follow_the_input_accepted_before_them` |

`acquire_is_refused_without_effects_until_it_is_served` pins the interim
`internal_error` refusal of Acquire on a registration, which #89 replaces;
it covers no matrix scenario.

Every wait for a daemon is bounded: `Session::finish`, its drop and
`run_unsupervised` kill and reap a daemon still running 20 seconds after it
should have ended, so a wedged daemon fails its test instead of hanging it (and
with it a whole mutation run).

On macOS, session-leader exit may revoke the PTY immediately, so the final
drain fixture permits EOF/EIO before the idle/cap clocks expire; it does not
claim those clocks necessarily expire in that OS scenario.

With `test-hooks` only, `CLIMON2_DAEMON_FAULT=panic|never-ready` injects named
pre-command startup faults. Their test checks capped panic capture, real
five-second startup timeout and absence of spawned command/files.
`CLIMON2_DAEMON_FAULT=pty-write` makes every PTY write fail, an authoritative
session failure: the controller receives its release bytes, then `Error`.
`CLIMON2_DAEMON_FAULT=pty-write-partial` makes every PTY write take at most one
byte after a 5 ms pause, forcing the partial writes the ordering tests need.
`CLIMON2_DAEMON_FAULT=end-hangup` makes the end of every hang-up fail, as one
the daemon cannot confirm: no Command final unless the command was reaped
before, and a failed standalone Kill. `CLIMON2_DAEMON_FAULT=cleanup` makes the
in-loop removal of the session metadata and socket fail (the daemon's final
cleanup still removes them): a failed standalone Kill that keeps its
confirmed result, and `Error` for private kill requesters.
These fault
branches and the extra generic fixture binary are absent from ordinary release
builds. The ignored libtest function is a subprocess fixture, not a skipped
requirement. Every controlled final path checks owned process/socket/metadata
cleanup; logs intentionally survive.

### Monitor acceptance scenarios

Contract-layer cases of the [acceptance matrix](../../docs/monitor-acceptance.md),
through the real daemon binary and raw-socket peers in `tests/lifecycle.rs`,
supplemented by pure decisions in the control state machine and the session
loop. Each test's doc comment cites its ID and case. A headless session's
fixture ends itself after two minutes (`DAEMON_FIXTURE_LIFE`), so a test killed
before it ends its session cannot leave that session running forever.

| Scenario | Named test |
|---|---|
| HDL-01 zero clients from startup: `detached` from readiness, outlives the initial attach window that ends an ordinary session started after it, exit reported to a later attachment | `a_headless_session_advertises_detached_and_outlives_the_initial_attach_window` |
| HDL-01 command completion with zero clients and no initial attach hold: the headless session is gone while an ordinary reference session started before it still advertises `starting` | `a_headless_session_ends_with_its_command_without_the_initial_attach_hold` |
| HDL-01 no initial attach hold, pure decision: only an unattached ordinary session keeps an early exit until its initial deadline | `session::tests::only_an_unattached_ordinary_session_holds_an_early_exit_until_its_initial_deadline` (unit) |
| HDL-01 survives the deliberate detach and the unexpected loss of its last attachment, then accepts another | `a_headless_session_survives_the_detach_and_the_loss_of_its_last_attachment` |
| HDL-01 pure decisions: `detached` from startup, the initial deadline ends nothing, the last attachment's detach or loss keeps it `detached`, Kill ends it | `control::tests::a_new_headless_session_is_detached_and_outlives_its_initial_deadline`, `control::tests::a_headless_session_survives_the_departure_of_its_last_attachment` (unit) |
| HDL-01 Kill with zero clients, confirmed after cleanup | `kill_ends_a_headless_session_with_zero_clients` |
| HDL-01 ordinary leave policy unchanged | `every_controlling_leave_hangs_up_then_kills_after_real_grace`, `an_unattached_session_ends_after_the_initial_deadline` |
| Startup policy: a missing or unknown `--lifetime` is an invalid invocation | `repeated_unknown_zero_pid_and_relative_options_are_invalid_invocations`; `args::tests::daemon_requires_a_known_lifetime` (unit) |
