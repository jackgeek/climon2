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
(#106), and row statuses, final outcomes and status order (#107). Nothing is
selected or acquired yet: initial selection (#266) and #108's Enter.

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
   Its limits must then hold one session's mandatory allocation at the
   largest geometry a session can have, `climon2-vt`'s `Geometry::LARGEST`
   (500x200), by the parts its `Model::mandatory` charges a blank model
   keeping `history.max_rows` (#167): `memory.total_bytes` the monitor's one
   shared parser (`Parser::BYTES`, #166), the current model and the history
   chunk holding its recent page together, then `client_bytes` the model and
   chunk, and `history_bytes_total`, then `history_bytes_per_session`, that
   chunk (`Limits::hold`). That admits the largest session but not a
   restoration staged beside it, at least 33,932,174 bytes in a client's
   budget (36,029,326 in the total) at 500x200, which follows every
   monitor-mode geometry change (#89).
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
  `the host terminal closed`. A read failing with EIO is the end of host input
  too: Linux fails reads with it while it is still hanging a terminal up.

## Sessions (#106)

- **Discovery** (`discovery.rs`) runs on its own thread through
  `climon2_discovery::Continuation`: the first pass at once, each later one a
  second after the previous one ended, so passes never overlap, bounded by
  `scheduling.probes` and `monitor.limits.discovery` (`pass_ms`,
  `batch_records`, `batch_bytes`, `metadata_file_bytes`, `metadata_depth`;
  `discovery.stall_ms` is not applied, #120). The pass deadline runs from the
  pass's start once it has reached an entry or started a probe. Opening the
  selector requests nothing of discovery, so it can neither start a pass
  early nor overlap one. Each batch reaches the UI thread as it is probed. It never deletes metadata.
  Invalid metadata is logged and adds no row; an incomplete pass shows
  `Discovery incomplete: <why>` under the body until a pass completes. The
  thread is never joined: a pass cannot be interrupted, and it changes
  nothing.
- **The registry** (`registry.rs`) is pure and the single owner of row state,
  keyed by full session ID: the discovery outcome and metadata, the connection
  status with the time the monitor entered it, the latest nonempty daemon
  title and whether the row is the monitor's own session
  (`CLIMON2_SESSION_ID`), and from its client the latest daemon status
  (`status-v1`: state and the daemon's stored change time) and the final
  outcome (exit code or signal), with the time it sorts by: the daemon's
  terminal status change time when it reported one, otherwise when the
  monitor learned it. A final outcome survives disconnection and is never
  replaced; a daemon status outlives its connection only as stale detail,
  which neither labels nor sorts the row. A live session with no client and not the monitor's
  own gets exactly one client request. Discovery never changes a client's
  connection. A session proven gone (stale) drops its row unless a client
  was ever connected to it, and a stale session nobody knew adds no row; a
  row that was connected stays, with its history (owner decision on #106).
  The status column is the effective status, Needs attention,
  Acknowledged, Running or Idle; Succeeded for exit zero; Failed with the
  exact code or signal, `Failed (exit 143)` or `Failed (signal 15)`; or, for
  a row with no current status and no final outcome, This monitor,
  Starting (also a client opened with `status-v1` until its Initial
  metadata), Unavailable (uncertain, failed, or connected without
  `status-v1`) or Disconnected (its connection ended). The library rejects
  a final status without its result (`invalid_record`, owner decision on
  #95), so no row shows a failure whose result is unavailable. The label is
  the latest nonempty title, otherwise the command and arguments joined by
  spaces, with control characters escaped, never a session ID. Rows list by
  group (needs attention, acknowledged, running, idle, failed, completed,
  then every not-connected row), within a group newest time first (the
  daemon's change time; for a final row its time above; for a
  not-connected row when the monitor recorded it entering its condition),
  then ascending full session ID. Only the daemon's effective change time
  moves a row, never when the monitor heard it.
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
  released them. The supervisor waits up to a 20 ms tick on the borrowed
  notification descriptors, then drains every live instance until it is
  empty (so one whose descriptor lookup failed still ends), releases every
  delivery at once (`supervisor/client.rs`), and reports Open's outcome
  (with whether `status-v1` was selected), titles from Initial metadata and
  `CM1_TITLE`, the status or final outcome from Initial metadata,
  `CM1_STATUS` and `CM1_FINAL`, and the end. A refused release (its loader `Failure` kept for
  #116's retry), a failed create, Open, notification lookup or drain, a run
  that returns without terminal evidence, or a client that ends before
  opening makes the row Unavailable; one that ends after opening,
  Disconnected. A failed instance is asked to stop and drained until its
  stream is over; its handles are then released, its start slot is free
  (only instances still creating or opening hold one), and its owner
  destroys it. Nothing is retried yet. Starts are newest created first only
  among the sessions waiting together: fairness across discovery batches
  and resource admission arrive with #120. The general log records `client
  library loaded` or `client library refused`, `client starting
  session=<id>`, `client published`, `client failed`, `client stopped`,
  `client ended` and client diagnostics, content-free.
- **The selector** (`selector.rs`): prefix+s, unless the modal key
  introduction is open, overlays the rows below the top row, one cell in from
  each edge, with a captionless double border. Each row is its status, padded
  to the widest status shown, a space and its label, clipped by display cell
  without splitting a wide character; the highlighted row is reversed and
  kept in view. It opens on the first row; prefix+s while open does nothing.
  Typed Up and Down (`ESC [ A`/`B` or `ESC O A`/`B`, decoded by one rule
  table) move the highlight without wrapping; while nothing is highlighted
  (it opened with no rows, or the highlighted row was dropped), either
  highlights the first row. An Escape that ends a host read closes it. Known gap (owner decision on #106): an Escape followed by
  other bytes in the same read does not close it; the input router closes
  this gap (#109, INP-02). It never resizes an
  application or acquires anything. Rows that reorder while it is open keep
  the highlight on the same row. While rows exist and the selector is
  closed, the body reads `No session selected.` above the key hint.
- **Exit** first asks the supervisor to shut every instance down and keeps
  draining until each has ended, its handles are released and its owner is
  joined, for at most `deadlines.shutdown_ms` (and one tick), then releases
  the host. An instance unconfirmed by then is logged (`client shutdown
  unconfirmed at its deadline`) and left to process exit: its owner never
  destroys it while its handles may be unreleased. The fail-stop that
  should follow arrives with #118.

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
| CFG-01 (Monitor): limits one byte short of the monitor's parser and one 500x200 session's current model and recent page (total), of the model and page (client budget) or of its recent page (history total, then per-session history budget) each refused with one diagnostic naming that limit and 125 before terminal changes; at those limits the monitor opens and prefix+x exits 0 (#167) | `installed_monitor_refuses_limits_that_cannot_hold_the_largest_session_model` (e2e) |
| CFG-01 (Monitor): `show_key_hints` false hides the empty state's hint, prefix+x still exits; unknown and unrelated settings ignored | `installed_monitor_without_key_hints_shows_none_and_still_exits` (e2e) |
| CFG-01 (Monitor): no live reload: after an edit, a repaint keeps the started settings and the edited prefix does not act | `editing_the_configuration_while_the_monitor_runs_changes_nothing` (e2e) |
| CFG-01 (Monitor): every effective setting reported in the general log | `installed_monitor_logs_its_effective_configuration` (e2e) |
| INP-03 (Monitor, empty monitor): pastes never exit, never complete a pending prefix and leave none pending | `pastes_never_exit_or_leave_a_pending_prefix` (e2e) |
| Derived scenario: monitor argument refusal (2, usage) | `installed_monitor_arguments_are_a_usage_error` (e2e); core `issue74_monitor_and_mon_take_no_arguments` |
| Derived scenario: monitor non-TTY stdin or stdout refusal | `installed_monitor_refuses_a_non_terminal_stdin_or_stdout` (e2e) |
| Spec #73 entry: a missing monitor names the version | `missing_installed_monitor_fails_naming_its_version` (e2e) |
| Spec #73 startup: the executable home is resolved as the CLI does | `a_monitor_outside_a_version_folder_refuses_to_start` (e2e); core `issue74_monitor_resolves_the_same_layout_as_the_cli` |
| EXT-03 (Monitor, empty monitor): SIGINT, SIGTERM and SIGHUP exit 130, 143 and 129 with baseline and exact termios | `signals_release_the_host_with_signal_style_status` (e2e) |
| EXT-03 (Monitor, empty monitor): hung-up host input, read as its end or as EIO, releases the host before its one diagnostic, logged, 125 | `a_hung_up_host_input_releases_the_host_before_its_diagnostic`, `host_input_failing_with_eio_releases_the_host_before_its_diagnostic` (e2e); unit `only_eio_ends_host_input` |
| EXT-03 (Fault, empty monitor): a render failure, or a failed wait for host input, releases the host before its one diagnostic, 125 | `debug_hook_render_failure_restores_the_host_before_its_diagnostic`, `debug_hook_input_wait_failure_restores_the_host_before_its_diagnostic` (e2e) |
| Spec #73 host terminal; TRM-04 (Monitor, empty monitor): SIGWINCH repaints without leftovers, erasing or showing the cursor | `resizing_the_host_redraws_the_monitor_at_the_new_size` (e2e) |
| Spec #73 exit: content-free general log | `installed_monitor_logs_content_free_records_to_its_general_log` (e2e) |
| DSC-03 (Monitor), CTL-01 (Monitor), COM-01 (Monitor), LIB-01 (Monitor, Linux maps), HDL-01 (Monitor rows): sessions created after an empty start appear unasked, one passive client and library log each, reused across passes; never attached or controlled; the daemon's Idle status, never a fabricated one; titles and command fallback; exactly the version's `libclimon2_client.so` | `sessions_appear_as_they_are_created_each_with_one_passive_client` (e2e) |
| DSC-04 (Monitor, Fault): malformed and oversized metadata, mismatched hello, silent probe, inaccessible discovery, uncertain disappearance; no metadata deleted, no client torn down; a stale session nobody knew adds no row and gets no client (owner decision on #106) | `uncertain_discovery_is_explicit_and_never_deletes_metadata_or_clients` (e2e) |
| DSC-01 (Monitor, start order): bounded starts newest created first, ascending ID ties, each after the previous one failed, as the fake daemons heard them | `clients_start_one_at_a_time_newest_first_with_ascending_id_ties` (e2e) |
| DSC-03 (Monitor, cadence): immediate first pass, each later pass a second after the previous one ended, no overlap, the selector opened during a pass | `discovery_passes_start_a_second_after_the_previous_one_ends_without_overlap` (e2e) |
| Client supervisor (Fault), LIB-05 (Monitor, Fault): a failed notification lookup, a run failing without terminal evidence, or a failed drain ends that instance cleanly (deliveries and handles released, destroyed) and frees its start slot | `a_failed_notification_lookup_ends_its_client_cleanly_and_frees_its_start_slot`, `a_failed_run_without_terminal_evidence_ends_its_client_cleanly_and_frees_its_start_slot`, `a_failed_drain_ends_its_client_cleanly_and_frees_its_start_slot` (`src/supervisor.rs`) |
| Client supervisor (Fault): exit is bounded by `deadlines.shutdown_ms`, logs any client it could not confirm, and never destroys an instance with unreleased handles; a refused release is loaded once and its sessions are Unavailable | `exit_is_bounded_and_never_destroys_an_instance_with_unreleased_handles`, `a_refused_release_is_loaded_once_and_its_sessions_are_unavailable` (`src/supervisor.rs`) |
| Client supervisor: `scheduling.starts` at its bound starts that many clients and holds the next session (no client, no log directory) until one opens | `client_starts_never_exceed_the_start_lane` (`src/supervisor.rs`) |
| Client supervisor: terminal evidence before Open's response makes the row Unavailable with its message, or a stated reason when it is empty | `terminal_evidence_before_open_says_why_the_session_is_unavailable` (`src/supervisor.rs`) |
| Own-session scenario (confirmed 2026-11-08), DSC-06 (Monitor), HDL-01 (Monitor rows), CTL-01 (controlled elsewhere), TRM-06 (wide labels), SEL-01 (cell-safe clipping) | `the_own_session_is_this_monitor_and_nested_and_controlled_sessions_appear` (e2e) |
| SEL-01 (Monitor, basic cases): border, columns, highlight, Up/Down without wrap, Escape, no application resize or acquisition | `the_selector_lists_rows_moves_its_highlight_and_closes_without_resizing` (e2e) |
| DSC-03 (pure registry supplement): one client request per live session; rows tied on their entry time in ascending full session ID | `live_sessions_need_one_client_each` (`src/registry.rs`) |
| DSC-01 (Monitor, status order), COM-06 (Monitor labels), STA-07 (Monitor, hidden change), DSC-05 (Monitor, reorder case): real sessions at known daemon clock times show Needs attention, Running, Idle, Failed (signal 15), Failed (exit 143) and Succeeded in group and daemon-time order, finals kept after their sessions end; working under pending attention moves nothing, even with its daemon clock moved past a newer attention row's time; a reorder keeps the highlighted row | `real_sessions_show_their_effective_status_and_final_outcomes_in_order` (e2e) |
| DSC-01 (Monitor, status order: every status group, a tied change time broken by ascending full session ID, never creation order, delayed Initial metadata Starting until it arrives), DSC-05 (Monitor, reorder case: a row moving in above the highlight, and the highlighted row's own status change) | `rows_order_by_status_group_daemon_change_time_and_full_session_id` (e2e, Fault peers) |
| Not-connected rows (confirmed 2026-11-08), final rows without a status time (derived), STA-07 (Monitor, disconnection case): This monitor, Unavailable (no `status-v1`, uncertain) and Disconnected after every status and final row, newest condition first; a lost connection never shows its stale status or a fabricated outcome; untimed finals from Initial metadata and Command final sort by when learned, between newer and older daemon times | `not_connected_rows_and_untimed_finals_sort_by_when_the_monitor_recorded_them` (e2e, Fault peers) |
| DSC-01 status order, STA-07 (pure supplement): group, newest daemon change time, ascending full session ID; only a new change time moves a row; stale status of a disconnected row | `rows_order_by_group_then_newest_daemon_change_then_session_id` (`src/registry.rs`) |
| COM-06 labels, final rows (pure supplement): exact exit and signal labels, daemon time or learned time, finals surviving disconnection, later statuses and repeats | `final_outcomes_are_labelled_exactly_and_sort_by_their_time` (`src/registry.rs`) |
| Not-connected rows (pure supplement): every not-connected label in the disconnected group by entry time | `not_connected_rows_sort_by_when_they_entered_their_condition` (`src/registry.rs`) |
| STA-07, COM-06 (pure supplement over native records): Initial metadata status or final outcome, `CM1_STATUS` states and exact times, `CM1_FINAL` exit apart from signal; nothing for final, unknown or unreadable values | `the_initial_metadata_reports_its_status_or_final_outcome`, `status_records_keep_their_state_and_exact_change_time`, `command_final_reports_its_exact_result` (`src/supervisor/client.rs`) |
| Own-session scenario (pure supplement) | `the_own_session_is_this_monitor_and_never_needs_a_client` (`src/registry.rs`) |
| DSC-04 (pure supplement): uncertainty never removes rows or tears down clients | `uncertainty_never_removes_rows_or_tears_down_clients` (`src/registry.rs`) |
| Stale rows (owner decision on #106): a session proven gone drops its row unless it was ever connected | `a_session_proven_gone_drops_its_row_unless_it_was_ever_connected` (`src/registry.rs`) |
| COM-01 and not-connected labels (pure supplement) | `connection_statuses_are_labelled_as_specified` (`src/registry.rs`) |
| TRM-06 label cases (pure supplement): latest nonempty title, command fallback, literal, never IDs; cell-safe clipping | `labels_are_the_latest_nonempty_title_or_command_and_arguments_literally`, `clipping_is_display_cell_safe` (`src/registry.rs`) |
| SEL-01 basic cases (pure supplement) | `the_selector_draws_a_captionless_double_border_with_two_columns`, `up_and_down_move_without_wrapping_and_escape_closes`, `opening_an_open_selector_keeps_its_highlight`, `the_highlight_spans_its_row_and_stays_in_view`, `the_selector_draws_only_where_its_border_fits` (`src/selector.rs`) |
| SEL-01, DSC-05 (pure supplement): rows found after an empty open, or losing the highlighted row, leave nothing highlighted until Up or Down highlights the first row | `with_nothing_highlighted_up_or_down_highlights_the_first_row` (`src/selector.rs`) |
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
search, footer and Enter (#108, #113), initial selection (#266, moved
from #107; it acquires through #108) and the CTL-03 status-value case (deferred to #107
from #110; background, covered and revoked rows need #108's control), resource admission and fairness, including newest-first starts
across discovery batches, and `discovery.stall_ms` (#120), passive retry
(#116), an Escape that does not end its host read closing the selector
(#109, INP-02), the fail-stop after an unconfirmed shutdown (#118), Escape closing the key introduction when Escape is not
the prefix (#109, whose input router tells a bare Escape from an escape
sequence), and the universal archive's runtime on both architectures, which
only CI's `dist` and `archive` jobs build and execute.
