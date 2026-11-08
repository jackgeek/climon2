# climon2-monitor

The [monitor](../../GLOSSARY.md#sessions) executable, `climon2-monitor`, ships in
every version folder beside the CLI, client and daemon. Users reach it as
`climon2 monitor` or `climon2 mon`: the launcher runs the current version's CLI,
which execs its own version's monitor with no arguments or envelope. That
handoff between executables of one release is not frozen. The library boundary
is `run() -> Result<u32, Error>`; the thin main prints `climon2: error: …` for an
`Error` and exits 125.

This is the #74 walking skeleton of spec #73. It discovers no sessions and loads
no client libraries yet.

## Startup, before the host terminal changes

1. stdin and stdout must be TTYs (`climon2 monitor needs a terminal`).
2. The climon2 home is resolved, created and canonicalized exactly as the CLI
   does. The monitor then opens its own general log,
   `logs/general/monitor-<stamp>-<pid>.log` with header `role=monitor`, named
   apart from the CLI's general log that the same pid wrote before exec.
3. The executable home and version folder come from the monitor's canonical
   path through `ExecutableLayout::from_binary_path`, exactly as the CLI finds
   its own; outside an install layout this fails.
4. The existing `client` settings are read with their defaults (`attach`
   validation only), and a null `client.escape_prefix` is refused.

Every failure prints one escaped line, with the general-log suffix once that log
exists, and exits 125 before raw mode, the alternate screen or any OSC output.

## Host terminal and screen

- Signals (HUP, TERM, INT, WINCH) are caught first. Raw mode comes from the
  platform terminal seam's `RawModeGuard`, then the monitor enters its own
  alternate screen and enables bracketed paste, SGR mouse reporting (buttons
  and drags) and focus reporting. It never writes an OSC, so the host title is
  untouched.
- Ratatui 0.30 draws through `CrosstermBackend` over the monitor-owned stdout at
  a fixed viewport, so it never queries the host. The permanent top row reads
  `No session selected`; below it the centred empty state reads
  `No sessions to show.` and `Press <prefix> then x to exit.` with the configured
  prefix. SIGWINCH clears the screen and redraws it at the new size.
- Host input is read as raw bytes. Only prefix+x acts; every other byte,
  including a doubled (literal) prefix and Ctrl+C, is discarded because no
  target exists. Paste markers and mouse reports are not classified yet, so a
  bracketed paste containing prefix+x still exits; the input router (#109,
  #111) owns that classification.

## Exit

Prefix+x exits 0; catchable SIGINT, SIGTERM and SIGHUP take the same path with
status 128 + signal. The monitor drops its Ratatui terminal, writes the
documented normal-shell baseline (from `climon2-vt`) on its alternate screen and
leaves that screen last, so the shell's cursor returns. Termios is restored
afterwards, and only then does main print a diagnostic. A render failure
therefore still releases the host first and exits 125. The general log
records `monitor started version=<folder>` and `monitor stopped status=<n>`,
never terminal content or input.

With `test-hooks` only, `CLIMON2_MONITOR_FAULT=draw` makes drawing fail.
`cargo test -p climon2-monitor` builds the plain binary; workspace tests unify
`test-hooks` through `climon2-test-term` into the same sibling path.

## Specification mapping

Monitor-layer tests drive the installed product from `climon2-e2e`'s
`tests/monitor.rs`; this crate keeps the build proof and pure key tests.

| Scenario | Named test |
|---|---|
| ENT-01 (Monitor): `monitor` and `mon` open the same installed monitor; top row, empty state; split prefix+x exits 0 with the host released | `installed_monitor_and_mon_open_the_same_monitor` (e2e) |
| INP-01 (Monitor): configured prefix hint and exit; null take-control key accepted; Ctrl+C is not an exit | `installed_monitor_hints_and_exits_with_the_configured_prefix` (e2e) |
| INP-01 (Monitor): null escape prefix and invalid keys refused before terminal changes | `installed_monitor_refuses_a_null_escape_prefix_and_invalid_keys` (e2e) |
| Derived scenario: monitor argument refusal (2, usage) | `installed_monitor_arguments_are_a_usage_error` (e2e); core `issue29_exec_preserves_literal_os_arguments_and_list_rejects_usage_errors` |
| Derived scenario: monitor non-TTY stdin or stdout refusal | `installed_monitor_refuses_a_non_terminal_stdin_or_stdout` (e2e) |
| Spec #73 entry: a missing monitor names the version | `missing_installed_monitor_fails_naming_its_version` (e2e) |
| Spec #73 startup: the executable home is resolved as the CLI does | `a_monitor_outside_a_version_folder_refuses_to_start` (e2e); core `issue74_monitor_resolves_the_same_layout_as_the_cli` |
| EXT-03 (Monitor, empty monitor): SIGINT, SIGTERM and SIGHUP exit 130, 143 and 129 with baseline and exact termios | `signals_release_the_host_with_signal_style_status` (e2e) |
| EXT-03 (Fault, empty monitor): a render failure releases the host before its one diagnostic, 125 | `debug_hook_render_failure_restores_the_host_before_its_diagnostic` (e2e) |
| Spec #73 host terminal: SIGWINCH redraws without leftovers | `resizing_the_host_redraws_the_monitor_at_the_new_size` (e2e) |
| Spec #73 exit: content-free general log | `installed_monitor_logs_content_free_records_to_its_general_log` (e2e) |
| INP-01 (pure input-router supplement): prefix+x across reads; everything else discarded; only the configured prefix | `prefix_then_x_exits_across_read_boundaries`, `every_other_key_is_discarded`, `only_the_configured_prefix_begins_the_action` (`src/keys.rs`) |
| #74 risk proof: plain and hooked sibling binaries without nested builds | `cargo_builds_the_plain_or_hooked_monitor_binary_for_its_tests` (`tests/build.rs`); fixtures `workspace_binaries_contain_test_hooks`; xtask `release_check_builds_unhooked_binaries_in_a_private_target` |
| #74 release checks: version and architecture | xtask `version_check_rejects_a_missing_or_wrong_version_monitor`, `binary_check_rejects_a_single_architecture_monitor` |

Not yet claimed: EXT-03's root OSC 7501 `clear` (#118), application input such
as Ctrl+C (#109) and the universal archive's runtime on both architectures,
which only CI's `dist` and `archive` jobs build and execute.
