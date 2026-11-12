# climon2-monitor

The [monitor](../../GLOSSARY.md#sessions) executable, `climon2-monitor`, ships in
every version folder beside the CLI, client and daemon, on macOS and x86-64
GNU/Linux. Users reach it as `c2 monitor` or `c2 mon` (or the same through
`climon2`): the launcher runs the current version's CLI, which execs its own
version's monitor with no arguments or envelope. That handoff between
executables of one release is not frozen. The library boundary is
`run() -> Result<u32, Error>`; the thin main prints `climon2: error: …` for an
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
  alternate screen and enables bracketed paste, button-event mouse tracking
  with SGR encoding, and focus reporting. It never writes an OSC, so the host
  title is untouched.
- Ratatui 0.30 renders each frame into a buffer, and its crossterm backend
  paints every cell of it over the monitor-owned stdout. Nothing is erased
  first, the cursor stays hidden, and the host is never queried. The permanent
  top row reads `No session selected`; below it the centred empty state reads
  `No sessions to show.` and `Press <prefix> then x to exit.` with the configured
  prefix. SIGWINCH repaints the whole frame at the new size over the old one,
  so no blank screen or cursor flashes between them.
- Host input is read as raw bytes. Only a typed prefix+x acts; every other
  byte, including a doubled (literal) prefix and Ctrl+C, is discarded because
  no target exists. A bracketed paste is data, however the host splits its
  reads: it never exits, it discards a pending prefix, and it leaves none
  pending. Mouse and focus reports are not classified yet: they never exit, but
  they end a pending prefix. The input router classifies them (#109) and
  routes them to applications (#111).
- A failed wait for host input, other than one a signal interrupts, and the
  end of host input (a hung-up host terminal) end the monitor through the exit
  path below with status 125: `cannot wait for host input: …` or
  `the host terminal closed`.

## Exit

Prefix+x exits 0; catchable SIGINT, SIGTERM and SIGHUP take the same path with
status 128 + signal. The monitor writes the documented normal-shell baseline
(from `climon2-vt`) on its alternate screen and leaves that screen last, so the
shell's cursor returns. Termios is restored afterwards, and only then does main
print a diagnostic. A render or host-input failure therefore still releases the
host first and exits 125. The general log records
`monitor started version=<folder>`, then `monitor stopped status=<n>` or
`monitor failed error=<diagnostic>`, never terminal content or input.

With `test-hooks` only, `CLIMON2_MONITOR_FAULT=draw` makes drawing fail, and
`CLIMON2_MONITOR_FAULT=poll` makes the kernel reject the wait for host input by
giving it no descriptor table. `cargo xtask test` first runs
`cargo test -p climon2-monitor --test build` alone, which builds the plain
binary; the workspace tests that follow unify `test-hooks` through
`climon2-test-term` into the same sibling path.

## Specification mapping

Monitor-layer tests drive the installed product from `climon2-e2e`'s
`tests/monitor.rs`; this crate keeps the build proof and pure key tests.

| Scenario | Named test |
|---|---|
| ENT-01 (Monitor): `c2 monitor`, `c2 mon`, `climon2 monitor` and `climon2 mon` open the same installed monitor; top row, empty state, exact host modes; prefix+x exits 0 with the host released, the prefix read alone | `installed_monitor_and_mon_open_the_same_monitor` (e2e) |
| INP-01 (Monitor): configured prefix hint and exit; null take-control key accepted; once read, Ctrl+C and the default prefix+x are not exits | `installed_monitor_hints_and_exits_with_the_configured_prefix` (e2e) |
| INP-01 (Monitor): null escape prefix and invalid keys refused before terminal changes | `installed_monitor_refuses_a_null_escape_prefix_and_invalid_keys` (e2e) |
| INP-03 (Monitor, empty monitor): pastes never exit, never complete a pending prefix and leave none pending | `pastes_never_exit_or_leave_a_pending_prefix` (e2e) |
| Derived scenario: monitor argument refusal (2, usage) | `installed_monitor_arguments_are_a_usage_error` (e2e); core `issue74_monitor_and_mon_take_no_arguments` |
| Derived scenario: monitor non-TTY stdin or stdout refusal | `installed_monitor_refuses_a_non_terminal_stdin_or_stdout` (e2e) |
| Spec #73 entry: a missing monitor names the version | `missing_installed_monitor_fails_naming_its_version` (e2e) |
| Spec #73 startup: the executable home is resolved as the CLI does | `a_monitor_outside_a_version_folder_refuses_to_start` (e2e); core `issue74_monitor_resolves_the_same_layout_as_the_cli` |
| EXT-03 (Monitor, empty monitor): SIGINT, SIGTERM and SIGHUP exit 130, 143 and 129 with baseline and exact termios | `signals_release_the_host_with_signal_style_status` (e2e) |
| EXT-03 (Monitor, empty monitor): hung-up host input releases the host before its one diagnostic, logged, 125 | `a_hung_up_host_input_releases_the_host_before_its_diagnostic` (e2e) |
| EXT-03 (Fault, empty monitor): a render failure, or a failed wait for host input, releases the host before its one diagnostic, 125 | `debug_hook_render_failure_restores_the_host_before_its_diagnostic`, `debug_hook_input_wait_failure_restores_the_host_before_its_diagnostic` (e2e) |
| Spec #73 host terminal; TRM-04 (Monitor, empty monitor): SIGWINCH repaints without leftovers, erasing or showing the cursor | `resizing_the_host_redraws_the_monitor_at_the_new_size` (e2e) |
| Spec #73 exit: content-free general log | `installed_monitor_logs_content_free_records_to_its_general_log` (e2e) |
| INP-01 (pure input-router supplement): prefix+x across reads; everything else discarded; only the configured prefix | `prefix_then_x_exits_across_read_boundaries`, `every_other_key_is_discarded`, `only_the_configured_prefix_begins_the_action` (`src/keys.rs`) |
| INP-02/INP-03 (pure input-router supplement): pasted prefix+x is data; a paste leaves no pending prefix; markers split into single bytes; Ctrl+[ as prefix; only a complete start marker begins a paste | `a_pasted_prefix_and_x_never_exit`, `a_paste_leaves_no_pending_prefix`, `paste_markers_split_into_single_bytes_are_recognised`, `an_escape_prefix_still_recognises_pastes`, `only_a_complete_start_marker_begins_a_paste` (`src/keys.rs`) |
| #74 risk proof: plain and hooked sibling binaries without nested builds | `cargo_builds_the_plain_or_hooked_monitor_binary_for_its_tests` (`tests/build.rs`, plain and hooked runs); fixtures `workspace_binaries_contain_test_hooks`; xtask `release_check_builds_unhooked_binaries_in_a_private_target` |
| #74 release checks: version, architecture and test-hook marker | xtask `version_check_rejects_a_missing_or_wrong_version_monitor`; Linux `linux_packaging_rejects_a_non_x86_64_or_hooked_monitor`; macOS `binary_check_rejects_a_single_architecture_monitor`, `binary_check_rejects_a_hooked_monitor` |

Not yet claimed: EXT-03's root OSC 7501 `clear` (#118), application input such
as Ctrl+C (#109), mouse and focus report classification (#109) and their
routing to applications (#111), and the universal archive's runtime on both
architectures, which only CI's `dist` and `archive` jobs build and execute.
