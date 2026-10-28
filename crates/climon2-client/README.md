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

Connection, hello write, and complete hello reply share one real two-second
deadline. Peers must have the effective UID and hello must match session,
protocol, and metadata version. Attach precedes raw mode. The event loop has
one incremental incoming frame, one pending Input/Resize frame, and one pending
Output payload. Core validates framing; no payload exceeds 65,535 bytes.
Socket or terminal backpressure stops the corresponding producer.

SIGWINCH is coalesced and unchanged sizes are suppressed. HUP/TERM/INT close
the socket and return 128+signal. Stdin EOF/EIO silently returns 129. Ctrl-C,
Ctrl-Z, binary bytes and all terminal control sequences are forwarded unchanged.
Exit is read only after pending output drains. Termios and nonblocking flags
are restored on normal exit, errors, signals, and unwinding; no screen reset
sequences are emitted. Leave policy belongs exclusively to the daemon.

## Behavioral evidence

`tests/bridge.rs` drives the real binary on real PTYs with local protocol peers:

- `raw_bridge_preserves_bytes_and_restores_termios_after_final_output`
- `raw_client_signals_restore_termios_close_connection_and_return_signal_status`
- `termination_before_attached_does_not_enter_raw_mode_or_report_error`
- `protocol_error_lost_connection_and_panic_restore_without_terminal_sequences`
- `hello_rejects_mismatched_session_protocol_version_and_unexpected_frames`
- `hello_deadline_is_absolute_across_partial_reply`
- `fragmented_frames_and_coalesced_resize_preserve_the_stream`
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

The panic injection `CLIMON2_CLIENT_FAULT=panic` exists only with `test-hooks`.
The panic hook logs a static event, never payloads, argv or environment values.
Tests create isolated files under the project and never run nested Cargo builds.
Restoration assertions compare all configuration flags, control characters and
speeds; Darwin's kernel-managed transient `PENDIN` flag is not configuration.
