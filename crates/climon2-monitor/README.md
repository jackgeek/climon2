# climon2-monitor

The [monitor](../../GLOSSARY.md#sessions) executable, `climon2-monitor`, ships in
every version folder beside the CLI, client and daemon, on macOS and x86-64
GNU/Linux. Users reach it as `c2 monitor` or `c2 mon` (or the same through
`climon2`): the launcher runs the current version's CLI, which execs its own
version's monitor with no arguments or envelope. That handoff between
executables of one release is not frozen. The library boundary is
`run() -> Result<u32, Error>`; the thin main prints `climon2: error: …` for an
`Error` and exits 125.

This is the #74 walking skeleton of spec #73, with its key introduction and
help (#105), session discovery, passive clients and the basic selector
(#106). Nothing is selected or acquired yet (#107, #108).

## Startup, before the host terminal changes

1. stdin and stdout must be TTYs (`climon2 monitor needs a terminal`).
2. The climon2 home is resolved, created and canonicalized exactly as the CLI
   does. The monitor then opens its own general log,
   `logs/general/monitor-<stamp>-<pid>.log` with header `role=monitor`, named
   apart from the CLI's general log that the same pid wrote before exec.
3. The executable home and version folder come from the monitor's canonical
   path through `ExecutableLayout::from_binary_path`, exactly as the CLI finds
   its own; outside an install layout this fails.
4. One configuration snapshot is read, `ConfigSource::monitor()`: the
   `client` bindings with their defaults, refusing a null
   `client.escape_prefix`, and the `monitor` section (`show_key_hints`, `bell`
   and `monitor.limits` with the ranges and rules the README lists). Nothing
   reads the file again, so editing it changes nothing until the next start.
5. The key introduction's preference is read from
   `<climon2 home>/monitor-preferences.json` (`HomeLayout::monitor_preferences`,
   schema `climon2_core::preferences`). A missing file saves nothing; an
   unreadable or malformed one means not suppressed and keeps its reason
   (`cannot read monitor-preferences.json: …` or `monitor-preferences.json is
   malformed: …`) for the screen. It never refuses the monitor.

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
  `No sessions to show.` and, unless `monitor.show_key_hints` is false, the key
  hint `Press <prefix> then x to exit.` with the configured prefix. Hiding the
  hint changes no binding. With it false, any problem with the key
  introduction's saved choice takes the hint's place: a diagnostic, not a
  hint. SIGWINCH repaints the whole frame at the new size over the old one,
  so no blank screen or cursor flashes between them.
- The key introduction (`introduction.rs`) opens at startup only when
  `monitor.show_key_hints` is true and **Don't show again** is not saved, and
  on prefix+? whatever either says (a no-op while open). It is a box titled
  `Keys`, one column of space inside its border, centred in the rows below the
  top row (an odd spare row falls above it, an odd spare column to its left):
  `<prefix> then x  Exit the monitor`, `… s  Open
  the session list`, `… ?  Show this dialog`, a blank line, `[ ] Don't show
  again (Space)` (`[x]` once saved), any problem with the saved choice, a
  blank line and `Continue (Enter)`. With key hints off, the empty state shows
  the same problem, until a save succeeds. The screen repaints only when its
  frame changes, and a resize always repaints it.
- One table of prefix actions (`keys.rs`, `BINDINGS`: key, action, label)
  completes the actions after the prefix and gives the key introduction its
  lines and the empty state's hint its key. Prefix+s opens the selector
  (below).
- Host input is read as raw bytes and classified byte by byte (`keys.rs`):
  typed prefix+x exits, typed prefix+? opens the key introduction, and every
  other typed byte outside a prefix, Ctrl+C included, goes to it while it is
  open, otherwise to the open selector, and is otherwise discarded, because
  nothing is selected. A doubled prefix
  is one literal prefix, which goes there too, so with `Ctrl+M` as the prefix
  Enter Enter continues. A prefix and the byte after it that completes no
  action reach nothing. The key introduction acts only on Enter (close) and
  Space (reverse the saved choice). A bracketed paste is data, however the
  host splits its reads: it never exits or reaches the key introduction, it
  discards a pending prefix, and it leaves none pending. Mouse and focus
  reports are not classified yet: they never exit and the key introduction
  ignores their bytes, but they end a pending prefix. The input router
  classifies them (#109) and routes them to applications (#111).
- Space saves the reversed choice (`preference.rs`). First it tries to remove
  the temporary files that saves interrupted before their rename left in the
  climon2 home: each `monitor-preferences.json.<pid>.tmp` whose pid no process
  runs as (`climon2_platform::process::start_time` finds none; a failed check
  keeps the file), and the one with its own pid, which only an earlier process
  with that pid can have left. Those of running processes stay: they may still
  be saving. The new `MonitorPreferences` then goes to its own
  `monitor-preferences.json.<pid>.tmp`, created afresh (`create_new`, mode
  0600, so private under any umask) and synced, then renamed over the file.
  The preference file itself is therefore always whole, and a save interrupted
  before its rename leaves its temporary file only until a later save in the
  same climon2 home, once its process has ended. A failed save removes what it
  wrote and shows `cannot save monitor-preferences.json: <error>` under the
  box, which keeps the saved choice. Nothing else writes the file.
- A failed wait for host input, other than one a signal interrupts, and the
  end of host input (a hung-up host terminal) end the monitor through the exit
  path below with status 125: `cannot wait for host input: …` or
  `the host terminal closed`.

## Sessions (#106)

- **Discovery** (`discovery.rs`) runs on its own thread through
  `climon2_discovery::Continuation`: the first pass at once, each later one a
  second after the previous one ended, so passes never overlap, bounded by
  `scheduling.probes` and `monitor.limits.discovery` (`pass_ms`,
  `batch_records`, `batch_bytes`, `metadata_file_bytes`, `metadata_depth`).
  Each batch reaches the UI thread as it is probed. It never deletes metadata.
  Invalid metadata is logged and adds no row; an incomplete pass shows
  `Discovery incomplete: <why>` under the body until a pass completes. The
  thread is never joined: a pass cannot be interrupted, and it changes
  nothing.
- **The registry** (`registry.rs`) is pure and the single owner of row state,
  keyed by full session ID: the discovery outcome and metadata, the connection
  status with the time the monitor entered it, the latest nonempty daemon
  title and whether the row is the monitor's own session
  (`CLIMON2_SESSION_ID`). A live session with no client and not the monitor's
  own gets exactly one client request. Discovery never removes a row or
  changes a client's connection; a stale session nobody knew adds no row. The
  status column is This monitor, Starting, Unavailable (uncertain, failed, or
  connected without `status-v1`), Disconnected (connection ended, or proven
  stale without a client), or empty for a connected row with `status-v1`,
  whose effective status arrives with #107. The label is the latest nonempty
  title, otherwise the command and arguments joined by spaces, with control
  characters escaped, never a session ID. Rows list newest created first, then
  ascending full session ID.
- **The client supervisor** (`supervisor.rs`) has its own thread. It loads
  each release once through `climon2_platform::library::Loader` on a worker
  thread (at most `scheduling.loads` at once; a session waiting on its release
  holds no start slot) and starts clients newest created first, then
  ascending session ID, at most `scheduling.starts` at once until each opens
  or fails. Each instance has an owner thread that creates it with the
  effective `monitor.limits` (`client_bytes` is all of `memory.client_bytes`:
  the monitor reserves no model capacity yet), both client keys and the log
  `logs/<session ID>/library-<stamp>-<pid>.log`, submits monitor Open (request
  1, geometry 0x0; `daemon-wire-v1` and `monitor-v1` required, `title-v1`,
  `status-v1`, `notification-v1`, `progress-v1` and `focus-v1` optional),
  publishes its handles, runs it, and destroys it once the supervisor has
  released them after terminal evidence. The supervisor polls the borrowed
  notification descriptors every 20 ms tick, drains each ready instance until
  it is empty, releases every delivery at once, and reports Open's outcome
  (with whether `status-v1` was selected), titles from Initial metadata and
  `CM1_TITLE`, and the end. A refused release, a failed create or Open, or a
  client that ends before opening makes the row Unavailable; one that ends
  after opening, Disconnected. Nothing is retried yet, and resource admission
  arrives with #120. The general log records `client library loaded` or
  `client library refused`, `client starting session=<id>`, `client opened`,
  `client open refused`, `client ended` and client diagnostics, content-free.
- **The selector** (`selector.rs`): prefix+s, unless the modal key
  introduction is open, overlays the rows below the top row, one cell in from
  each edge, with a captionless double border. Each row is its status, padded
  to the widest status shown, a space and its label, clipped by display cell
  without splitting a wide character; the highlighted row is reversed and
  kept in view. It opens on the first row; prefix+s while open does nothing.
  Typed Up and Down (`ESC [ A`/`B` or `ESC O A`/`B`) move the highlight
  without wrapping, and an Escape that ends a host read closes it; the input
  router (#109) replaces this per-read Escape rule. It never resizes an
  application or acquires anything. While rows exist and the selector is
  closed, the body reads `No session selected.` above the key hint.
- **Exit** first asks the supervisor to shut every instance down and keeps
  draining until each has ended, its handles are released and its owner is
  joined, for at most `deadlines.shutdown_ms`, then releases the host.

## Exit

Prefix+x exits 0; catchable SIGINT, SIGTERM and SIGHUP take the same path with
status 128 + signal. The monitor writes the documented normal-shell baseline
(from `climon2-vt`) on its alternate screen and leaves that screen last, so the
shell's cursor returns. Termios is restored afterwards, and only then does main
print a diagnostic. A render or host-input failure therefore still releases the
host first and exits 125. The general log records
`monitor started version=<folder>`, then
`monitor configuration effective=<JSON>` with every effective setting in the
file's shape, then `monitor preference unusable problem=<reason>` if the
preference file could not be used, `monitor preference saved
suppressed=<bool>` or `monitor preference not saved problem=<reason>` for each
save, and `monitor stopped status=<n>` or `monitor failed error=<diagnostic>`,
never terminal content or input.

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
| CFG-01 (Monitor): unreadable and malformed JSONC, null, wrong type, duplicate, overflow, out-of-range and inconsistent settings each refused with one diagnostic and 125 before terminal changes | `installed_monitor_refuses_every_invalid_configuration_before_the_terminal_changes` (e2e) |
| CFG-01 (Monitor): `show_key_hints` false hides the empty state's hint, prefix+x still exits; unknown and unrelated settings ignored | `installed_monitor_without_key_hints_shows_none_and_still_exits` (e2e) |
| CFG-01 (Monitor): no live reload: after an edit, a repaint keeps the started settings and the edited prefix does not act | `editing_the_configuration_while_the_monitor_runs_changes_nothing` (e2e) |
| CFG-01 (Monitor): every effective setting reported in the general log | `installed_monitor_logs_its_effective_configuration` (e2e) |
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
| DSC-03 (Monitor), CTL-01 (Monitor), COM-01 (Monitor), LIB-01 (Monitor, Linux maps), HDL-01 (Monitor rows): sessions created after an empty start appear unasked, one passive client and library log each, reused across passes; never attached or controlled; Unavailable without `status-v1`; titles and command fallback; exactly the version's `libclimon2_client.so` | `sessions_appear_as_they_are_created_each_with_one_passive_client` (e2e) |
| DSC-04 (Monitor, Fault): malformed and oversized metadata, mismatched hello, silent probe, inaccessible discovery, uncertain disappearance; no metadata deleted, no client torn down | `uncertain_discovery_is_explicit_and_never_deletes_metadata_or_clients` (e2e) |
| DSC-01 (Monitor, start order): bounded starts newest created first, ascending ID ties | `clients_start_one_at_a_time_newest_first_with_ascending_id_ties` (e2e) |
| Own-session scenario (confirmed 2026-11-08), DSC-06 (Monitor), HDL-01 (Monitor rows), CTL-01 (controlled elsewhere), TRM-06 (wide labels), SEL-01 (cell-safe clipping) | `the_own_session_is_this_monitor_and_nested_and_controlled_sessions_appear` (e2e) |
| SEL-01 (Monitor, basic cases): border, columns, highlight, Up/Down without wrap, Escape, no application resize or acquisition | `the_selector_lists_rows_moves_its_highlight_and_closes_without_resizing` (e2e) |
| DSC-01/DSC-03 (pure registry supplement): one client request per live session, newest-first order | `live_sessions_need_one_client_each_and_list_newest_first` (`src/registry.rs`) |
| Own-session scenario (pure supplement) | `the_own_session_is_this_monitor_and_never_needs_a_client` (`src/registry.rs`) |
| DSC-04 (pure supplement): uncertainty never removes rows or tears down clients | `uncertainty_never_removes_rows_or_tears_down_clients` (`src/registry.rs`) |
| COM-01 and not-connected labels (pure supplement) | `connection_statuses_are_labelled_as_specified` (`src/registry.rs`) |
| TRM-06 label cases (pure supplement): latest nonempty title, command fallback, literal, never IDs; cell-safe clipping | `labels_are_the_latest_nonempty_title_or_command_and_arguments_literally`, `clipping_is_display_cell_safe` (`src/registry.rs`) |
| SEL-01 basic cases (pure supplement) | `the_selector_draws_a_captionless_double_border_with_two_columns`, `up_and_down_move_without_wrapping_and_escape_closes` (`src/selector.rs`) |
| HLP-01 (Monitor): at every start with hints on and nothing saved, the introduction lists the actual prefix with x, s and ? and an unchecked box; Enter continues, closing saves nothing, prefix+x exits from it | `the_key_introduction_opens_at_every_start_until_dont_show_again_is_saved` (e2e) |
| HLP-01 (Monitor): hints off or the choice saved keep it closed; prefix+? opens it regardless, showing the saved choice; prefix+? while open is a no-op; a later version's file with unknown fields is honoured | `the_key_introduction_opens_on_prefix_question_mark_whatever_hints_and_the_saved_choice_say` (e2e) |
| HLP-01 (Monitor): Space saves and reverses the choice per climon2 home, exact version 1 bytes, mode 0600 under umask 000, by rename (a hard link keeps the old bytes), nothing left behind, a file already under the temporary name replaced; restarts and another home | `dont_show_again_is_saved_privately_and_atomically_per_home_and_reversible` (e2e) |
| HLP-01 (Monitor; derived scenario: a save interrupted before its rename): saving first removes the temporary files of processes no longer running and keeps those of running processes and any other file | `saving_removes_temporary_files_that_ended_processes_left_behind` (e2e) |
| HLP-01 (Monitor; derived scenario: unreadable or malformed preference file): not suppressed, the reason under the box and in the log, the file never rewritten implicitly | `an_unreadable_or_malformed_preference_file_shows_why_beside_the_introduction` (e2e) |
| HLP-01 (Monitor, Fault; derived scenario: unreadable or malformed preference file, key hints off): the introduction stays closed and the empty state shows the reason in place of the hint, a diagnostic; prefix+? shows it under the box; a failing save shows its failure in both places; a successful save over the file clears them | `without_key_hints_the_empty_state_shows_problems_with_the_saved_choice` (e2e) |
| HLP-01 (Fault): a save that cannot create its temporary file or replace the preference file is reported in the key introduction and logged, saves nothing and leaves nothing behind | `a_failing_save_is_reported_in_the_key_introduction_and_saves_nothing` (e2e) |
| HLP-02 (Monitor, over the empty monitor): the key introduction owns typed keys, prefixed keys, reports and pastes, none acts; Enter closes, prefix+? reopens, prefix+x exits | `the_key_introduction_is_modal_and_prefix_x_still_exits_from_it` (e2e) |
| HLP-02 (Monitor, over the empty monitor): with Ctrl+[ as the prefix, Escape cannot close the key introduction and prefix+x still exits | `with_escape_as_the_prefix_escape_cannot_close_the_key_introduction_and_prefix_x_still_exits` (e2e) |
| HLP-02 (Monitor, over the empty monitor): a doubled prefix reaches the key introduction as one literal prefix, so with Ctrl+M as the prefix Enter Enter continues; the prefix and a key it does not complete reach nothing; prefix+x still exits | `with_enter_as_the_prefix_a_doubled_enter_continues_from_the_key_introduction` (e2e) |
| INP-01 (pure input-router supplement): prefix+x across reads; no other key exits; only the configured prefix | `prefix_then_x_exits_across_read_boundaries`, `every_other_key_is_not_an_exit`, `only_the_configured_prefix_begins_the_action` (`src/keys.rs`) |
| HLP-01 (pure input-router supplement): prefix+? across reads, only after the configured prefix | `prefix_then_question_mark_opens_the_key_introduction_across_read_boundaries` (`src/keys.rs`) |
| HLP-02 (pure input-router supplement): typed keys outside a prefix and a doubled prefix's one literal prefix (Ctrl+M Enter Enter is one Enter) reach the key introduction; other prefixed and pasted ones never | `only_typed_keys_and_a_doubled_prefix_reach_the_key_introduction` (`src/keys.rs`) |
| INP-02/INP-03 (pure input-router supplement): pasted prefix+x is data; a paste leaves no pending prefix; markers split into single bytes; Ctrl+[ as prefix; only a complete start marker begins a paste | `a_pasted_prefix_and_x_never_exit`, `a_paste_leaves_no_pending_prefix`, `paste_markers_split_into_single_bytes_are_recognised`, `an_escape_prefix_still_recognises_pastes`, `only_a_complete_start_marker_begins_a_paste` (`src/keys.rs`) |
| #74 risk proof: plain and hooked sibling binaries without nested builds | `cargo_builds_the_plain_or_hooked_monitor_binary_for_its_tests` (`tests/build.rs`, plain and hooked runs); fixtures `workspace_binaries_contain_test_hooks`; xtask `release_check_builds_unhooked_binaries_in_a_private_target` |
| #74 release checks: version, architecture and test-hook marker | xtask `version_check_rejects_a_missing_or_wrong_version_monitor`; Linux `linux_packaging_rejects_a_non_x86_64_or_hooked_monitor`; macOS `binary_check_rejects_a_single_architecture_monitor`, `binary_check_rejects_a_hooked_monitor` |

Not yet claimed: EXT-03's root OSC 7501 `clear` (#118), application input such
as Ctrl+C (#109), mouse and focus report classification (#109) and their
routing to applications (#111), HLP-02 over the selector (#113), selector
search, footer and Enter (#108, #113), status order and initial selection
(#107), resource admission and fairness (#120), passive retry and
`discovery.stall_ms`, Escape closing the key introduction when Escape is not
the prefix (#109, whose input router tells a bare Escape from an escape
sequence), and the universal archive's runtime on both architectures, which
only CI's `dist` and `archive` jobs build and execute.
