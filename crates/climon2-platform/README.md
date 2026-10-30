# climon2-platform

Shared macOS seams (ADR 0003). Other targets fail inside each seam; neutral
contracts remain in `climon2-core`. No runtime dispatch or shell-specific logic.

## Runtime and peers

- `runtime::canonical_home(&Path) -> io::Result<PathBuf>` creates and
  canonicalizes the resolved absolute home **once per invocation**.
- `RuntimeDirectory::prepare(&canonical_home)` creates/checks the core-derived
  `/tmp/climon2-<euid>-<hash8>` directory. `path()` and
  `socket_path(&SessionId)` expose its paths; the socket limit is 103 bytes.
  The directory is never deleted.
- `runtime::private_directory(&Path)` creates mode 0700, then checks with
  `symlink_metadata`: real directory, effective UID owner, no group/other bits.
  Existing unsafe directories are refused, never repaired.
- `runtime::bind(&Path)` binds under umask 077. Bind before starting worker
  threads: umask is process-wide; the lock serializes only this helper's calls.
- `runtime::{SocketListener,SocketStream}` name the macOS transport types
  without exposing libc types in callers' signatures.
- `peer::accept(&SocketListener)` returns only authenticated streams.
  `peer::{uid,authenticate}` support already accepted streams; authentication
  must happen before hello and callers must drop rejected streams.

## Processes and startup

- `process::start_time(pid) -> io::Result<Option<u64>>` uses `proc_pidinfo`;
  timestamps are Unix-epoch microseconds. `is_same_process(pid, expected)`
  distinguishes PID reuse. Permission/inspection errors are **not** death.
- `process::{wait,try_wait}` own reaping by PID and return core `ExitStatus`
  without portable-pty's lossy signal mapping. Do not also wait through
  portable-pty or `std::process::Child`.
- `process::signal_group(group, Signal)` supports Hangup, Interrupt, Terminate,
  Kill, rejects 0, 1 (launchd) and out-of-range IDs, so `kill(0)`/`kill(-1)`
  are unreachable, and tolerates a nonexistent group.
- `process::Terminal::from_path(path)` identifies a terminal (a PTY slave) by
  its character device number; anything else is refused.
- `process::TerminalSession::new(leader, terminal)` is the session a command
  leads on its PTY. Candidates for a session kill come **only** from kernel
  lists: `proc_listpids(PROC_TTY_ONLY, terminal)` and, per recorded job group,
  `proc_listpids(PROC_PGRP_ONLY, group)`; every candidate must also satisfy
  `getsid == leader` and not be the caller, and is signalled through a
  non-zero-PID-only kill. Nothing ever enumerates all processes, so no single
  predicate change can reach processes off that PTY and its job groups.
  macOS detaches the terminal from the session when the leader exits, so
  `observe()` records the members' job groups while the leader lives.
  `kill()` observes once more, then SIGKILLs members in bounded rounds until
  none remain; it works after the leader was reaped. Processes that started
  their own session (setsid/daemonize) left the terminal and are never
  candidates. Callers reap only their own children.
- `process::hangup(&mut session, close_master)` observes job groups, closes
  every PTY-master handle through the caller's closure, waits the real five
  seconds, reaps an exited leader, kills the session, then returns exact status.
- `process::validate_cwd(&Path)` must run immediately before portable-pty spawn,
  which otherwise silently falls back to HOME. As with ordinary `chdir`, a
  concurrent directory removal can still race validation.
- `process::before_fork()` must run on the forking thread immediately before
  every fork (pre_exec spawns, portable-pty spawn). macOS runs libnotify's fork
  handler in the child, which traps (SIGILL) or hangs if another thread was
  initialising libnotify at the fork; this call waits that initialisation out.
  `daemon::spawn` calls it itself.
- `daemon::spawn(Command, configure)` calls `configure(&mut Command, fd: u32)`
  so the caller can append its readiness descriptor argument. It uses exec via
  `Command::pre_exec(setsid)`, inherited pipe, null stdin/stdout, and the caller's
  stderr configuration. No PTY-command environment is changed.
- `PendingDaemon::{id,wait_ready}` exposes the PID and enforces an absolute real
  five-second deadline. `wait_ready(&[metadata, socket])` returns
  `Readiness::{Ready,Failed { code, message }}`; failure/EOF/timeout kills and
  reaps the daemon, then removes the supplied paths in order. Dropping a pending
  startup also kills/reaps it; callers that want file cleanup must call
  `wait_ready`. Successful startup leaves the daemon running.
- Daemons exclusively adopt the inherited fd through
  `unsafe ReadinessWriter::from_inherited(fd)`, which marks it close-on-exec
  before PTY spawn. `send(&Readiness)` uses a bounded internal binary protocol:
  Ready byte, or Failed byte + big-endian code/UTF-8 length + message (4096-byte
  maximum, codes 125/126/127). Failure messages must contain no command content.

## Terminal

- `terminal::require_ttys()` checks stdin/stdout; `input()` duplicates stdin
  into an owned `File`; `size(&File)` uses TIOCGWINSZ and core's 80x24 fallback.
- `RawModeGuard::new(&File)` enters raw mode only after Attached.
  Drop and a chained panic hook restore termios only. One raw-mode
  owner is permitted in a process. No screen reset sequences are emitted.
- `Signals::client()` defers HUP/INT/TERM and coalesces WINCH.
  `termination_signal()` and `take_resize()` are polled by the event loop,
  which must reread the size and suppress unchanged resizes.
  `Signals::daemon()` ignores HUP/INT/PIPE and defers TERM. Registrations are
  unregistered on drop; no terminal operations run in signal handlers.

## Behavioral evidence

`tests/runtime.rs`: `issue18_private_directory_refuses_symlink_file_and_permissive_mode_without_repair`,
`issue18_home_is_created_then_canonicalized_and_socket_is_private_and_authenticated`.

`tests/process.rs`: `issue26_identity_uses_start_time_and_refuses_zero_or_pid_reuse`,
`issue19_waitpid_preserves_exit_and_real_signal_status`,
`issue19_hangup_escalates_group_after_real_five_seconds`,
`issue19_closing_real_controlling_pty_delivers_hangup`,
`issue19_signal_group_signals_the_group_and_tolerates_a_nonexistent_one`,
`issue19_kill_session_kills_every_job_but_not_new_sessions`,
`issue19_kill_session_spares_the_calling_process`,
`issue19_kill_session_only_signals_session_members_on_the_given_terminal`,
`issue19_kill_session_never_enumerates_beyond_terminal_and_observed_groups`,
`issue19_terminal_must_be_a_character_device`,
`issue15_bad_cwd_is_rejected_before_pty_can_fall_back`.

`tests/daemon.rs`: `issue19_daemon_exec_is_detached_and_readiness_preserves_launch_failure`,
`issue19_empty_readiness_kills_daemon_and_cleans_metadata_before_socket`,
`issue19_readiness_uses_real_five_second_absolute_deadline`.

`tests/terminal.rs`: `issue20_real_pty_raw_mode_restores_on_drop_and_panic_without_screen_sequences`,
`issue20_real_pty_size_and_zero_fallback`,
`issue20_signals_coalesce_resize_and_report_exit_signal`.

Tests create files only under project-local isolated directories. The two
ignored libtest functions are owned subprocess fixtures, executed by the
behavioral tests, not skipped requirements. Production `/tmp` naming and home
isolation are independently covered by core's layout tests.
