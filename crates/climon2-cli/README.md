# climon2-cli

CLI orchestration for `exec COMMAND [ARG...]`, `attach SESSION-ID`,
`kill SESSION-ID` and `list [--json]`. The usage string is
`usage: c2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json]`.

`run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error>`
excludes argv[0]. `Error` exposes `code`, `message`, and optional absolute `logs`.
A successful exec or attach replaces the CLI process with the metadata
version's client. Usage errors (including a malformed session selector) are 2;
climon2 failures are 125; daemon startup preserves 126/127.

Home is resolved, created and canonicalized once. Common dispatch opens the
general log and prunes old logs before any command. Exec checks stdin and
stdout TTYs before spawning, then reads and validates the whole
`config.jsonc` before any session side effect. It resolves canonical companions
from its own canonical version-layout path, generates one v4 GUID and socket,
and switches to the linked session log. Core projects the exact OS-string
command environment and provides depth/ordinal rules; `TERM=xterm-256color` and
`COLORTERM=truecolor` are always set. The command's inherited `CLIMON2_HOME`
and `CLIMON2_LOG` are not rewritten.

Daemon startup uses the exact private interface documented in
`../climon2-daemon/README.md`, including `--scrollback-max-rows` from the
validated configuration; readiness fd and untouched command arguments are
appended inside the platform spawn callback. The real five-second readiness
deadline cleans metadata/socket on failure. After Ready, an armed guard sends
TERM to the daemon, waits up to six seconds, then KILLs it if metadata/client
handoff fails. The exact frozen client invocation is derived from the published
metadata; client home/filter configuration travels only through its
environment, and the validated client settings through the
`CLIMON2_CLIENT_STARTUP` envelope.

The parent-owned list interface is `list::run(home: &Path, json: bool) ->
io::Result<()>`, called after common logging/pruning. Columns are SESSION,
CREATED, VERSION, STATE and CLIENTS (from the daemon's hello extensions,
`unknown` when not advertised or malformed, which is logged), COMMAND and ARGS;
`--json` adds a `state` object or `null`.

Attach discovers live sessions and resolves an exact ID or unique prefix (1 to
32 hex digits; none or several matches is 125). It then requires the daemon's
`attach` and `client-startup-v1` capabilities, refuses self-attachment
(`cannot attach to the current session`) and non-TTY use
(`climon2 attach needs a terminal`), validates only the `client` configuration,
and replaces the CLI with the version-matched client carrying an `attach`
envelope, overwriting any inherited one.

Kill never reads configuration. It selects one live session the same way,
requires the `kill` and `client-startup-v1` capabilities, and spawns that
session's version of the client with a `kill` envelope and null stdin/stdout.
One ten-second deadline runs from selection to confirmation; the client's own
status is returned (0 after confirmed termination). On expiry the CLI kills and
reaps only its own helper and fails with 125
`termination of session <id> was not confirmed`.

With `test-hooks` only, `CLIMON2_CLI_HOLD=<path>` makes exec, after the daemon
is ready and immediately before the client starts, create `<path>` with its
extension replaced by `.held`, then wait, without a bound, until `<path>`
exists. It lets a test change the configuration between session start and
client start.

## Behavioral evidence

`tests/exec.rs` drives copied real sibling binaries in a version install layout:

- `exec_rejects_non_tty_before_creating_a_session`,
  `exec_requires_both_stdin_and_stdout_ttys_but_not_stderr`
- `invalid_exec_config_is_rejected_before_session_side_effects`
- `packaged_exec_preserves_final_command_status`
- `packaged_exec_preserves_runtime_argv_cwd_environment_and_initial_size`
- `exec_restores_a_waiting_alternate_screen_without_application_redraw`
- `packaged_client_can_detach_and_reattach_to_a_live_session`
- `nested_warning_uses_exact_ordinal_and_overflow_never_spawns`
- `actual_nested_bash_session_increments_depth_and_sees_the_fixed_terminal_identity`
- `startup_failure_codes_and_signal_exit_are_preserved`
- `resizing_outer_terminal_resizes_the_running_command`
- `missing_client_after_ready_cleans_command_and_names_version_with_logs`,
  `unexecutable_client_after_ready_cleans_daemon_and_command`,
  `never_ready_daemon_is_killed_after_real_five_seconds`
- `kill_terminates_a_detached_session_silently_after_cleanup`,
  `kill_while_attached_gives_the_client_the_signal_status`,
  `an_unconfirmed_kill_fails_at_the_deadline_and_reaps_only_its_helper`
- `attach_overwrites_an_inherited_kill_envelope`
- `a_config_edit_before_the_client_starts_changes_neither_client_nor_daemon`
  (uses `CLIMON2_CLI_HOLD`)

`tests/attach.rs` covers selection and capability checks for attach and kill:
`a_selector_must_match_exactly_one_live_session`,
`a_session_without_attach_capabilities_is_refused`,
`a_session_cannot_attach_to_itself`, `attach_needs_a_terminal`,
`a_session_of_an_uninstalled_version_names_that_version`,
`attach_validates_only_the_client_configuration`,
`kill_selects_exactly_one_live_session`,
`kill_refuses_an_older_session_without_capabilities` and
`kill_of_an_uninstalled_version_names_that_version_and_ignores_config`.

`tests/list.rs` covers discovery, stale cleanup and presentation, including
`list_shows_version_state_and_clients_only_when_advertised`,
`list_json_adds_the_state_object_or_null`,
`malformed_hello_extensions_are_logged_and_the_session_stays_live`,
`list_quotes_only_the_arguments_that_need_it` and
`list_args_that_exactly_fit_are_whole_and_one_cell_less_is_truncated`.

Probe workers retain the caller's logging subscriber, including warnings when
Linux's stable daemon identity is unavailable. An already absent stale socket
is successful cleanup, not a warning. A trickled then stalled hello reply
still observes the single two-second discovery deadline.

The generic terminal fixture is a test-hooks-only binary. Cross-package binaries
are found beside the integration-test executable; no test invokes Cargo.
The exact-environment check also launches the fixture directly, accounting only
for LLVM's independently observed pre-main marker while rejecting unknown keys.
Ordinary cases share one immutable copied install layout with isolated data
homes. Cases that remove or change executable permissions get private copies.
Leak checks await asynchronous daemon cleanup and inspect unique process markers,
metadata and sockets; fixture cleanup also terminates owned processes on panic.
Typed input follows [`climon2-e2e`'s rule](../climon2-e2e/README.md): a client
discards keys typed while it restores, so before input that must reach the session
`type_until` retypes a harmless line until its echo appears, and the input is then
sent once. Where typing would only end the command, the command waits instead for
an acknowledgement file the test creates (the fixture's `FIXTURE_REPORT_RELEASE`);
the zero-size case's fixture prints the size and exits, so that test types nothing.
