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
(#106), selection, control and painting of the selected session (#108), and
the input router that sends typed keys to it (#109). Nothing is selected
automatically yet (#107).

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
- Host input is read as raw bytes and classified by the pure input router
  (`router.rs`), never as lossy key events, and never depending on how the
  host splits its reads, with no timer anywhere (owner decision on #109): a
  sequence waits for its next byte whatever read brings it, and so does a
  lone Escape, which begins every sequence: the byte after it decides (`[`
  or `O` begins a key sequence; `]`, `P`, `_` or `^` a host reply's string;
  anything else makes it Escape then that key, as Alt and that key are). So
  in the selector and the key introduction Escape takes effect when the
  next key arrives. A lone Escape that ends a read bound for the selected
  session is released to it at once instead (`Router::release`), since
  applications time Escape themselves; if a sequence follows, only its rest
  is sent, still encoded for the application's modes. Known limits of that
  release: a reply, report or paste marker split right after its Escape
  leaves that Escape with the application, and a keypad key split so cannot
  become a numeric-mode key. Bracketed paste
  (data until its end marker, which discards a pending prefix and leaves
  none), SGR mouse reports, focus reports and host replies bypass shortcuts
  and reach no application. The monitor's rule for host replies (confirmed
  by the owner on #109): OSC, DCS, APC and PM strings; CSI sequences with a
  `?`, `>` or `=` marker, an intermediate byte, or a `c`, `n`, `t` or `y`
  final; and any control sequence longer than 64 bytes. CSI `R` stays a key
  (modified F3 sends it, and the monitor never asks the host for its cursor
  position); pastes, mouse and focus reach
  applications with #111. Prefix+s, x, j and ? (lowercase) are the global
  actions from every screen, the selector and the too-small host; prefix+j
  is recognised and never forwarded, and its jump arrives with #114. A
  doubled prefix is one literal prefix, a prefix and any other key (`d`,
  `h`, `r`, `J`, the take-control key, a whole cursor key) are both sent
  literally, and a lone prefix waits indefinitely. With `Ctrl+[` as the
  prefix a bare Escape is the prefix, so it waits and never cancels, and the
  legacy Alt+s, Alt+x, Alt+j and Alt+? encodings are the actions.
- Only the selected session, controlling with its restoration applied,
  Ready accepted and painted, with no modal open and room on the host,
  receives application input; otherwise input is discarded, never kept for
  later. Ctrl+C is ordinary input. Each read's keys for the session are
  submitted together, before anything after them acts. Nothing read after
  prefix+x is routed; keys read before it in the same read reach the session
  before shutdown (below). Unmodified cursor keys (`A`-`D`, `H`, `F`, as CSI or SS3) are
  sent as the application's cursor mode (DECCKM) expects, and application
  keypad keys (SS3) as its keypad mode (DECKPAM) expects; the host keypad is
  put in application mode while the selected, uncovered session asks for it,
  so its keypad keys are distinct. The supervisor submits input in chunks of
  at most `transfer.chunk_bytes`, in order, and stops at the first refusal;
  the top row then shows `Input not sent (N bytes; earlier input was
  accepted): <why>`, N counting the refused chunk and every byte after it.
- Every other key goes to the key introduction while it is open, otherwise
  to the open selector, and a prefix with a key that completes no action
  reaches neither. A doubled prefix is one literal prefix there too, so with
  `Ctrl+M` as the prefix Enter Enter continues. The key introduction acts
  only on Enter (close), a bare Escape (close; never the prefix's) and Space
  (reverse the saved choice).
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
  Up and Down (`ESC [ A`/`B` or `ESC O A`/`B`) move the highlight without
  wrapping, and a bare Escape closes it when the next key arrives (the
  router has no timer); a cursor key split across reads anywhere never
  closes it, and the key after the Escape is routed once it has closed
  (#109, INP-02). Opening it never resizes an application or
  acquires anything. Enter (a typed CR) on a connected row not known to be
  final selects that session, closes the selector and acquires it (below);
  on any other row it does nothing yet. While rows exist, nothing is
  selected and the selector is closed, the body reads `No session selected.`
  above the key hint.
- **Exit** first waits until every input chunk an open client accepted is
  answered, at most `deadlines.operation_ms` (owner decision on #109), so
  keys typed with prefix+x reach their session and a stuck session holds
  exit no longer than one operation may take. It then asks the supervisor
  to shut every instance down and keeps
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
  from 2, after Open, and matches each answer to its request). A session
  selected earlier keeps control in the background when another is selected.
- **Terminals** (`terminal.rs`, pure): each session has one `climon2-vt`
  model, kept by session ID for the whole run and bounded by
  `memory.client_bytes` and `history.max_rows` (until admission divides that
  budget with the library, #120). Its client's records drive it: a grant
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
  frozen view stays shown. Applied, Ready accepted and painted are
  distinct; input may use the period's authority only with all three and a
  current model (`Terminal::input`, which the input router checks).
- **Canvas.** The selected session's active screen is painted directly below
  the top row at the model's geometry, clipped to the host, with blank
  margins: each cell's character and combining marks, a wide character over
  two columns (blank if the right edge would cut it), colours and rendition
  (every underline style as a single underline). Its cursor is shown with its
  DECSCUSR shape and blink where the application left it, unless hidden, off
  the host or covered by the selector or key introduction; never in the top
  row. While its restoration is staged the last applied frame stays; before
  any, the canvas is blank.
- **Top row.** With a session selected: its label (clipped, never moved by
  the fields), then at the right edge, two spaces apart, its status as the
  selector shows it (Unavailable until status reaches the monitor, #95,
  #107), its control (`Acquiring`, `Restoring`, `Controlling` once input may
  go, `Not controlling`, or `Ended` once frozen), the latest error (a refused
  request, or the model's limit) until the next applied restoration, and,
  unless `monitor.show_key_hints` is false, `<prefix> then s: sessions`. A
  request refused under a restoration since superseded is not an error; a
  Resize refused so is sent again under the current one.
- **Geometry.** The selected controller is asked for the host width by its
  height less the top row: after every grant, on selection and on each real
  host resize, once per control period and size. Background controllers get
  none. A host under two rows (no row below the top row) or two columns
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
| CTL-05 (Monitor), TRM-01 (Monitor, basic cases), spec #73 Enter and initial selection before status (#108): nothing selected at startup; Enter selects and acquires; plain, ANSI-coloured, bold-on-blue, wide and combining text and the cursor painted as written, live; a later arrival never moves the selection | `enter_selects_and_acquires_a_session_and_paints_it_live` (e2e) |
| TRM-02 (Monitor): only the daemon answers session queries; no session byte or title reaches the host; the host is never queried | `only_the_daemon_answers_session_queries_and_no_session_byte_reaches_the_host` (e2e) |
| TRM-03 (Monitor) at 80x24, 200x60, 500x200 and a larger host: one top row; the desired area is the host less one row; canonical bytes at the effective geometry with blank margins | `the_selected_session_fills_the_host_below_one_top_row_at_its_effective_geometry` (e2e) |
| CTL-05 (Monitor, resize), TRM-03 (real host resizes): Resize only to the selected controller, never to a background one | `host_resizes_reach_only_the_selected_controller` (e2e) |
| TRM-04 (Monitor, too-small cases): one row too small, two rows and two columns enough; no resize while too small; recovery; prefix+x still exits | `a_host_too_small_for_a_session_says_so_sends_no_resize_and_recovers` (e2e) |
| Top-row key hints (#108): none with `monitor.show_key_hints` false | `without_key_hints_the_top_row_shows_no_hint` (e2e) |
| Spec #73 Enter, never exposing cached output; CTL-05 (revoked): a revoked session's cache is never painted; Enter takes control again | `a_revoked_session_shows_no_cached_output_until_enter_takes_control_again` (e2e) |
| #94 rule for #108: a finished session's view freezes at Command final; Enter does not acquire a final row | `a_finished_session_keeps_its_last_screen_and_cannot_be_acquired` (e2e) |
| CTL-05, COM-05 (pure supplement): Ready only after the apply; applied, Ready accepted and painted distinct, input needs all three; only Ready for the applied restoration counts | `ready_applied_and_painted_stay_distinct_and_input_needs_all_three`, `only_ready_accepted_for_the_applied_restoration_opens_input` (`src/terminal.rs`) |
| TRM-04, COM-05 (pure supplement): staged restorations replace the model whole once applied; superseded ones never become current; a cache is never shown | `a_restoration_replaces_the_model_whole_only_once_applied`, `a_cache_from_an_earlier_period_is_never_shown` (`src/terminal.rs`) |
| Carried from #76 (owner decision 2026-11-13): a Display at a geometry the model lacks marks it stale until the next restoration | `a_display_at_a_geometry_the_model_lacks_marks_it_stale_until_restored` (`src/terminal.rs`) |
| #94 rule (pure supplement): the view freezes at Command final | `the_view_freezes_at_the_command_final` (`src/terminal.rs`) |
| CTL-05 (pure supplement): Resize once per grant and desire, never without control; superseded refusals are not errors and a refused Resize is resent; Acquiring and errors | `resize_follows_every_grant_and_each_new_desire_only_while_controlling`, `work_refused_under_a_superseded_restoration_is_not_an_error_and_resize_is_resent`, `acquiring_ends_at_the_grant_or_the_answer_and_errors_are_shown`, `each_control_has_its_label` (`src/terminal.rs`) |
| LIM-03 (pure supplement): a restoration over the model's limit is an error without Ready | `a_restoration_over_the_model_limit_is_an_error_without_ready` (`src/terminal.rs`) |
| TRM-01 (pure supplement): exact SGR for the 16 ANSI, palette, direct and default colours, underline colour and renditions; wide and combining text; the cursor | `colours_and_renditions_are_painted_with_their_own_sgr`, `wide_and_combining_text_and_the_cursor_are_painted_in_place`, `each_row_starts_with_its_own_cursor_position` (`src/paint.rs`) |
| TRM-03, TRM-04 (pure supplement): the desired area, and one column or one row too small | `the_application_area_is_the_host_less_its_top_row_unless_too_small` (`src/screen.rs`) |
| Spec #73 Enter (pure supplement): only connected rows not known to be final are acquirable; Enter chooses the highlighted row | `only_connected_rows_not_known_to_be_final_are_acquirable` (`src/registry.rs`), `enter_chooses_the_highlighted_row` (`src/selector.rs`) |
| Client supervisor (#108): control and display records reach the terminal as delivered; Acquire, Ready and Resize submitted in order and answered as themselves | `control_and_display_records_reach_the_terminal_as_delivered`, `requests_are_submitted_in_order_and_answered_as_themselves` (`src/supervisor/tests`) |
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
| INP-01, INP-02, TRM-02 (Monitor), COM-04 (Monitor, typed), EXT-03 (Monitor, Ctrl+C): typed bytes reach the selected session exactly, binary, invalid UTF-8 and Ctrl+C included; one literal prefix for a doubled one, both bytes for d, h, r, J and the take-control key, nothing for prefix+j, replies and reports; identical when split into ten reads; cursor and keypad keys follow the application's modes and the host keypad follows it | `typed_bytes_reach_the_selected_session_exactly_however_reads_split` (e2e) |
| INP-01 (Monitor, live and too-small screens), spec #73 Routing: prefix+? and prefix+s from a live session, their keys never reach it; on a too-small host input is discarded, never buffered, prefix+s still opens the selector and prefix+x still exits | `global_actions_work_from_live_and_too_small_screens_and_input_is_never_buffered` (e2e) |
| INP-02, HLP-02 (Monitor, Escape; owner decisions on #105, #106 and #109): a cursor key split right after its Escape or after its introducer never closes the selector or key introduction; a lone Escape closes it when the next key arrives, which reaches the session; Escape then a key in one read does the same | `split_escape_sequences_never_close_dialogs_and_escape_then_a_key_is_both` (e2e) |
| Spec #73 exit (owner decision on #109): keys read with prefix+x reach the session before shutdown; a stuck session holds exit at most the operation deadline (plus the shutdown deadline) | `typed_bytes_reach_the_selected_session_exactly_however_reads_split`, `a_stuck_session_cannot_hold_exit_beyond_the_operation_deadline` (e2e); `stopping_waits_for_accepted_input_at_most_the_operation_deadline` (`src/supervisor/tests`) |
| COM-03 (Monitor, queue full), COM-04 (Monitor, chunks): a refused chunk is shown with the bytes not sent, earlier input accepted; only accepted input arrives | `a_full_input_queue_is_shown_and_only_accepted_input_arrives` (e2e) |
| INP-01 (pure input-router supplement): the global actions with default and custom prefixes; a doubled prefix; unknown suffixes; a lone prefix waits | `the_prefix_and_lowercase_s_x_j_or_question_mark_are_the_global_actions`, `a_doubled_prefix_is_one_literal_prefix`, `an_unknown_suffix_is_the_prefix_and_that_key`, `a_lone_prefix_waits_indefinitely` (`src/router.rs`) |
| INP-02 (pure input-router supplement): every split routes identically, and sends an application the same bytes even with a released Escape; a split cursor key is never an Escape; Escape and a key; a lone Escape waits for the next byte, and one bound for an application is released at once, once, never as or after a prefix; Ctrl+[ as the prefix; broken and overlong (64 and 65 bytes) sequences; cursor and keypad encodings | `every_split_of_the_reads_routes_identically`, `a_split_cursor_key_is_never_an_escape`, `escape_and_a_key_in_one_read_are_escape_then_that_key`, `a_lone_escape_waits_for_the_next_byte`, `a_lone_escape_for_an_application_is_released_at_once`, `an_application_receives_the_same_bytes_for_every_split`, `with_escape_as_the_prefix_bare_escape_waits_and_alt_keys_are_actions`, `a_broken_sequence_is_its_bytes`, `an_overlong_sequence_is_discarded`, `cursor_keys_follow_the_application_cursor_mode`, `keypad_keys_follow_the_application_keypad_mode` (`src/router.rs`); `the_host_keypad_mode_is_written_when_it_changes` (`src/screen.rs`) |
| TRM-02, INP-03, INP-05 (pure input-router supplement): replies, mouse and focus reports and pastes are never keys and never complete a pending prefix; a paste discards one | `replies_reports_and_pastes_are_never_keys`, `a_paste_discards_a_pending_prefix_and_leaves_none` (`src/router.rs`) |
| COM-04, EXT-03 (pure input-router supplement): typed bytes reach an application exactly | `typed_bytes_reach_the_application_exactly` (`src/router.rs`) |
| COM-03, COM-04 (pure supplement): chunks of at most `chunk_bytes`; the first refusal carries every unsent byte and consumes no request ID; a refusal is shown | `input_is_submitted_in_chunks_and_a_refusal_carries_every_unsent_byte` (`src/supervisor/tests`), `refused_input_shows_what_was_not_sent` (`src/terminal.rs`) |
| #74 risk proof: plain and hooked sibling binaries without nested builds | `cargo_builds_the_plain_or_hooked_monitor_binary_for_its_tests` (`tests/build.rs`, plain and hooked runs); fixtures `workspace_binaries_contain_test_hooks`; xtask `release_check_builds_unhooked_binaries_in_a_private_target` |
| #74 release checks: version, architecture and test-hook marker | xtask `version_check_rejects_a_missing_or_wrong_version_monitor`; Linux `linux_packaging_rejects_a_non_x86_64_or_hooked_monitor`; macOS `binary_check_rejects_a_single_architecture_monitor`, `binary_check_rejects_a_hooked_monitor` |

Not yet claimed: EXT-03's root OSC 7501 `clear` (#118), paste, mouse and focus
routing to applications and target-change invalidation (#111, INP-03 to
INP-05), global actions from the placeholder (#110) and history (#116)
screens, HLP-02 over the selector (#113), selector
search and footer (#113), Enter on a row that is not connected, is final or
is this monitor (read-only selection and its placeholders), the revoked
placeholder and take-control key (#110), the label marquee and synchronized
updates (#112), underline styles other than single, status order and
initial selection (#107), resource admission and fairness, including newest-first starts
across discovery batches, and `discovery.stall_ms` (#120), passive retry
(#116), the fail-stop after an unconfirmed shutdown (#118), and the universal archive's runtime on both architectures, which
only CI's `dist` and `archive` jobs build and execute.
