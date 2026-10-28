# climon2-cli

CLI orchestration for `exec COMMAND [ARG...]` and `list [--json]`.

`run(arguments: impl IntoIterator<Item = OsString>) -> Result<u32, Error>`
excludes argv[0]. `Error` exposes `code`, `message`, and optional absolute `logs`.
A successful exec replaces the CLI process with the metadata version's client.
Usage errors are 2; climon2 failures are 125; daemon startup preserves 126/127.

Home is resolved, created and canonicalized once. Common dispatch opens the
general log and prunes old logs before either command. Exec checks stdin and
stdout TTYs before spawning, resolves canonical companions from its own canonical
version-layout path, generates one v4 GUID and socket, and switches to the
linked session log. Core projects the exact OS-string command environment and
provides depth/TERM/ordinal rules. The command's inherited `CLIMON2_HOME` and
`CLIMON2_LOG` are not rewritten.

Daemon startup uses the exact private interface documented in
`../climon2-daemon/README.md`; readiness fd and untouched command arguments are
appended inside the platform spawn callback. The real five-second readiness
deadline cleans metadata/socket on failure. After Ready, an armed guard sends
TERM and waits for daemon leave-policy cleanup if metadata/client handoff fails.
The exact frozen client invocation is derived from the published metadata;
client home/filter configuration travels only through its environment.

The parent-owned list interface is `list::run(home: &Path, json: bool) ->
io::Result<()>`, called after common logging/pruning.

## Behavioral evidence

`tests/exec.rs` drives copied real sibling binaries in a version install layout:

- `exec_rejects_non_tty_before_creating_a_session`
- `packaged_exec_preserves_final_command_status`
- `packaged_exec_preserves_runtime_argv_cwd_environment_and_initial_size`
- `nested_warning_uses_exact_ordinal_and_overflow_never_spawns`
- `actual_nested_bash_session_increments_depth_and_defaults_only_absent_term`
- `startup_failure_codes_and_signal_exit_are_preserved`
- `resizing_outer_terminal_resizes_the_running_command`
- `missing_client_after_ready_cleans_command_and_names_version_with_logs`

The generic terminal fixture is a test-hooks-only binary. Cross-package binaries
are found beside the integration-test executable; no test invokes Cargo.
The exact-environment check also launches the fixture directly, accounting only
for LLVM's independently observed pre-main marker while rejecting unknown keys.
Ordinary cases share one immutable copied install layout with isolated data
homes. Cases that remove or change executable permissions get private copies.
Leak checks await asynchronous daemon cleanup and inspect unique process markers,
metadata and sockets; fixture cleanup also terminates owned processes on panic.
