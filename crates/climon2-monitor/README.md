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
(#106), and selection, control and painting of the selected session (#108).
Nothing is selected automatically yet (#107), and typed input reaches no
session yet (#109).

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
- Ratatui 0.30 renders each frame into a buffer, and the monitor's own
  writer (`paint.rs`) paints every cell of it over the monitor-owned stdout,
  each with its full SGR: the 16 ANSI colours keep their own SGR 30-37 and
  90-97 (and backgrounds), which Ratatui's crossterm backend would rewrite as
  palette entries, and SGR 58 underline colours are kept. Nothing is erased
  first, the cursor is hidden while cells are written and shown only for a
  selected session (below), and the host is never queried. With nothing
  selected, the permanent top row reads `No session selected`; below it the centred empty state reads
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
  open, otherwise to the open selector, and is otherwise discarded: routing
  it to the selected session is the input router's (#109). A doubled prefix
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
  (`CLIMON2_SESSION_ID`). A live session with no client and not the monitor's
  own gets exactly one client request. Discovery never changes a client's
  connection. A session proven gone (stale) drops its row unless a client
  was ever connected to it, and a stale session nobody knew adds no row; a
  row that was connected stays, with its history (owner decision on #106).
  The status column is This monitor, Starting, Unavailable (uncertain,
  failed, or connected without `status-v1`), Disconnected (its connection
  ended), or empty for a connected row with `status-v1`,
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
  released them. The supervisor waits up to a 20 ms tick on the borrowed
  notification descriptors, then drains every live instance until it is
  empty (so one whose descriptor lookup failed still ends), releases every
  delivery at once (`supervisor/client.rs`), and reports Open's outcome
  (with whether `status-v1` was selected), titles from Initial metadata and
  `CM1_TITLE`, and the end. A refused release (its loader `Failure` kept for
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
  table) move the highlight without wrapping, and an Escape that ends a host
  read closes it. Known gap (owner decision on #106): an Escape followed by
  other bytes in the same read does not close it; the input router closes
  this gap (#109, INP-02). Opening it never resizes an application or
  acquires anything. Enter (a typed CR) on a connected row not known to be
  final selects that session, closes the selector and acquires it (below);
  on any other row it does nothing yet. While rows exist, nothing is
  selected and the selector is closed, the body reads `No session selected.`
  above the key hint.
- **Exit** first asks the supervisor to shut every instance down and keeps
  draining until each has ended, its handles are released and its owner is
  joined, for at most `deadlines.shutdown_ms` (and one tick), then releases
  the host. An instance unconfirmed by then is logged (`client shutdown
  unconfirmed at its deadline`) and left to process exit: its owner never
  destroys it while its handles may be unreleased. The fail-stop that
  should follow arrives with #118.

## Selection, control and painting (#108)

- **Selection.** Nothing is selected at startup; Enter selects. Once a
  session is selected, no arrival moves the selection. Selecting submits
  Acquire through the session's client (`supervisor.rs` numbers each request
  from 2, after Open, and matches each answer to its request). A request
  enters the client's pending ledger, and takes its ID, only once the
  library accepts it; a refused one is answered at once and consumes no ID.
  The ledger holds at most `queues.request_records` + `queues.operations`
  requests (every request the library may have accepted), and one more is
  refused at once. A session selected earlier keeps control in the
  background when another is selected. Enter on a row that is not acquirable
  (disconnected, final, starting, awaiting resources, unavailable without a
  client, or This monitor) is ignored until
  [#110](https://github.com/jackgeek/climon2/issues/110) and
  [#116](https://github.com/jackgeek/climon2/issues/116), which own those
  cases.
- **Terminals** (`terminal.rs`, pure): each session has one `climon2-vt`
  model, kept by session ID for the whole run and bounded by
  `memory.client_bytes` and `history.max_rows`. The model and the library
  are each bounded by all of `memory.client_bytes`, so a client may use up to
  twice it: an approved POC gap, recorded on
  [#120](https://github.com/jackgeek/climon2/issues/120), which divides that
  budget. Its client's records drive it: a grant
  starts a control period; each Restoration begin stages a separate model
  (a newer one supersedes it, and it never becomes current); its Display
  bytes build it; Restoration complete replaces the model and its history
  whole, and only then is Ready submitted, also for a background controller.
  Live Display bytes update the current model, but a Display at a geometry
  the model does not have (after a Geometry with no restoration yet) marks
  it stale: it is never reflowed and takes no more bytes until the next
  restoration replaces it. The model never answers queries and nothing of
  the session reaches the host. A revocation, or the end of the client,
  ends control and abandons a staged restoration; the model is then a cache
  and is never shown, nor while a later grant's restoration is staged. A
  Command final (from the Initial metadata or its own record) freezes the
  view: the last output and the release that follow change nothing, and the
  frozen view stays shown, but only if the control period held then had
  applied a restoration; otherwise (after a revocation, or during the
  period's first restoration) no view is shown, never an earlier period's
  cache or a blank model. The daemon sends Command final only after the
  output drains (#88), so no Display is held or guessed. A finished session
  is sent no Resize. Applied, Ready accepted and painted are distinct; input
  may use the period's authority only with all three and a current model
  (`Terminal::input`, for #109's router). Painted is decided before each
  frame: true only for the selected session's terminal while nothing covers
  its canvas and the host has room, so selecting another session, opening
  the selector or key introduction, or a too-small host closes input until a
  frame shows the restoration again. The control label ignores visibility
  (owner decision, 2026-11-14): `Controlling` once the restoration is applied
  and Ready accepted, even while the canvas is covered.
- **Canvas.** The selected session's active screen is painted directly below
  the top row at the model's geometry, clipped to the host, with blank
  margins: each cell's character and combining marks, a wide character over
  two columns (blank if the right edge would cut it; the engine's width, never
  a re-measured symbol, decides what a cell covers, and the next cell is
  placed with CUP wherever the host may measure it otherwise), colours and
  rendition (every underline style as a single underline until #112; SGR 58
  only on an underlined cell with a colour of its own, in the T.416 colon
  form, since SGR 0 resets it and a host that does not know it skips the
  colon form whole). Its cursor is shown with its
  DECSCUSR shape and blink where the application left it, unless hidden, off
  the host or covered by the selector or key introduction; never in the top
  row. While its restoration is staged the last applied frame stays; before
  any, the canvas is blank.
- **Top row** (owner decision, 2026-11-14). With a session selected,
  fixed-width slots at the right edge, each after ` | `, never move whatever
  their values: its status as the selector shows it (Unavailable until
  status reaches the monitor, #95, #107) in 18 cells, its control
  (`Acquiring`, `Restoring`, `Controlling` once its restoration is applied and ready, `Not
  controlling`, or `Ended` once frozen) in 15, and, unless
  `monitor.show_key_hints` is false, `<prefix> then s: sessions`. The label
  fills the cells they leave from the left; the latest error (a refused
  request, or the model's limit, until the next applied restoration) follows
  it after ` | ` in what the label leaves, so the error is clipped first and
  never squeezes out the label. A host narrower than the slots shows them
  from its left edge, clipped. #112 adds the label marquee. A request
  refused under a restoration since superseded is not an error; a Resize
  refused so is sent again under the current one.
- **Geometry.** The selected controller is asked for the host width by its
  height less the top row: after every grant, on selection and on each real
  host resize, once per control period and size. Background controllers get
  none. A host under four rows or five columns (owner decision, 2026-11-14)
  shows only `Terminal too small` in its top row and sends no resize; clients,
  control and models carry on, global keys still act, and a larger host
  resizes and paints the session again.

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
| DSC-01 (Monitor, start order): bounded starts newest created first, ascending ID ties, each after the previous one failed, as the fake daemons heard them | `clients_start_one_at_a_time_newest_first_with_ascending_id_ties` (e2e) |
| DSC-03 (Monitor, cadence): immediate first pass, each later pass a second after the previous one ended, no overlap, the selector opened during a pass | `discovery_passes_start_a_second_after_the_previous_one_ends_without_overlap` (e2e) |
| Client supervisor (Fault): a failed notification lookup, or a run failing without terminal evidence, ends that instance cleanly (deliveries and handles released, destroyed) and frees its start slot | `a_failed_notification_lookup_ends_its_client_cleanly_and_frees_its_start_slot`, `a_failed_run_without_terminal_evidence_ends_its_client_cleanly_and_frees_its_start_slot` (`src/supervisor/tests`) |
| Client supervisor (Fault): exit is bounded by `deadlines.shutdown_ms` and never destroys an instance with unreleased handles; a refused release is loaded once and its sessions are Unavailable | `exit_is_bounded_and_never_destroys_an_instance_with_unreleased_handles`, `a_refused_release_is_loaded_once_and_its_sessions_are_unavailable` (`src/supervisor/tests`) |
| Own-session scenario (confirmed 2026-11-08), DSC-06 (Monitor), HDL-01 (Monitor rows), CTL-01 (controlled elsewhere), TRM-06 (wide labels), SEL-01 (cell-safe clipping) | `the_own_session_is_this_monitor_and_nested_and_controlled_sessions_appear` (e2e) |
| SEL-01 (Monitor, basic cases): border, columns, highlight, Up/Down without wrap, Escape, no application resize or acquisition | `the_selector_lists_rows_moves_its_highlight_and_closes_without_resizing` (e2e) |
| DSC-01/DSC-03 (pure registry supplement): one client request per live session, newest-first order | `live_sessions_need_one_client_each_and_list_newest_first` (`src/registry.rs`) |
| Own-session scenario (pure supplement) | `the_own_session_is_this_monitor_and_never_needs_a_client` (`src/registry.rs`) |
| DSC-04 (pure supplement): uncertainty never removes rows or tears down clients | `uncertainty_never_removes_rows_or_tears_down_clients` (`src/registry.rs`) |
| Stale rows (owner decision on #106): a session proven gone drops its row unless it was ever connected | `a_session_proven_gone_drops_its_row_unless_it_was_ever_connected` (`src/registry.rs`) |
| COM-01 and not-connected labels (pure supplement) | `connection_statuses_are_labelled_as_specified` (`src/registry.rs`) |
| TRM-06 label cases (pure supplement): latest nonempty title, command fallback, literal, never IDs; cell-safe clipping | `labels_are_the_latest_nonempty_title_or_command_and_arguments_literally`, `clipping_is_display_cell_safe` (`src/registry.rs`) |
| SEL-01 basic cases (pure supplement) | `the_selector_draws_a_captionless_double_border_with_two_columns`, `up_and_down_move_without_wrapping_and_escape_closes`, `opening_an_open_selector_keeps_its_highlight` (`src/selector.rs`) |
| CTL-05 (Monitor), TRM-01 (Monitor, basic cases), spec #73 Enter and initial selection before status (#108): nothing selected at startup; Enter selects and acquires; plain, ANSI-coloured, bold-on-blue, wide and combining text and the cursor painted as written, live; a later arrival never moves the selection; the open selector covers the canvas and the control still reads Controlling | `enter_selects_and_acquires_a_session_and_paints_it_live` (e2e) |
| TRM-02 (Monitor): only the daemon answers session queries; no session byte or title reaches the host; the host is never queried | `only_the_daemon_answers_session_queries_and_no_session_byte_reaches_the_host` (e2e) |
| TRM-03 (Monitor) at 80x24, 200x60, 500x200 and a larger host: one top row; the desired area is the host less one row; canonical bytes at the effective geometry with blank margins | `the_selected_session_fills_the_host_below_one_top_row_at_its_effective_geometry` (e2e) |
| CTL-05 (Monitor, resize), TRM-03 (real host resizes): Resize only to the selected controller, never to a background one | `host_resizes_reach_only_the_selected_controller` (e2e) |
| TRM-04 (Monitor, too-small cases): 3 rows and 4 columns too small, 4 rows and 5 columns enough; the session reads its size while too small and was never resized; recovery; prefix+x still exits | `a_host_too_small_for_a_session_says_so_sends_no_resize_and_recovers` (e2e) |
| Top-row key hints (#108): none with `monitor.show_key_hints` false | `without_key_hints_the_top_row_shows_no_hint` (e2e) |
| Spec #73 Enter, never exposing cached output; CTL-05 (revoked): a revoked session's cache is never painted; its screen changes while revoked, and Enter takes control again and paints only the new restoration | `a_revoked_session_shows_no_cached_output_until_enter_takes_control_again` (e2e) |
| #94 rule for #108: a finished session's view freezes at Command final; Enter does not acquire a final row | `a_finished_session_keeps_its_last_screen_and_cannot_be_acquired` (e2e) |
| CTL-05, COM-05 (pure supplement): Ready only after the apply; applied, Ready accepted and painted distinct, input needs all three; only Ready for the applied restoration counts | `ready_applied_and_painted_stay_distinct_and_input_needs_all_three`, `only_ready_accepted_for_the_applied_restoration_opens_input` (`src/terminal.rs`) |
| TRM-04, COM-05 (pure supplement): staged restorations replace the model whole once applied; superseded ones never become current; a cache is never shown | `a_restoration_replaces_the_model_whole_only_once_applied`, `a_cache_from_an_earlier_period_is_never_shown` (`src/terminal.rs`) |
| Carried from #76 (owner decision 2026-11-13): a Display at a geometry the model lacks marks it stale until the next restoration | `a_display_at_a_geometry_the_model_lacks_marks_it_stale_until_restored` (`src/terminal.rs`) |
| #94 rule (pure supplement): the view freezes at Command final, only if the period held then had applied a restoration; a finished session gets no Resize; painted is not sticky; the control label ignores visibility | `the_view_freezes_at_the_command_final`, `a_command_final_without_a_live_model_shows_no_view`, `a_finished_session_is_sent_no_resize`, `a_frame_that_stops_showing_the_restoration_closes_input`, `the_control_label_ignores_whether_the_canvas_shows` (`src/terminal.rs`) |
| CTL-05 (pure supplement): Resize once per grant and desire, never without control; superseded refusals are not errors and a refused Resize is resent; Acquiring and errors | `resize_follows_every_grant_and_each_new_desire_only_while_controlling`, `work_refused_under_a_superseded_restoration_is_not_an_error_and_resize_is_resent`, `acquiring_ends_at_the_grant_or_the_answer_and_errors_are_shown`, `each_control_has_its_label` (`src/terminal.rs`) |
| LIM-03 (pure supplement): a restoration over the model's limit is an error without Ready | `a_restoration_over_the_model_limit_is_an_error_without_ready` (`src/terminal.rs`) |
| TRM-01 (pure supplement): exact SGR for the 16 ANSI, palette, direct and default colours, underline colour (colon form, underlined cells only) and renditions; wide and combining text; each cell covers its given width; the cursor | `colours_and_renditions_are_painted_with_their_own_sgr`, `wide_and_combining_text_and_the_cursor_are_painted_in_place`, `each_cell_covers_the_width_it_was_given`, `each_row_starts_with_its_own_cursor_position` (`src/paint.rs`) |
| TRM-03, TRM-04 (pure supplement): the desired area; 4 rows and 5 columns enough, one fewer of either too small | `the_application_area_is_the_host_less_its_top_row_unless_too_small` (`src/screen.rs`) |
| Top row (owner decision 2026-11-14, pure supplement): fixed slots for status, control and hint whatever the control word; the error clipped first, never squeezing out the label | `status_and_control_keep_their_slots_whatever_the_control`, `an_error_is_clipped_first_and_never_squeezes_out_the_label` (`src/screen.rs`) |
| TRM-01 (pure supplement): the cursor only inside the host; a wide character the edge would cut left blank; each rendition reaches its cell; DECSCUSR for every shape | `the_cursor_shows_only_inside_the_host`, `a_wide_character_the_host_edge_would_cut_is_left_blank`, `each_rendition_reaches_its_cell`, `each_cursor_shape_has_its_decscusr` (`src/screen.rs`) |
| Spec #73 Enter (pure supplement): only connected rows not known to be final are acquirable; Enter chooses the highlighted row | `only_connected_rows_not_known_to_be_final_are_acquirable` (`src/registry.rs`), `enter_chooses_the_highlighted_row` (`src/selector.rs`) |
| Client supervisor (#108): control and display records reach the terminal as delivered; Acquire, Ready and Resize submitted in order and answered as themselves; a request refused at submit is not pending and consumes no ID; the pending ledger is bounded | `control_and_display_records_reach_the_terminal_as_delivered`, `requests_are_submitted_in_order_and_answered_as_themselves`, `a_request_refused_at_submit_is_not_pending_and_consumes_no_id`, `the_pending_ledger_is_bounded_by_the_requests_the_library_may_hold` (`src/supervisor/tests`) |
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
search and footer (#113), Enter on a row that is not connected, is final or
is this monitor (read-only selection and its placeholders), the revoked
placeholder and take-control key (#110), the label marquee and synchronized
updates (#112), underline styles other than single, status order and
initial selection (#107), resource admission and fairness, including newest-first starts
across discovery batches, and `discovery.stall_ms` (#120), passive retry
(#116), an Escape that does not end its host read closing the selector
(#109, INP-02), the fail-stop after an unconfirmed shutdown (#118), Escape closing the key introduction when Escape is not
the prefix (#109, whose input router tells a bare Escape from an escape
sequence), and the universal archive's runtime on both architectures, which
only CI's `dist` and `archive` jobs build and execute.
