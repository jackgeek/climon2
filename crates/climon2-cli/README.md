# climon2-cli

CLI orchestration for `exec [--headless] COMMAND [ARG...]`, `attach SESSION-ID`,
`kill SESSION-ID`, `list [--json]` and `monitor` (alias `mon`). The usage string is
`usage: c2 exec [--headless] <command> [args...] | attach <session-id> | kill <session-id> | list [--json] | monitor | mon`.

`run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error>`
excludes argv[0]. `Error` exposes `code`, `message`, and optional absolute `logs`.
A successful ordinary exec or attach replaces the CLI process with the metadata
version's client; a successful headless exec returns 0; a successful monitor
replaces it with the CLI's own version's `climon2-monitor`. Usage errors
(including a malformed session selector) are 2; climon2 failures are 125; daemon
startup preserves 126/127.

Home is resolved, created and canonicalized once. Common dispatch opens the
general log and prunes old logs before any command. Exec reads and validates the
whole `config.jsonc`, its `monitor` section included, then, unless headless,
checks stdin and stdout TTYs, all before any session side effect. It resolves
canonical companions
from its own canonical version-layout path, generates one v4 GUID and socket,
and switches to the linked session log. Core projects the exact OS-string
command environment and provides depth/ordinal rules; `TERM=xterm-256color` and
`COLORTERM=truecolor` are always set. The command's inherited `CLIMON2_HOME`
and `CLIMON2_LOG` are not rewritten. The initial size is the stdin terminal's,
or 80x24 when stdin is not a terminal (only possible for headless exec).

Daemon startup uses the exact private interface documented in
`../climon2-daemon/README.md`, including `--scrollback-max-rows` from the
validated configuration and `--lifetime ordinary|headless`; readiness fd and
untouched command arguments are appended inside the platform spawn callback. The
real five-second readiness deadline cleans metadata/socket on failure. After
Ready, an armed guard sends TERM to the daemon, waits up to six seconds, then
KILLs it if metadata/client handoff fails. Headless exec instead disarms the
guard, leaving the session running, writes the session ID and a newline to
stdout and exits 0; when stdout cannot be written, it only logs that and still
exits 0. The exact frozen client invocation is derived
from the published metadata; client home/filter configuration travels only
through its environment, and the validated client settings through the
`CLIMON2_CLIENT_STARTUP` envelope.

`list`, `attach` and `kill` share one discovery pass from
[`climon2-discovery`](../climon2-discovery/README.md), through
`list::discover_live(home)`. It reads whole metadata files
(`MetadataLimits::UNBOUNDED`), removes stale metadata
(`StaleMetadata::Remove`), and keeps only live sessions, newest created first
with ascending session ID breaking ties. `list::LiveSession` is the library's
type. The library logs nothing: the CLI logs each pass's diagnostics on the
calling thread, so the general log records them under the CLI's own target, as
before the extraction. They are `invalid session metadata` (warn) once per
invalid entry; then one per probe that proves nothing or fails its cleanup:
`session probe did not match` (debug, `responded=`), `session connection
failed` (debug, `errno=`), `refused connection belongs to a live daemon`
(debug), `daemon identity is uncertain` (warn, `errno=`; Linux also warns when
the daemon's stable identity is unavailable) and `stale session cleanup failed`
(warn, `errno=`); then `malformed hello extensions` (warn, `session=`) for each
live session whose capabilities or state are malformed.

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

Monitor takes no arguments (any is a usage error, 2). After the common general
log is opened, it finds `<executable home>/<own version>/climon2-monitor` with
the same layout code used for the client and daemon, then replaces the CLI with
it, passing no arguments or envelope: the handoff is between executables of one
release and is not frozen. If that fails it reports
`cannot start the climon2 <version> monitor: …` with 125. The monitor resolves
the home and reads its configuration itself.

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

Headless exec (`exec --headless`, HDL-01 and the confirmed headless scenarios)
is proven through the installed launcher in `../climon2-e2e/tests/headless.rs`;
its scenario table is in that crate's README.

`tests/attach.rs` covers selection and capability checks for attach and kill:
`a_selector_must_match_exactly_one_live_session`,
`a_session_without_attach_capabilities_is_refused`,
`a_session_cannot_attach_to_itself`, `attach_needs_a_terminal`,
`a_session_of_an_uninstalled_version_names_that_version`,
`attach_validates_only_the_client_configuration`,
`kill_selects_exactly_one_live_session`,
`kill_refuses_an_older_session_without_capabilities` and
`kill_of_an_uninstalled_version_names_that_version_and_ignores_config`.

The monitor acceptance-matrix cases proven here are listed under
[Specification mapping](#specification-mapping).

The `monitor`/`mon` hand-off is proven through the installed launcher, as `c2`
and `climon2`, by `climon2-e2e` `tests/monitor.rs`:
`installed_monitor_and_mon_open_the_same_monitor` (ENT-01),
`installed_monitor_arguments_are_a_usage_error` (derived argument refusal) and
`missing_installed_monitor_fails_naming_its_version`.
`tests/scaffold.rs` checks the exact usage line.

`tests/list.rs` covers discovery, stale cleanup and presentation, including
`list_shows_version_state_and_clients_only_when_advertised`,
`list_json_adds_the_state_object_or_null`,
`malformed_hello_extensions_are_logged_and_the_session_stays_live`,
`list_quotes_only_the_arguments_that_need_it`,
`list_args_that_exactly_fit_are_whole_and_one_cell_less_is_truncated` and
`every_probe_diagnostic_is_logged_once_with_its_errno` (DSC-06: the general
log keeps every probe diagnostic after the extraction).

An already absent stale socket is successful cleanup, not a warning. A
trickled then stalled hello reply still observes the single two-second
discovery deadline.

The list, attach and kill tests above run unchanged after discovery moved into
`climon2-discovery`. Those that run a discovery pass are the CLI part of DSC-06:
their doc comments cite it, and the specification mapping below names them. The
DSC-06 Contract case and the library's own cases are in that crate's tests.

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

## Specification mapping

| Scenario | Named test |
|---|---|
| CFG-01 (exec case): exec refuses invalid `monitor` settings, as it does an invalid `scrollback`, before a session exists | `invalid_exec_config_is_rejected_before_session_side_effects` (`tests/exec.rs`) |
| CFG-01 (attach case): attach ignores an invalid `monitor` section, validating only `client` | `attach_validates_only_the_client_configuration` (`tests/attach.rs`) |
| DSC-06 (CLI part): `list` discovery, stale cleanup, probing, ordering and output are unchanged after discovery moved into `climon2-discovery` | `tests/list.rs`: `malformed_metadata_is_hidden_warned_and_never_deleted`, `stale_cleanup_requires_connection_refusal_and_proven_dead_identity`, `uncertain_linux_daemon_identity_is_warned_and_metadata_is_retained` (Linux), `already_missing_stale_socket_does_not_report_cleanup_failure`, `gone_pid_is_stale_but_wrong_or_malformed_hello_never_is`, `list_args_fit_the_real_stdout_terminal_width`, `live_daemon_list_preserves_the_controlling_session_and_json_arguments`, `failed_hello_retains_artifacts_under_a_single_two_second_deadline`, `stalled_partial_hello_does_not_block_past_the_discovery_deadline`, `stale_metadata_cannot_unlink_a_regular_file_as_a_socket`, `frozen_probes_are_concurrent_and_sort_newest_first`, `malformed_hello_extensions_are_logged_and_the_session_stays_live` |
| DSC-06 (CLI part): `attach` and `kill` select exactly one live session and refuse the sessions they cannot serve, as before | `tests/attach.rs`: `a_selector_must_match_exactly_one_live_session`, `a_session_without_attach_capabilities_is_refused`, `a_session_cannot_attach_to_itself`, `attach_needs_a_terminal`, `a_session_of_an_uninstalled_version_names_that_version`, `kill_selects_exactly_one_live_session`, `kill_refuses_an_older_session_without_capabilities`, `kill_of_an_uninstalled_version_names_that_version_and_ignores_config`, `attach_validates_only_the_client_configuration` |
| DSC-06 (CLI part): `attach` and `kill` of real sessions, with their statuses and cleanup, are unchanged | `tests/exec.rs`: `packaged_client_can_detach_and_reattach_to_a_live_session`, `kill_terminates_a_detached_session_silently_after_cleanup`, `kill_while_attached_gives_the_client_the_signal_status`, `attach_overwrites_an_inherited_kill_envelope`, `an_unconfirmed_kill_fails_at_the_deadline_and_reaps_only_its_helper` |
| DSC-06 (CLI part): the general log keeps each probe diagnostic, once per session, with its level, target and errno | `tests/list.rs`: `every_probe_diagnostic_is_logged_once_with_its_errno` |
