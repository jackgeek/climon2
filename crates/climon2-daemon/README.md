# climon2-daemon

Real session daemon library with a thin executable. macOS only; no client
implementation, terminal emulator, scrollback or descriptor passing.

## CLI-to-daemon API

```text
climon2-daemon --session-id GUID --home CANONICAL_ABSOLUTE_HOME \
  --socket ABSOLUTE_SOCKET --version SEMVER --cli-pid PID \
  --readiness-fd FD --rows ROWS --cols COLS --cwd ABSOLUTE_CWD \
  [--log FILTER] -- COMMAND [ARG...]
```

Options are private, order-independent, unique and required except `--log`.
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

A short-lived authenticated `<socket stem>.exec` socket confirms the exec:
the helper sends an attempt marker, and successful exec closes its CLOEXEC
connection; failed exec sends 126/127. The helper adds no environment variables.
Both sockets are bound under the shared private-directory/umask seam before
starting stderr's capture thread. The startup socket is removed on every
controlled startup path. The outer CLI readiness deadline remains five seconds.

## Ownership and lifecycle

One event loop owns the master and keeps portable-pty's writer alive.
`Session::pump_output` is the sole PTY reader, through `Pty::read_output`;
there are no cloned readers or master handles outside the daemon.
**ADR 0003 / #27 review rule:** reviewers must preserve this single-reader
call graph. A textual ownership assertion would not prove it, so no vacuous
architecture test is claimed.

Authenticated connections use core's typed protocol messages and connection
states. Incremental nonblocking decoding enforces an absolute two-second hello
deadline and bounded frames. Up to 64 concurrent connections are admitted;
each has one bounded incoming frame and one pending 32 KiB Output frame
(plus a final small Exit). PTY input has one pending <=65,535-byte payload.
When output is blocked, the pump stops reading the PTY. Probes remain serviceable.

Early exits and PTY output remain available for the first Attach. Exactly one
connection controls the PTY, and Attached precedes its output. Final status
comes exclusively from shared exact `waitpid`, never portable-pty waiting.
After reaping, drain ends at EOF/EIO, 100 ms of actual PTY idle, or one second;
Exit is queued last and final socket flushing is bounded to one further second.
A receiver that never reads cannot be guaranteed delivery after this bound.

All controlling-client leave paths and daemon TERM use one leave policy:
close every master handle, real five-second grace, group KILL and reap.
HUP/INT/PIPE are ignored by the daemon. Natural exit also removes surviving
process-group members after drain. Cleanup always attempts metadata before
the session socket; runtime directories and logs remain.

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
- `runtime_argv_cwd_and_inherited_environment_are_exact_not_display_metadata`:
  literal/empty/option-looking/non-UTF8 argv, cwd, whole environment,
  absent/empty/non-UTF8 SHELL, explicit logging filter and diagnostic privacy.
- `exact_signal_exit_and_early_exit_are_delivered`,
  `final_drain_eof_or_deadline_is_bounded_and_leaves_no_descendants`:
  real signal mapping, early exit, final drain ordering and process groups.
- `concurrent_probes_absolute_hello_deadline_and_second_attach`,
  `malformed_unknown_oversized_and_out_of_state_messages_close_only_the_probe`:
  frozen hello, concurrent probes, single Attach and terminal protocol errors.
- `initial_fallback_and_resize_use_the_real_pty`,
  `raw_input_output_are_byte_transparent`: real dimensions and binary framing.
- `saturation_backpressures_pty_without_blocking_probes_or_losing_bytes`:
  an 8 MiB producer actually stalls, then delivers every byte.
- `every_controlling_leave_hangs_up_then_kills_after_real_grace`:
  EOF, Error, repeated hello/Attach, unknown message, TERM and stalled input all use
  the real five seconds and leave no command process.
- Startup failure/invalid invocation and daemon-ignore-signal tests cover
  safe launch diagnostics, refused permissions and ignored HUP/INT/PIPE.

On macOS, session-leader exit may revoke the PTY immediately, so the final
drain fixture permits EOF/EIO before the idle/cap clocks expire; it does not
claim those clocks necessarily expire in that OS scenario.

With `test-hooks` only, `CLIMON2_DAEMON_FAULT=panic|never-ready` injects named
pre-command startup faults. Their test checks capped panic capture, real
five-second startup timeout and absence of spawned command/files. These fault
branches and the extra generic fixture binary are absent from ordinary release
builds. The ignored libtest function is a subprocess fixture, not a skipped
requirement. Every controlled final path checks owned process/socket/metadata
cleanup; logs intentionally survive.
