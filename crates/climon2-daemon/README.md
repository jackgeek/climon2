# climon2-daemon

Real session daemon library with a thin executable on macOS and Linux; no client
implementation or descriptor passing. The daemon owns the VT engine, bounded
scrollback, terminal restoration and live presentation.

## CLI-to-daemon API

```text
climon2-daemon --session-id GUID --home CANONICAL_ABSOLUTE_HOME \
  --socket ABSOLUTE_SOCKET --version SEMVER --cli-pid PID \
  --readiness-fd FD --rows ROWS --cols COLS \
  --scrollback-max-rows ROWS --cwd ABSOLUTE_CWD \
  [--log FILTER] -- COMMAND [ARG...]
```

Options are private, order-independent, unique and required except `--log`.
`--scrollback-max-rows` sets the VT engine's retained normal-screen history.
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
deadline and bounded frames. Up to 64 concurrent connections are admitted;
a further connection is closed unanswered. Each connection's outgoing queue is
capped at 16 MiB. Restoration prefixes are produced incrementally, whenever less
than 1 MiB is queued, while live output waits behind them; a controller whose
pending live output plus queued bytes would exceed 16 MiB, or any connection
that accepts none of its queued bytes for five seconds, is treated as lost. A
controller that does not send `Ready` within 30 s plus one second per 4 MiB of
restoration prefix is also lost. Probes remain serviceable throughout.

Every PTY byte goes through one ordered write queue (`PtyWrites`): terminal
replies and paste cleanup bytes ahead of queued input, and a partly written
unit always completes before the next starts, so sequences never interleave
across partial writes. Input is admitted only from the controller of the
current control period after its `Ready`; when admission ends, its unwritten
input is discarded and, if an application paste is open while mode 2004 is
set, closing-marker cleanup bytes are queued. A PTY write failure is an
authoritative session failure.

Control is a pure state machine (`control.rs`): each attach or take-control is
granted at once in a new period (attaches in one iteration grant only the
last), the previous controller gets its release then `Revoked`, and a departing
controller hands back to the most recently controlling attachment still
present. Only the controller's size drives the PTY and VT; the session geometry
is its host size clamped to 2..=500 columns and 1..=200 rows, and a host larger
than that gets Derived presentation. Resizes are coalesced to one per iteration.

Early exits and PTY output remain available for the first attachment: the exit
of a command that ends first is reported once that attachment's restoration is
confirmed, or after the five-second initial attach window. Final status comes
exclusively from shared exact `waitpid`, never portable-pty waiting. After
reaping, drain ends at EOF/EIO, 100 ms of actual PTY idle, or one second.
Metadata and the socket are then removed while connections stay open, the
controller gets its release bytes, every attachment gets `Exit`, and final
flushing is bounded to one further second. A receiver that
never reads cannot be guaranteed delivery after this bound.

The leave policy: while another attachment remains, the session continues;
after an accepted final detach it keeps running with no idle timeout; a final
unexpected departure, `Kill`, daemon TERM, an authoritative failure or no
attachment within five seconds of readiness begins shutdown. Shutdown closes
every master handle without writing to it, then runs a real five-second hang-up grace as a loop
deadline: probes see `terminating`, attaches are refused, repeated kills join
without restarting it, and the grace is not shortened if the command exits
meanwhile. Pending PTY writes and resizes are dropped after hang-up. When the
grace ends, the daemon SIGKILLs every process still in the command's terminal
session (all job-control process groups, per the #19/#27 owner decision) and
reaps the command. Only after metadata and socket removal does each attachment
get its release then `Exit` (or `Error` after a failure), and each kill
requester `Terminated`; a failed cleanup never confirms a kill. If the loop
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
  themselves, other framing failures say `invalid session protocol`.
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
These fault
branches and the extra generic fixture binary are absent from ordinary release
builds. The ignored libtest function is a subprocess fixture, not a skipped
requirement. Every controlled final path checks owned process/socket/metadata
cleanup; logs intentionally survive.
