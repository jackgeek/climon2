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
never collide; a collision would be an ordinary I/O failure. Failures after the
log exists carry the ` (logs: <session-log directory>)` suffix.

The operation comes from the `CLIMON2_CLIENT_STARTUP` startup envelope (core
`startup_envelope`, at most 4096 bytes), which the CLI sets: `attach` with the
exact validated client settings, or `kill`. A present envelope is used exactly
or rejected. Without one, a directly started client reads and validates the
`client` section of `config.jsonc` and attaches.

Connection, hello write, and complete hello reply share one real two-second
deadline. Peers must have the effective UID and hello must match session,
protocol, metadata version, and the capability for the operation (`attach` or
`kill`); a missing capability fails before any request is sent.

**Kill mode** opens no terminal and no attachment: it sends `Kill` and waits for
`Terminated`. Confirmed termination returns 0 whatever the command's status; an
`Error`, a refusal or a lost connection is a failure, and HUP/TERM/INT while
waiting return 128+signal. The CLI bounds the whole operation.

**Attach mode**: raw mode begins only after the daemon accepts Attach. A
controller waits for Granted, writes the restoration and live backlog, drains
and discards input typed during restoration, then sends Ready; application
input is admitted only after that confirmation. A revoked client shows the
placeholder: the first line `This session is being viewed elsewhere.` and, when
a take-control key is configured, `Press <key> to take control`. It is redrawn
on resize. That key takes control; observer input is never sent. The escape
prefix (default Ctrl+]) followed by `d` requests an explicit detach from either
role; after `Detached` the client prints
`climon2: detached from session <id>; reattach with: climon2 attach <id>` on the
restored terminal and returns 0. Bracketed paste spanning a role change is
discarded through its closing marker, never replayed or read as keys.

The event loop has one incremental incoming frame, one pending Input/Resize
frame, and one pending Output payload. Core validates framing; no payload
exceeds 65,535 bytes. Socket or terminal backpressure stops the corresponding
producer, and no further frames are read while stdout has pending bytes. A
pending detach or a hung-up session has a bounded five-second closing deadline
(`detach not confirmed` or `session result not confirmed`), serviced even while
terminal output is blocked.

SIGWINCH is coalesced and unchanged sizes are suppressed. HUP/TERM/INT close
the socket and return 128+signal. Stdin EOF/EIO silently returns 129. Ctrl-C,
Ctrl-Z, binary bytes and all terminal control sequences are forwarded unchanged.
Exit is read only after pending output drains. Termios and nonblocking flags
are restored on normal exit, errors, signals, and unwinding. Leave policy
belongs exclusively to the daemon.

The daemon's release bytes return the terminal to its normal-shell baseline. A
shown placeholder is ended below itself (cursor to the last row, CR LF) on Exit,
Detached and Error, and with one non-blocking attempt on a termination signal.
Only when no daemon response arrives (unexpected connection loss, or the
five-second deadline expiring) does the client write its conservative reset:
synchronized update ended, `CSI 0 m`, cursor shown, mouse, focus and bracketed
paste off, application cursor keys and keypad off. It never leaves the
alternate screen blindly, so alternate-screen and other unknown state (for
example the title) cannot be restored without a daemon response. On expiry
pending output is discarded and the reset gets one non-blocking attempt.

## Client library (`libclimon2_client.dylib`)

This package's library target is also a cdylib (`crate-type = ["lib",
"cdylib"]`), so every release ships `libclimon2_client.dylib` in its version
folder, published atomically by the installer. It is an artifact, not another
crate. It exports only the frozen `climon2_client_get_interface` bootstrap of
[native client interface revision 1](../../docs/protocol/monitor-client/core-v1.md),
using the shared `#[repr(C)]` declarations in `climon2_core::native`; Rust
layouts, unwinding and allocators never cross it. The bootstrap and the table
entries do no fallible work and allocate nothing, so this build never reports
`CM1_INTERNAL_ERROR`; contained-panic reporting at every entry arrives with the
instance lifecycle (#85).

Requesting revision 1 returns one static, immutable description that lives
until process exit, and the same pointer on every call:

- `release`: the version folder name, compiled in (`CARGO_PKG_VERSION`). A copy
  placed in another version's folder still reports its own release.
- `capabilities`: empty (null, zero). No instance behaviour exists yet, so no
  token is advertised.
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

- `functions`: all eleven entries. Until #85 and #93 implement them, each
  fails with `CM1_UNSUPPORTED_OPERATION`, library origin and the stable error
  code `unsupported_operation`; failed calls zero their nonnull output structs,
  `notification` writes -1, and null outputs are never written.

Any other revision returns `CM1_UNSUPPORTED_REVISION` and writes a null
description; null output storage returns `CM1_INVALID_ARGUMENT`. The bootstrap
carries the `test_hook_marker!`, so hooked and release builds stay
distinguishable.

Loaders open the absolute `<executable home>/<version>/libclimon2_client.dylib`
with eager, local resolution; `current`, the working directory and
`DYLD_FALLBACK_LIBRARY_PATH` do not change which file loads. dyld does,
however, search `DYLD_LIBRARY_PATH` for the leaf name before an absolute
`dlopen` path, so the shared loader (#84) must confirm which image it loaded.
The library's install name is its build path; nothing links against it.

`cargo test` builds the hooked dylib beside the test executables in
`target/debug/deps/` (Cargo uplifts only binaries), where tests and the
installer's `Payload::from_siblings` find it without nested builds. Release
builds (`cargo build --release -p climon2-client`) uplift the unhooked dylib.

| Scenario | Named test |
|---|---|
| LIB-01 Library: absolute exact-release load, one shared load | `installed_library_describes_revision_one_with_its_version_identity` (`climon2-e2e/tests/library.rs`) |
| LIB-01 Library: `current`, working directory and fallback search paths; LIB-02 Library: misplaced copy reports its own release | `installed_library_loads_by_absolute_version_path_whatever_current_cwd_and_search_paths` (`climon2-e2e`) |
| LIB-02 Library: unsupported revisions and null output storage | `installed_library_rejects_other_revisions_without_a_description` (`climon2-e2e`) |
| DOC-01: unsupported table calls follow the frozen call rules | `installed_library_table_reports_every_instance_call_unsupported` (`climon2-e2e`) |
| DOC-01: exported symbol set is only the bootstrap | `installed_library_exports_only_the_frozen_bootstrap` (`climon2-e2e`) |
| DOC-01 layout case | `rust_native_declarations_match_the_published_header` (`climon2-core/tests/native_layout.rs`) |
| Hooked sibling dylib under `cargo test` | `workspace_client_library_is_a_hooked_sibling_of_the_test_executables` (`climon2-fixtures/tests/hooks.rs`) |
| Release checks: architecture, exact version, hook markers, exported symbols | `release_checks_*` (`xtask/src/library.rs`), `release_check_builds_unhooked_binaries_in_a_private_target` (`xtask/tests/release.rs`) |

## Behavioral evidence

`tests/bridge.rs` drives the real binary on real PTYs with local protocol peers:

- `raw_bridge_preserves_bytes_and_restores_termios_after_final_output`
- `raw_client_signals_restore_termios_close_connection_and_return_signal_status`
- `termination_before_attached_does_not_enter_raw_mode_or_report_error`
- `protocol_error_lost_connection_and_panic_restore_before_one_line_diagnosis`
- `an_error_while_observing_ends_the_placeholder_before_the_diagnostic`
- `a_signal_while_observing_ends_the_placeholder_and_keeps_the_signal_status`
- `a_lost_connection_while_observing_resets_conservatively_and_ends_the_placeholder`
- `hello_rejects_mismatched_session_protocol_version_and_unexpected_frames`
- `hello_deadline_is_absolute_across_partial_reply`
- `fragmented_frames_and_coalesced_resize_preserve_the_stream`
- `separate_stdin_and_stdout_descriptions_both_get_their_flags_back`
- `bounded_output_backpressure_drains_every_byte_before_exit`
- `terminal_eof_or_eio_is_silent_status_129_and_closes_connection`
- `terminal_eio_is_silent_status_129_and_restores_termios` (background job of a
  `/bin/sh` session leader with SIGTTIN/SIGTTOU ignored, so reads fail with EIO)
- `attach_refusal_or_unexpected_reply_fails_without_raw_mode`
- `hung_up_session_during_blocked_output_sleeps_then_delivers_everything` (bounds
  the client's CPU time, via `ps`, while stdout is blocked and the session is gone)
- `disconnect_before_attached_names_the_session`
- `invalid_length_prefix_closes_immediately_without_a_type_byte`
- `input_backpressure_is_bounded_and_lossless`
- `nonexistent_socket_and_bad_metadata_fail_without_touching_termios`
- `log_filter_reaches_client_through_environment_without_cli_flags`
- `concurrent_clients_on_one_session_each_write_their_own_log`
- `input_typed_before_restore_end_is_drained_and_never_replayed`
- `a_revoked_client_shows_the_placeholder_until_the_session_ends`
- `a_paste_spanning_role_changes_is_never_replayed_or_read_as_keys`
- `unconfirmed_detach_exits_after_five_seconds_and_restores_terminal`
- `lost_session_while_stdout_is_blocked_ends_within_the_closing_deadline`
- `pending_detach_while_stdout_is_blocked_ends_within_the_bound`

`tests/termination.rs` covers kill mode:
`confirmed_termination_is_silent_success_whatever_the_command_status`,
`a_daemon_error_or_lost_connection_is_a_failed_kill`,
`kill_needs_the_kill_capability_and_never_sends_a_request_without_it`,
`a_kill_capable_session_without_attach_refuses_attach_mode` and
`a_signal_while_waiting_for_confirmation_is_the_signal_status`.
`tests/scaffold.rs` has `missing_metadata_is_a_one_line_failure_without_stdout`.

The only fault injection, `CLIMON2_CLIENT_FAULT=panic` (with `test-hooks`
only), panics after entering raw mode. The client has no attach-hold hook (the
plan's `hold-attach` was not implemented); the CLI's `CLIMON2_CLI_HOLD` hook
holds before the client starts instead.
The panic hook logs a static event, never payloads, argv or environment values.
Tests create isolated files under the project and never run nested Cargo builds.
Restoration assertions compare all configuration flags, control characters and
speeds; Darwin's kernel-managed transient `PENDIN` flag is not configuration.
They also compare the terminal's file-status flags (`F_GETFL`, e.g. `O_NONBLOCK`)
for stdin's and stdout's open-file descriptions; Darwin's kernel-private
`FWASWRITTEN` bit (set by any write, not settable) is not configuration.
