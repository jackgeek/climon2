# climon2-monitor

The [monitor](../../GLOSSARY.md#sessions) executable, `climon2-monitor`, ships in
every version folder beside the CLI, client and daemon, on macOS, x86-64
GNU/Linux and x86-64 Windows (#327). Users reach it as `c2 monitor` or `c2 m`: the launcher runs the current version's CLI, which execs its own
version's monitor with no arguments or envelope. That handoff between
executables of one release is not frozen. The executable is built from the
monitor's terminal interface,
[`climon2-monitor-tui`](../climon2-monitor-tui/README.md), whose library
boundary is `run() -> Result<u32, Error>`; the thin main prints
`climon2: error: …` for an `Error` and exits 125.

This crate is the [monitor library](../../GLOSSARY.md#sessions) (#490,
[ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md)): a
whole monitor with no interface of its own. This document describes the
monitor as its user meets it, so it covers both crates; each module named
below lives in the crate [Two crates](#two-crates) assigns it.

This is the #74 walking skeleton of spec #73, with its key introduction and
help (#105), session discovery, passive clients and the basic selector
(#106), row statuses, final outcomes and status order (#107), selection,
control and painting of the selected session (#108), the input router that
sends typed keys to it (#109), and switching, revocation, taking control back
and Enter on Starting and Unavailable rows (#110), initial selection
(#266), paste, mouse and focus (#111), history scrolling, faithful painting
and static truncation (#112), the selector's search, navigation and footer (#113), viewing, acknowledgement, the prefix+j attention jump and the host attention reports (#114), toasts and the bell (#115), disconnection, read-only history
and passive recovery (#116), Delete in the selector (#117), coordinated shutdown with a truthful exit
status (#118), memory accounting, bounded history and the
render and work cadence (#119), and admission, library loading and fair
scheduling (#120). Every string the monitor shows is in [On-screen strings](#on-screen-strings).

## Two crates

One consumer thread owns a `Monitor` and steps it. `Monitor::start` takes
everything it reads as `Inputs`: the climon2 home, the client library loader,
the configuration snapshot, the monitor's own session and where its threads
log. `step` takes a `Clock` and returns the `Event`s since the last step:
host attention, a toast's bell, a selection, a removed row. `act` takes one
`Action` and the time. The actions are one closed set, grouped by what they
act on: `Choice` (select, the attention jump, Delete), `Drive` (take control,
input, a paste, a resize) and `Look` (scroll, dismiss the toast,
acknowledge). A `Consumer` reports only what the library cannot know: whether
a modal of its own covers the selected session, whether the user's window has
focus, the size it can show, and whether a toast fits and where it would
cover the canvas. The consumer reads rows, toasts, the selected `Session` and
its read-only views straight from the monitor. The library never reads or
writes the process's terminal; its tests in `tests/monitor.rs` drive a whole
monitor on a home of their own with no terminal.

| Crate | Modules |
| --- | --- |
| `climon2-monitor` (this crate) | `monitor` (the stepped monitor and its shutdown rounds), `action`, `consumer`, `session` (what the top row and canvas show of the selected session), `registry`, `discovery`, `initial`, `supervisor`, `terminal`, `pressure`, `cadence` (the work quanta), `attention` (deferred acknowledgement, the jump's notice, which attention state the host last heard), `toast` (the queue, its ledger and timing), `search`, `delete` (where a Delete leaves the selection), `shutdown`, `startup` (the home, the install layout and the configuration), `links` (the hub monitor's side of monitor links: pairing, #501), `remote` (its links to the relays it remembers, #502), `machine` (a session by its machine and session ID, #502) |
| [`climon2-monitor-tui`](../climon2-monitor-tui/README.md) | `lib` (the host loop, which steps the monitor and delivers its events), `host`, `router`, `keys`, `mouse`, `selector` (navigation, its search box and drawing), `introduction`, `preference`, `screen`, `paint`, `frames` (the render cadence), `toast` (placement, hit-testing and drawing), `delete` (the confirmation), `attention` (the host attention reports' bytes), `failstop` |

## Startup, before the host terminal changes

1. stdin and stdout must be TTYs (`c2 monitor needs a terminal`).
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
   and `monitor.limits` with the ranges and rules the
   [configuration guide](../../docs/for-humans/guide/configuration.md) lists). Nothing
   reads the file again, so editing it changes nothing until the next start.
   Its limits must then hold one session's mandatory allocation at the
   largest geometry a session can have, `climon2-vt`'s `Geometry::LARGEST`
   (500x200), by the parts its `Model::mandatory` charges a blank model
   keeping `history.max_rows` (#167): `memory.total_bytes` the monitor's one
   shared parser (`Parser::BYTES`, #166), the current model, the history
   chunk holding its recent page and the client library's share (#119)
   together, then `client_bytes` the model, chunk and library share, and
   `history_bytes_total`, then `history_bytes_per_session`, that chunk
   (`Limits::hold`). That admits the largest session but not a restoration
   staged beside it, at least 33,932,174 bytes in the model's share of a
   client's budget (36,029,326 in the total, with the parser) at 500x200,
   which follows every monitor-mode geometry change (#89).
5. The key introduction's preference is read from
   `<climon2 home>/monitor-preferences.json` (`HomeLayout::monitor_preferences`,
   schema `climon2_core::preferences`). A missing file saves nothing; an
   unreadable or malformed one means not suppressed and keeps its reason
   (`cannot read monitor-preferences.json: …` or `monitor-preferences.json is
   malformed: …`) for the screen. It never refuses the monitor.

Every failure prints one escaped line, with the general-log suffix once that log
exists, and exits 125 before raw mode, the alternate screen or any OSC output.

On Windows the monitor has no host rules of its own. Its host input is the
platform terminal seam's `Input`, which there is the client's bridged
console: raw mode, UTF-8 key-down input, window-size records as resizes, and
close, logoff or shutdown as a hang-up that ends it with 129. Its
notification waits watch each instance's event, 64 at a time, and every live
instance is still drained each tick; startup paths cross the native
interface as UTF-16LE (`CM1_NATIVE_WINDOWS`); the preference file gets the
private descriptor; and the descriptor limit admission counts against is
the 2^24-entry handle table. `cargo test -p climon2-monitor` passes on
Windows; the CLI's `tests/monitor_windows.rs` drives the packaged monitor.

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
  hint `Press <prefix> then x to exit.` with the configured prefix; while
  the initial collection lasts, the startup progress takes the empty
  state's place (below, #266). Hiding the
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
  the session list`, `… j  Jump to the session that needs attention`,
  `… ?  Show this dialog`, a blank line, the selector's
  keys under `In the session list:` (owner decision on #113: the footer names
  only select, cancel and help, and help lists the rest; see
  [On-screen strings](#on-screen-strings)), a blank line, `[ ] Don't show
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
  become a numeric-mode key. Bracketed pastes (data until their end
  marker, which discards a pending prefix and leaves none), SGR mouse
  reports, focus reports and host replies bypass shortcuts; pastes, mouse
  and focus are routed as [below](#paste-mouse-and-focus-111), and host
  replies reach no application. The monitor's rule for host replies (confirmed
  by the owner on #109): OSC, DCS, APC and PM strings; CSI sequences with a
  `?`, `>` or `=` marker, an intermediate byte, or a `c`, `n`, `t` or `y`
  final; and any control sequence longer than 64 bytes. CSI `R` stays a key
  (modified F3 sends it, and the monitor never asks the host for its cursor
  position). Prefix+s, x, j and ? (lowercase) are the global
  actions from every screen, the selector and the too-small host; prefix+j
  is recognised and never forwarded, and jumps as [below](#viewing-acknowledgement-and-attention-114). A
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
  reaches neither. When a key closes the last modal in the middle of a read,
  a frame is painted before the rest of that read is routed (owner decision
  on #109), so a key typed straight after Escape reaches the uncovered
  session; input still reaches only a painted, visible session, and nothing
  is kept beyond the read. A doubled prefix is one literal prefix there too, so with
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
  `the host terminal closed`. A read failing with EIO is the end of host input
  too: Linux fails reads with it while it is still hanging a terminal up.

## Sessions (#106)

- **Discovery** (`discovery.rs`) runs on its own thread through
  `climon2_discovery::Continuation`: the first pass at once, each later one a
  second after the previous one ended, so passes never overlap, bounded by
  `scheduling.probes` and `monitor.limits.discovery` (`pass_ms`,
  `batch_records`, `batch_bytes`, `metadata_file_bytes`, `metadata_depth`;
  `discovery.stall_ms` is not applied: an open owner question on #120). The pass deadline runs from the
  pass's start once it has reached an entry or started a probe. Opening the
  selector requests nothing of discovery, so it can neither start a pass
  early nor overlap one. Each batch reaches the UI thread as it is probed. It never deletes metadata.
  Invalid metadata is logged and adds no row; an incomplete pass shows
  `Discovery incomplete: <why>` under the body until a pass completes. A
  pass cannot be interrupted, and it changes nothing; only shutdown joins
  the thread, once its current pass ends (#118).
- **The registry** (`registry.rs`) is pure and the single owner of row state,
  keyed by machine and full session ID ([remote sessions](#remote-sessions-in-the-selector-502)): the discovery outcome and metadata, the connection
  status with the time the monitor entered it, the latest nonempty daemon
  title, and from its client the latest daemon status (`status-v1`: state
  and the daemon's stored change time) and the final outcome (exit code or
  signal), with the time it sorts by: the daemon's terminal status change
  time when it reported one, otherwise when the monitor learned it. A final
  outcome survives disconnection and is never replaced; a daemon status
  outlives its connection only as stale detail, which neither labels nor
  sorts the row. The monitor's own enclosing session (`CLIMON2_SESSION_ID`)
  never gets a row or a client (owner decision on #110): it is not listed at
  all. A live session with no client gets exactly one client request.
  Discovery never changes a client's connection. A session proven gone
  (stale) drops its row unless a client was ever connected to it, and a
  stale session nobody knew adds no row; a row that was connected stays,
  with its history (owner decision on #106).
  The status column is the effective status, Needs attention,
  Acknowledged, Running or Idle; Succeeded for exit zero; Failed with the
  exact code or signal, `Failed (exit 143)` or `Failed (signal 15)`; or, for
  a row with no current status and no final outcome, Starting (also a
  client opened with `status-v1` until its Initial metadata), Unavailable
  (uncertain, failed, or connected without `status-v1`) or Disconnected (its
  connection ended). The library rejects
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
- **The client supervisor** (`supervisor.rs`) has its own thread. It decides
  when each instance starts, stops and is retried; running one instance is
  [client embedding](../../GLOSSARY.md#sessions)'s part, the
  [`climon2-embed`](../climon2-embed/README.md) crate: the shared loader, the
  owner thread, the native calls behind requests and deliveries, the input
  stream, the backlog and stall accounting and admission's arithmetic. It loads
  each release once through `climon2_platform::library::Loader` on a worker
  thread (at most `scheduling.loads` at once; a session waiting on its release
  holds no start slot) and starts clients fairly, at most `scheduling.starts`
  at once until each opens or fails, each once admission reserves it
  ([below](#admission-and-fair-scheduling-120)). Each instance has an owner
  thread, with the stack admission reserved, that creates it with the
  effective `monitor.limits` (`client_bytes` is the library's share of
  `memory.client_bytes`, #119), both client keys and the log
  `logs/<session ID>/library-<stamp>-<pid>-<generation>.log`, submits monitor Open (request
  1, geometry 0x0; `daemon-wire-v1` and `monitor-v1` required, `title-v1`,
  `status-v1`, `notification-v1`, `progress-v1` and `focus-v1` optional),
  publishes its handles, runs it, and destroys it once the supervisor has
  released them. The supervisor waits up to a 20 ms tick on the borrowed
  notification descriptors, then drains every live instance until it is
  empty (so one whose descriptor lookup failed still ends), releases every
  delivery at once (`climon2-embed`'s `instance.rs`, read by `supervisor/client.rs`), and reports Open's outcome
  (with whether `status-v1` was selected), titles from Initial metadata and
  `CM1_TITLE`, the status or final outcome from Initial metadata,
  `CM1_STATUS` and `CM1_FINAL`, and the end. A refused release, a failed
  create, Open, notification lookup or drain, a run that returns without
  terminal evidence, or a client that ends before opening makes the row
  Unavailable; one that ends after opening, Disconnected. A failed instance
  is asked to stop and drained until its stream is over; its handles are
  then released, its start slot is free (only instances still creating or
  opening hold one), and its owner destroys it. Whether it is retried is
  [#116's recovery](#disconnection-read-only-history-and-passive-recovery-116).
  The general log records `client
  library loaded`, `client library loading slowly`, `client library
  resource-blocked` or `client library refused`, `client starting
  session=<id>`, `client published`, `client failed`, `client stopped`,
  `client ended`, `client retry scheduled session=<id> delay_ms=<n>` and
  client diagnostics, content-free.
- **The selector** (`selector.rs`, `search.rs`; #106, #113): prefix+s,
  unless the modal key introduction is open, overlays the rows below the top
  row above the toasts' four-row lane at the bottom of the host (owner
  decision on #115), one cell in from each edge, with a captionless double
  border. Its
  first line is the search, always focused, with the host cursor at its
  insertion point (scrolled so the cursor stays in view); a single separator
  `╟──╢` joins the side borders below it. Then the rows the search matches,
  in row order: each is its status, padded to the widest status of any row,
  a space and its label, ending in `…` without splitting a wide character
  when it does not fit (owner decision on #112); the highlighted row is
  reversed across the list. With `monitor.show_key_hints` on, the last line
  inside the border is the footer, `Enter: select  Esc: cancel  <prefix>
  then ?: help` (owner decision on #113: only these three), with
  `<prefix> then <prefix>` for Enter when Ctrl+M is the prefix and no Escape
  hint when Ctrl+[ is, ending in `…` when too long; with hints off its row
  goes to the list. Search, separator, footer and list each draw only while
  the box has room, and the overlay never resizes an application.
  It opens with an empty search on the selected session, scrolled into
  view, or else on the first row; prefix+s while open does nothing.
  - **Search** matches a row when every search character occurs in order in
    its displayed status, a space and its label, both lowercased (so case
    is ignored beyond ASCII), as `fuzzy-matcher`'s `SkimMatcherV2` finds
    them; spaces count, scores are never used, and there is no cutoff.
    Typed characters insert at the cursor once their UTF-8 is complete
    (control characters, Tab included, and invalid bytes insert nothing);
    Backspace (DEL or BS) deletes before it; Left, Right, Home and End
    (`ESC [ H`/`F`, `ESC O H`/`F`, `ESC [ 1 ~`/`4 ~`) move it. A bracketed
    paste is inserted at its end as one line: CR, LF, CRLF and tab each one
    space, other control characters and invalid UTF-8 discarded; it never
    acts as keys. The search holds at most `ui.search_bytes` UTF-8 bytes: a
    typed character or paste that would exceed them is refused whole, and
    the footer's row (lent to it while hints are off) says `Search is
    limited to <N> bytes` until the next key or paste.
  - **Highlight.** A search edit keeps a highlight that still matches,
    otherwise highlights the first match (or nothing), and scrolls back to
    the list's start as far as the highlight allows; clearing the search
    never restores an older highlight. Up and Down (`ESC [ A`/`B` or
    `ESC O A`/`B`) move it a row, PageUp and PageDown (`ESC [ 5 ~`/`6 ~`) a
    list's height, never wrapping, scrolling to keep it in view; while
    nothing is highlighted, each highlights the first match. Rows that
    reorder keep the highlight on the same row; when an update removes the
    highlighted row or it stops matching, the highlight is gone for good,
    and Enter does nothing until the user navigates or clicks.
  - **Mouse** (SGR reports, `mouse.rs`): a left press on a listed row only
    highlights it; one elsewhere inside the box does nothing, and one
    outside it cancels the selector (owner decision on #113). Each wheel
    notch scrolls the list one row, as far as it goes, without moving the
    highlight. Releases, motion and other buttons do nothing.
  - **Keys** come from the input router (#109). A bare Escape closes it when
    the next key arrives (the router has no timer); a cursor key split
    across reads anywhere never closes it, and the key after the Escape is
    routed once it has closed (INP-02). With Ctrl+[ as the prefix a bare
    Escape waits as the prefix and cannot cancel (the accepted limitation).
    Cancelling, by Escape or a click outside, changes neither the selection
    nor control. Opening it never acquires anything. Enter (a typed CR)
    acts on the highlighted row's condition (below, #108 and #110), and
    forward Delete (`ESC [ 3 ~`) asks to terminate or remove it
    ([below](#delete-in-the-selector-117)). Prefix+?
    opens the key introduction over it, which takes every key, paste and
    click until it closes, leaving the selector's search and highlight as
    they were. While rows exist, nothing is selected and the selector is
    closed, the body reads `No session selected.` above the key hint.
- **Exit** is the coordinated shutdown [below](#exit-and-coordinated-shutdown-118).

## Selection, control and painting (#108)

- **Selection.** Initial selection selects a row at startup (below, #266);
  Enter selects explicitly. Once a session is selected, no arrival moves
  the selection. Selecting submits
  Acquire through the session's client (`supervisor.rs` numbers each request
  from 2, after Open, and matches each answer to its request). A request
  enters the client's pending ledger, and takes its ID, only once the
  library accepts it; a refused one is answered at once and consumes no ID.
  The ledger holds at most `queues.request_records` + `queues.operations`
  requests (every request the library may have accepted), and one more is
  refused at once. A session selected earlier keeps control in the
  background when another is selected. Enter on other rows is #110's
  (below), and on disconnected and final rows
  [#116's](#disconnection-read-only-history-and-passive-recovery-116).
- **Terminals** (`terminal.rs`, pure): each session has one `climon2-vt`
  model, kept by session ID for the whole run and bounded by
  `memory.client_bytes` and `history.max_rows`: the model holds what the
  library's share leaves of that budget, never a second allowance (#119,
  closing the gap recorded on
  [#120](https://github.com/jackgeek/climon2/issues/120)). Its client's
  records drive it: a grant
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
  (`Terminal::input`, which the input router checks). Painted is decided before each
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
  rendition (each underline style with its own SGR 4 subparameter, `4:2`
  double, `4:3` curly, `4:4` dotted, `4:5` dashed and plain `4` single,
  carried beside the Ratatui frame, whose cells hold only whether they are
  underlined; SGR 58
  only on an underlined cell with a colour of its own, in the T.416 colon
  form, since SGR 0 resets it and a host that does not know it skips the
  colon form whole). Its cursor is shown with its
  DECSCUSR shape and blink where the application left it, unless hidden, off
  the host, scrolled out of view or covered by the selector or key
  introduction; never in the top row. While its restoration is staged the
  last applied frame stays; before any, the canvas is blank. While the
  application's synchronized update (`CSI ? 2026 h`) is open, the canvas
  painted before it stays, cursor included, for the same session at the
  same host size, while the top row and overlays are drawn as ever; the
  update shows whole once it ends, which the session daemon makes happen
  within vte's 150 ms timeout.
- **History** (#112). A vertical wheel notch over the selected
  application taking input, when it does not report the mouse, scrolls
  its normal screen's view one row into the model's history (up goes
  back; one row a notch, owner decision 2026-11-16), as far as the oldest
  row kept, or one row further once older rows were discarded: there the
  glossary's truncation notice, `[climon2: older session scrollback was
  discarded]`, sits above the oldest kept row (owner decision
  2026-11-16). The canvas then shows that many history rows above the
  screen's top rows, and the cursor moves down with them, hidden once
  scrolled out of view. Output keeps the view anchored on the rows it
  shows, counted per engine step
  ([climon2-vt](../climon2-vt/README.md#read-only-views)); when they are
  evicted it moves to the notice above the oldest row kept, never to
  live. While scrolled back the top row's control slot says `History`
  (owner decision 2026-11-16). Any application input (typed keys, mouse
  reports, a paste) returns the view live and reaches the application;
  scrolling alone changes neither application focus nor input. On the
  alternate screen, which never shows normal history, the wheel is a
  cursor key (up or down, in the application's cursor mode) only while
  the application has alternate scroll (1007) on, and does nothing
  otherwise; the normal view keeps its place until the application
  returns. An applied restoration replaces the model whole, so its view
  is live. The monitor draws only on its own alternate screen, so nothing
  reaches the host's native scrollback.
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
  from its left edge, clipped. A label that overflows its cells is
  truncated statically to end with `…` within them, never splitting a
  wide character, and so is a selector row: nothing marquees (owner
  decision on #112, 2026-11-16). A request
  refused under a restoration since superseded is not an error; a Resize
  refused so is sent again under the current one.
- **Geometry.** The selected controller is asked for the host width by its
  height less the top row: after every grant, on selection and on each real
  host resize, once per control period and size. Background controllers get
  none. A host under four rows or five columns (owner decision, 2026-11-14)
  shows only `Terminal too small` in its top row and sends no resize; clients,
  control and models carry on, global keys still act, and a larger host
  resizes and paints the session again.

## Initial selection (#266)

- **Collection** (`initial.rs`, pure). The monitor collects statuses until
  the first discovery pass has ended and every client started for it has
  left Starting (its Initial metadata reported, it failed, or it is
  connected without `status-v1`) or is `deadlines.start_ms` past the moment
  the monitor started it, measured on the monotonic clock. Clients started
  after the first pass are not waited for. While it collects, the body
  reads `Connecting to N sessions (M ready)` (`1 session` for one, owner
  decision on #266): N clients started for the first pass, M of them no
  longer starting. A resource-blocked row, Awaiting resources (#120), is
  settled: collection never waits for it, and while it awaits resources it
  is never eligible.
- **Selection.** When collection ends, the monitor selects and acquires the
  first eligible row in row order: connected, its status known and not
  final. Not-connected rows, a connected row without `status-v1`, final and
  disconnected rows are never eligible, and the own session has no row. An
  open selector or key introduction stays open over it, its highlight
  unmoved. With nothing eligible then, nothing is selected, and the first
  row to become eligible later, a new arrival or a row that was still
  Starting, is selected and acquired once (owner decision, 2026-11-16).
  Any selection, automatic or explicit (Enter on any row, even read-only),
  ends initial selection: nothing is ever chosen automatically again, and
  the choice stays pinned while it starts.
- **Uncertainty.** When collection ends with a client still starting,
  `Discovery incomplete: some sessions are still starting` shows for as
  long as one of them still is: in the body, below any incomplete discovery
  pass, while nothing is selected, and in the top row's error slot while a
  row is selected, unless that session shows its own error (owner decision
  on #266, 2026-11-16).

## Switching, revocation and taking control back (#110)

- **Enter by row condition** (`registry.rs`, one `Kind` per row): a
  connected row not known to be final, a connected Unavailable row
  included, is selected and acquired; a Starting row, or an Unavailable row
  with no connected client (uncertain or failed), is selected but never
  acquired; disconnected and final rows are selected read-only (#116,
  below). The monitor's own session has no row to press Enter on. An
  Awaiting resources row is selected read-only like a Starting one, with its
  condition, any retained cache and its overlay, and never acquired; once
  admitted and connected it shows the placeholder until the user acquires
  (#120).
- **Switching** keeps control of the session switched away from in the
  background: its client stays, its model stays live, and it gets no
  input or resize. Enter on it again acquires it, which the daemon answers
  without a new restoration while the monitor still controls it, so its
  model shows at once.
- **What the canvas shows.** A selected session with a live or frozen view
  shows it. Otherwise a connected session the monitor neither controls nor
  is acquiring (revoked by another client, failed acquisition, or a row
  selected without a client whose client connected) shows only the placeholder,
  centred: `This session is being viewed elsewhere.` once another client
  revoked the monitor's control on this connection, otherwise
  `This session is not controlled here.` (#400), and `Press <key> to
  take control` while a take-control key is bound and
  `monitor.show_key_hints` is true; never its cached output. A row selected
  without a connected client shows `No terminal content is available.` and its condition
  (`Starting` or `Unavailable`), unless it has retained history (#116,
  below). A connected session the monitor is acquiring, or before its first
  restoration, shows a blank canvas.
- **The take-control key** (`client.take_control_key`), unprefixed, acquires
  the selected session while it shows the placeholder, and is consumed
  either way when nothing takes input, also on a host too small to show
  the placeholder (owner decision on #110); inside the selector or key
  introduction it does nothing, and for the ready controller it is
  ordinary input. Prefixed it is two literal bytes (#109).
- **Automatic only once.** The monitor acquires on Enter, the take-control
  key and the attention jump (prefix+j) to a session it does not control.
  The one automatic Acquire is initial selection's, at most once (#266).
  A grant the daemon makes by itself (promotion or hand-back) is applied
  and Ready is sent, in the background without selection, input or resize; the selection is kept through revocation,
  completion, loss and failed acquisition, and input never goes to another
  session.

## Disconnection, read-only history and passive recovery (#116)

- **Classifying an end** (`supervisor.rs`, `supervisor/client.rs`). Each
  instance that is over is retried as its terminal evidence's retry says
  (none, passive or permanent), or passively when the monitor stopped it
  itself (a failed notification lookup or drain, a run that returned
  without terminal evidence). Open's refusal ends the stream by itself, so
  its evidence decides. A refused release is retried when a later load may
  succeed (the library missing, missing a dependency or of another
  architecture, a contained bootstrap failure, or no load thread), and is
  otherwise permanent; create's refusal is permanent for invalid arguments,
  an unsupported revision or limit or another release, otherwise passive.
  Nothing is retried once the client delivered the command's final outcome
  (the library also says none then), or once destroy failed, when worker
  cleanup cannot be confirmed: that row says `cannot confirm the client's
  cleanup: <error>` and is never replaced; its unconfirmed cleanup makes
  the monitor's exit a fail-stop (#118, below).
- **Passive retry.** A retry waits until the old instance's deliveries and
  handles are released, its run has returned and its owner has destroyed it
  and been joined, then a backoff (`supervisor/backoff.rs`): from
  `retry.initial_ms`, doubling with saturation to `retry.cap_ms`, each delay
  with fresh uniform jitter of up to `retry.jitter_percent` either way,
  clamped to the cap, with no attempt-count cutoff; an open client starts
  the next backoff afresh. The new instance has the session's next
  connection generation (the first is 1) and its own library log; it is
  created with the same startup, never replays a request (input, Acquire,
  Ready, a paste, Kill) and never acquires: the row shows the non-controller
  placeholder once it opens, until the user acquires. A recoverably refused
  release is loaded again, once, when a waiting session's retry is due. A
  session discovery proves gone (its daemon not running) is never retried
  again (owner decision, 2026-11-16). Shutdown starts no retry. A retry
  that is due waits for its start like a first start, in its own lane, and
  through admission (#120); #119 reuses this path for a
  session dropped at its memory limit.
- **Rows** (`registry.rs`). A row learns its recovery with its end: retrying
  with its diagnostic, or rejected for good with why. A retry's new
  instance makes it Starting, then Unavailable if it fails before opening
  or Disconnected if it ends after; it is never a first start, so it never
  holds initial selection. A new connection ends recovery and drops the old
  connection's stale status, so only the new client's reports label the
  row (#107's review). A known final outcome survives every loss, and a
  loss alone is never shown as completion. Discovery proving the session
  gone marks a not-connected, not-final row so for good: `Session no
  longer exists`, the state #117's Delete reads to offer Remove alone.
- **Enter** on a disconnected or final row selects it read-only, never
  acquiring, as on Starting and Unavailable rows with no connected client.
- **What a not-connected row shows** (`lib.rs`, `screen.rs`). A row neither
  connected nor known final shows its terminal's retained history
  read-only: the model as its last applied restoration and the output
  after it left it, from any generation, without a cursor, with no input,
  resize, acquisition or acknowledgement; the wheel scrolls it a row a
  notch, with `History` in the control slot. Without one (no restoration was
  ever applied) it shows `No terminal content is available.` and its
  condition. Over either, exactly `Reconnecting...` while a retry is
  scheduled or running, `Not reconnecting: <why>` once rejected for good, or
  `Session no longer exists`; the top row's error slot carries the row's
  diagnostic (owner decision on #116), unless its terminal has its own
  error. `Session no longer exists` shows there even then: the terminal's
  error may only be a request the lost connection cancelled. A final row shows only the view frozen at its end (#108), never an
  earlier period's cache, otherwise no content and its outcome. A
  connected row, revoked or reconnected, still shows only the placeholder,
  never its cache. Global actions work from every one of these screens.
- **Rejected updates** (#119; owner decision 2026-11-16). An update a
  session's model refuses, live output or a restoration, shows `terminal
  model over its limit: <needed> bytes needed, <limit> allowed` in the top
  row's error slot and drops its client: the supervisor fails it, the row
  is Unavailable with that diagnostic, its model keeps the page its
  accepted output produced, read-only like any retained cache, and the
  client is retried passively through this path. Other sessions go on.

## Admission and fair scheduling (#120)

- **Admission** (`climon2-embed`'s `admission.rs`, pure). Every reservation is
  admitted whole or not at all against aggregate accounting: memory within
  `memory.total_bytes` less the monitor's parser, and descriptors within the
  process's soft descriptor limit (read afresh each time, so a raised limit
  admits more) less `resources.descriptor_headroom`. Each holder holds one
  reservation, so a release counts once however many clients use it.
  - Before the dynamic loader opens a release: its configured
    `resources.release_bytes` and `resources.descriptors_per_load`, and the
    load thread's stack (`resources.stack_bytes`, which that thread gets),
    held until the load returns. A refused release whose handle the loader
    never opened (missing, missing a dependency, of another architecture, or
    no load thread) is freed; any other handle and its reservation stay
    until exit.
  - Relaxed pre-load qualification (confirmed 2026-11-08): once loaded, the
    release's qualified requirements come from its bootstrap. Qualified
    workers above `resources.max_workers_per_client`, or a qualified instance
    allocation the library's share of `memory.client_bytes` cannot hold
    beside its queues and one frame, reject the release explicitly and for
    good, before any create. Otherwise its reservation is raised to the
    greater of configured and qualified before any create; a raise that does
    not fit leaves it resource-blocked with its handle loaded.
  - Before each create: the library's share of `memory.client_bytes`, the
    owner's stack and each qualified worker's stack (each the greater of
    `resources.stack_bytes` and the qualified stack) and the greater of
    `resources.descriptors_per_client` and the qualified instance
    descriptors, held until the instance is destroyed and its owner joined,
    so partially started instances stay counted.
  - The bytes admission holds are counted beside the parser, unapplied
    reports and every model when history is kept within
    `memory.total_bytes` (#119's pressure policy).
- **Resource-blocked rows.** A session admission refuses, or whose release
  it refuses, shows `Awaiting resources` in the not-connected group, sorted
  by when it entered that condition (a changed reason keeps that time), with
  what is short in the top row's error slot. It is reconsidered at most once
  per `retry.admission_ms`, never in a busy loop; freed capacity admits it at
  its next reconsideration.
- **Fair starts** (`supervisor/queue.rs`, pure). First starts run newest
  created first, then ascending session ID; after
  `scheduling.newer_start_burst` newer starts the oldest waiting eligible one
  has its turn. Passive retries wait in their own lane in arrival order, and
  the lanes take turns when both have a session ready (owner decision on
  #120). A session whose release is still loading, or that waits out its
  admission interval, is passed over and keeps its place, so a stalled
  release holds up no other release's sessions. A Kill-only instance
  (#117's Delete) is admitted ahead of the queue (owner decision on #120).
- **Slow loads.** A load running past `deadlines.load_ms` is logged once as
  `client library loading slowly` and its waiting rows, still Starting, show
  `library loading slowly` in the top row's error slot; it is never aborted
  (owner decision on #120). On glibc a stalled load also holds up every new
  thread (the loader's TLS lock), so meanwhile no new client or discovery
  probe starts, even of a loaded release; running clients, the selector and
  exit carry on. Whether to keep threads ready for that is an open owner
  question on #120.
- **Not yet:** `discovery.stall_ms` is not applied; mandatory identity
  exhaustion has no coordinated failure (#118's fail-stop does not cover
  it); model
  growth, renderer, search, toast and identity state are not reserved at
  admission; and the operating system's thread limit is met only as a
  failed thread start (Unavailable, retried passively).

## Delete in the selector (#117)

- **The confirmation** (`delete.rs`). Forward Delete (`ESC [ 3 ~`) alone,
  never Backspace (owner decision on #117), opens it over the selector on
  the highlighted row, unless a Delete of that row is already under way:
  then it opens nothing, so a repeated Delete never sends a second Kill.
  It keeps that session's ID whatever the rows do. Its box, titled
  `Delete session`, centred below the top row, shows the row's label and
  status as they are now, then `[ Cancel ]`, focused first and reversed,
  and `[ Terminate and remove ]`, or `[ Remove ]` once the session is final
  or discovery proved its daemon gone (owner decision on #116). Tab, Left
  and Right move the focus, Enter activates it and Escape cancels (owner
  decision on #117); other keys, pastes and mouse reports do nothing, so
  nothing reaches a session, and the host cursor is hidden. Should
  discovery drop the row meanwhile (a session never connected, proven
  gone), the dialog closes. Inside it prefix+x exits the monitor, prefix+j
  cancels it and jumps (#114), and prefix+s and prefix+? do nothing
  (owner decision on #117). Either decision returns to the selector.
  Cancelling sends nothing and never acknowledges attention (Confirmed
  2026-11-08). The key introduction lists `Del  Delete the highlighted
  session`, `Del (fn+⌫)` on macOS (owner decision on #117). The monitor's
  own enclosing session has no row, so it is never deleted (owner decision
  on #117, amending spec #73).
- **Confirming** takes the offer as it stands then, so a session that
  became final while asked is only removed, and the row shows
  `Terminating` or `Removing` in its status column, in its place, until it
  goes. Other sessions stay usable throughout.
- **Kill** (`supervisor.rs`). Terminate and remove submits one Kill,
  requiring no family as `c2 kill`'s does, through the session's open
  client, or, when it has none (a client that failed, is starting, is
  waiting for a retry or never started), through a Kill-only instance: no
  Open, its Kill as request 2, never retried, and its row learns nothing
  of it but the Kill's outcome. Kill-only instances start ahead of every
  waiting client in the start lane: admission (#120) reserves each before
  any queued session, and one it refuses is reconsidered at most once per
  `retry.admission_ms`. While a Kill is wanted or pending, the session's
  waiting start or retry is held. A Kill's termination result, or a
  known-final refusal's, is the row's final outcome.
- **Removal.** The session is removed only once its end is known (from
  the Kill, or the client's own Initial metadata or Command final) or it
  needed no Kill, and no instance of it is left: each is asked to stop,
  its deliveries and handles released, destroyed and its owner joined.
  Then the row is dismissed for the run (the registry's run-local
  dismissal: discovery never lists the session again, whatever it finds,
  and nothing is persisted), and its terminal model and retained history
  are discarded; nothing else is deleted, its logs included. A selected
  row's selection falls to the row now at its place, else the one before
  it, else none, and the open selector's highlight likewise among the
  rows it lists; neither acquires anything or routes input.
- **Short of removal.** A refused Kill ends the Delete: the row is back to
  its status with `Termination refused: <why>`, a retry it held is
  started again, and Delete may ask again. A failed or lost one (or a
  client ending before its Kill was answered) keeps the row Terminating
  with `Termination not confirmed: <why>`; the Kill is never sent again,
  and only a passive client (the open one, or a retry after a backoff)
  learns the outcome. A Delete whose instance's cleanup cannot be
  confirmed stops there with `Termination not confirmed: cannot confirm
  the client's cleanup: <error>` (the fail-stop is #118's). The top row's
  error slot shows these first, before the terminal's own error and the
  recovery diagnostic.

## Paste, mouse and focus (#111)

- **Paste** (`router.rs`, `climon2-embed`'s `stream.rs`). The router passes a
  bracketed paste on as data, read by read, holding back only bytes that
  may begin its end marker until the next byte decides, so a paste is
  exact however the host splits it. It goes to the target that took
  application input when it began (the selected, controlling, ready,
  painted session with no modal and room on the host), with the
  application's own markers (`ESC [ 200 ~`, `ESC [ 201 ~`) only if the
  application had bracketed paste on then, and otherwise as the payload
  alone. A prefix pending when a paste begins reaches that target as one
  literal prefix first; with no target taking input the prefix and the
  paste are discarded. An open modal owns a paste and discards the prefix:
  the selector's search takes the paste as one line (#113), the key
  introduction discards it. A paste a modal covers part-way is cut (below),
  and the search ignores the rest of it, which it never saw begin. The supervisor submits a paste's parts as input in
  chunks of at most `transfer.chunk_bytes`, its markers in chunks of their
  own; once the library refuses one (owner decision
  on #111), the top row's error slot says `Paste truncated`, the rest of
  that paste is discarded, and an application that received any of its
  start marker is owed the rest of it and the end marker: submitted after
  every answer, when the library may have room again, and before any later
  input to that session, until the library accepts it.
- **Target changes.** A change of the selection, of whether a modal covers
  the canvas, or of the selected session's control period or restoration
  (a grant, a new restoration, revocation, the end of its client), seen
  between two reads, discards a pending prefix. A paste or a mouse gesture
  belongs to the target, and the authority, that took input when it began;
  once that target stops taking input for any reason (those changes, a
  too-small host), the rest of it is discarded through its end marker or
  release, even if the target takes input again, and never goes to another
  session, period or restoration. A bracketed paste cut short so leaves
  its application inside the paste, so (owner decision on #111) while the
  control period that received it lasts, its terminal owes the end marker
  and submits it, before anything else, under the session's current
  authority as soon as that restoration is ready: at once for a
  too-small host, after a new restoration once its Ready is accepted.
  When control changes hands nothing is owed: the daemon itself ends a
  bracketed paste it saw begin. A paste cut short after the input queue
  truncated it owes no second end marker: what the truncation still owes
  goes under the current authority instead.
- **Mouse** (`mouse.rs`). SGR reports are hit-tested: a press inside the
  selected session's canvas below the top row, within its geometry and the
  host, begins a gesture the application owns; one anywhere else (the top
  row, a margin, any report while a modal is open or nothing takes input)
  begins a gesture the monitor keeps; while the selector is open, its
  reports go to the selector (#113), and otherwise nowhere. Every later report of a gesture, until its release, goes to
  its owner, at the nearest application cell when it strays, or is
  discarded, with the rest of the gesture, once its owner stops taking
  input. Motion and the wheel outside a gesture go where they land. An
  application receives an event translated one row up, only as its
  reporting mode asks (1000 presses, releases and the wheel; 1002 also
  motion with a button held; 1003 any motion) and in its encoding (SGR;
  UTF-8, coordinates up to 2015; or single bytes, values up to 223, where a
  release is button 3), and nothing it cannot encode. The host mirrors the
  tracking level of the application whose canvas is shown uncovered (owner
  decision on #111): click-only (1000), button-event (1002) or any-event
  (1003), switched in `screen.rs` beside the keypad mode by resetting the
  old mode and setting the new one. While that application reports nothing,
  a modal covers it, another session or no session is shown, or the host is
  too small, the host is in the monitor's own button-event tracking (1002),
  as it entered; leaving resets every mode. The wheel over an application
  that does not report the mouse scrolls its history or sends cursor keys
  (History above, #112).
- **Host focus.** The host's focus reports bypass keys and set whether the
  host has focus, assumed until it reports otherwise. A host that never
  reports focus cannot be detected: its focus is always assumed, and nothing
  on screen says so (owner decision on #111). Viewing, which host focus
  ends without dropping control, arrives with #114.
- **Application focus.** A session is keyboard-focused while it is selected,
  no modal covers it and the host has focus; every other session is not.
  Before each frame, each ready controller (Ready accepted) whose client
  opened with `focus-v1` is sent Focus for a desire it was not yet sent
  under its current restoration: so opening the selector or the key
  introduction, switching away and the host's focus out send focus out to
  the session that had focus, closing them or the host's focus in send
  focus in, and a newly ready restoration (a grant, a hand-back, a new
  restoration) sends the current desire once, never an earlier one. The
  daemon forwards a changed focus only while the application has focus
  reporting on, and the current one once when it turns it on. A session
  whose client or daemon lacks `focus-v1` is sent no Focus and stays usable.
  Like a host that never reports focus, this limitation is documented here
  only; nothing on screen says so (owner decision on #111).

## Viewing, acknowledgement and attention (#114)

- **Viewing.** The monitor views the selected session only while it
  controls it with its restoration applied and Ready accepted, the frame
  just painted shows it uncovered by any modal on a host with room, its
  view is live (not scrolled back into history), the host has focus
  (assumed until the host reports otherwise) and no toast covers any cell of
  its canvas (#115). After each painted frame, each
  session whose client opened with `status-v1` is sent Set viewing under
  its current authority when that changes; a control period starts not
  viewing, as the daemon's does. Viewing never acknowledges by itself.
- **Coalesced reports.** Focus and Set viewing are coalesced per session
  (owner decision on #114): while one of a kind is unanswered no other of
  that kind is sent, and once it is answered, accepted or refused, the
  latest desire goes if it differs from the one last sent. Toggling the
  selector on a slow daemon so puts at most one Focus and one Set viewing
  per session in the library's operation queue (`queues.operations`), not
  one per change, which had filled it and got typed keys refused with
  `queue_full`. The
  end of the client forgets an unanswered one; a revocation does not, as
  its answer still comes. Acknowledge is not coalesced: one per explicit
  action, and a deferred one waits until Set viewing true was sent in the
  current control period, so the daemon never refuses it as not viewing
  while a coalesced viewing report is still held back.
- **Acknowledgement.** Enter on a connected row, prefix+j, and closing or
  cancelling the selector back to the selected session (Escape or a click
  outside) each defer one Acknowledge for that session. It is submitted,
  after the viewing report, once the session is viewed: an action made
  while the host is unfocused, the view scrolled back, the host too small
  or the session not yet ready acknowledges once that clears (owner
  decision on #114). It is cancelled when the target context changes
  first: another selection, a modal opening, control lost or another
  control period. An Acknowledge a newer restoration of its period
  superseded (a resize) is deferred again. Closing help, host focus-in and
  visibility never acknowledge.
- **prefix+j.** From any screen, the connected row needing attention with
  the newest daemon change at the keypress (the lower full session ID on a
  tie); acknowledged, final, disconnected and not-connected rows never
  qualify. It closes the selector and the key introduction, selects the
  session, acquires it only when the monitor neither controls nor is
  acquiring it, returns its view live and defers its acknowledgement. With
  none, the selection stays and `No sessions need attention` shows in the
  top row's error slot, or the body when nothing is selected, for 3 seconds
  (owner decision on #114). Uppercase J and a pasted prefix+j are input.
- **Host attention.** After each painted frame the host hears, through a
  root OSC 7501 report ending in `ESC \`, whether any connected row needs
  attention: `state=blocked:kind=question:app=climon2-monitor` while one
  does, otherwise `state=working:app=climon2-monitor`, the first state and
  then changes only, with no heartbeat or query, and never a session's own
  report. Once a started monitor has released the host on exit, it writes
  the root `state=clear`, before main prints any diagnostic.

## Toasts and bell (#115)

`toast.rs` is pure; `lib.rs` feeds it what the registry announces.

- **Eligibility.** Each status needing attention announces its episode ID;
  a final outcome is announced once, and only when the session's Initial
  metadata knew no final outcome, so a final already known at startup or
  first connection never toasts (owner decision on #115). Each episode and
  the final outcome toast at most once per session for the whole run,
  whatever repeats or menus bring. An announcement toasts at once unless
  this monitor viewed the session when it arrived: an episode first seen
  while viewed toasts once a menu (the selector or the key introduction)
  opens, while it still needs attention, and every frame with a menu open
  makes such episodes eligible; a final outcome learned while viewed never
  toasts. Another client's or monitor's viewing suppresses nothing.
- **Queue.** The queue holds three toasts, the one shown among them, in
  order; a fourth is replaced by one `More events occurred. Check sessions
  list` toast after them, which every later one joins until it shows
  (owner decision on #115). Nothing fails a connection.
- **Appearance.** One toast shows at a time, flush with the host's
  bottom-right corner inside the four bottom rows the selector leaves free:
  a yellow single-line border around the session's label, as the row showed
  it when the toast was queued, after one blank cell, and, on a
  needs-attention toast while `monitor.show_key_hints` is on, a row
  `<prefix> then j: jump to attention`; each line ends in `…` when it does
  not fit. It is as wide as its widest line plus four cells, at most 40% of
  the host's width; a toast that has no room for one cell of text and its
  rows waits until it has. Over everything else, menus included, the border
  draws clockwise from the top-left corner in 400 ms over a blank inside,
  the text then shows, held for 2,000 ms from when the frame first showing
  it has finished writing to the host (owner decision on #115, 2026-11-17),
  dims for 300 ms, and the border erases clockwise in 400 ms with the
  inside showing through. A host that takes output late so still sees the
  whole hold. Each frame is drawn for the time it is painted, so no
  obsolete frame waits; a resize redraws the toast at once for the new
  host. A wide character the toast's left
  edge would cut is blanked.
- **Dismissal.** A left click on a toast, or prefix+j or Enter on a session
  whose canvas it covers (owner decision on #114), dismisses it: before its
  title has been shown, what was drawn erases at once, as fast as it drew;
  later, its text dims at once and the border then erases. Dismissal never
  selects, acquires or acknowledges.
- **Input.** A toast is nonmodal. While it covers any cell of the selected
  session's canvas, the session is not viewed, so a deferred acknowledgement
  waits until it has gone, yet keys still reach the session. A mouse report
  outside any gesture that lands on a toast is the toast's; a gesture begun
  before a toast appeared keeps its target over it through its release.
- **Bell.** With `monitor.bell` on (the default), one BEL follows the frame
  that first shows each toast's full title, once written, the overflow toast
  included; never per frame, redraw, resize or coalesced announcement, and
  never for a toast dismissed before its title showed. With it off only the
  BEL is skipped.

## Memory accounting, bounded history and cadence (#119)

- **The client budget split.** `memory.client_bytes` is one budget per
  session: the client library's `client_bytes` is its request and event
  queues, one wire frame and its 1048576-byte instance allocation, and the
  session's terminal model may hold the rest (owner decision on #119; the
  split rule is an open owner question). The startup check counts the
  library's share beside the largest session's model and page.
- **Rejected updates.** See the recovery section above: a session whose
  model refuses an update is Unavailable, its last safe cache read-only,
  and its client is retried passively; a connection is dropped once, and
  what it still delivers drops nothing more.
- **Reports in flight** (`supervisor/backlog.rs`; `climon2-embed`'s `backlog.rs` and `stall.rs`). The supervisor copies
  each delivery into reports and releases it at once; each report is then
  charged to its client, by the allocation it holds (capacity, not
  length), until the UI has applied it. A reporting client whose
  unapplied reports fill `queues.event_bytes` is not drained: its records
  wait in the library and nothing is dropped. Once that congestion lasts
  `deadlines.delivery_stall_ms` the client is failed explicitly with
  `delivery congestion lasted past <N> ms` and retried passively. Exit
  drains every client regardless.
- **Work quanta** (`cadence.rs`). The UI applies client reports in fair
  turns: each client for at most `ui.client_quantum_ms`, all of them for at
  most `ui.background_quantum_ms` a turn, the next turn starting after the
  client served last. A report is one bounded step (a display chunk is at
  most `transfer.chunk_bytes`); what is left waits, still charged.
- **Render cadence** (`cadence.rs`). Output alone paints at most `ui.fps`
  frames a second: a frame not due is skipped, never queued, and a later
  step paints the scene as it is then, so obsolete animation frames are
  dropped. A host read paints at once, as do a revealed canvas and a
  resize, so input, the selector and exit never wait for the cadence; every
  frame counts toward it.
- **Bounded history** (`pressure.rs`, `lib.rs`). After each turn, each
  session's history (what its model holds beyond its current model) is
  kept within `memory.history_bytes_per_session` by shortening its own,
  oldest chunk first; then, while all histories exceed
  `memory.history_bytes_total` or what the monitor tracks (its parser,
  every model with its staging, every unapplied report) exceeds
  `memory.total_bytes`, caches of sessions not connected yield a chunk at a
  time, unselected ones least recently inspected first and the selected
  one last. A connected session's history is bounded only by its own
  budgets. No cache gives up the chunk holding its newest page, and a
  shortened history shows the truncation notice above its oldest row.
- **Identifiers.** Connection generations start at 1 and strictly increase
  per session; once a session's are exhausted it is never attempted again
  (`connection generations are exhausted`, rejected for good). Request IDs
  start after Open's and are refused, consuming nothing, once exhausted.
  Neither wraps. With `test-hooks` only, `CLIMON2_MONITOR_FIRST_GENERATION`
  and `CLIMON2_MONITOR_FIRST_REQUEST` start them near exhaustion.
- **Not yet:** the aggregate pressure order is proven over real terminal
  models in unit tests, not yet in the packaged monitor; live models are not refused growth for aggregate
  pressure before allocation (only their own limit and the per-session
  history budget bound them); renderer, search, toast and identity state
  are not yet charged to `memory.total_bytes`; and library `snapshot`
  measurements and total process memory are not yet reported.

## Exit and coordinated shutdown (#118)

Prefix+x and the catchable SIGINT, SIGTERM and SIGHUP take one path
(`lib.rs`, `supervisor.rs`, `shutdown.rs`, `failstop.rs`):

1. Application input, acquisition, discovery and retries stop at once:
   nothing read after prefix+x is routed (keys read before it in the same
   read reach the session first), discovery starts no pass after the
   current one, and the supervisor schedules no start or retry. The
   supervisor learns of the exit once its current round ends: a round's
   drains end a 20 ms tick after its wait, each instance drained at least
   once and each round starting one instance further on, so instances
   whose deliveries never end neither starve the others nor keep it from
   reading the exit (#462).
2. One deadline, `deadlines.shutdown_ms` from the exit request, bounds the
   rest. The supervisor first waits until every input chunk an open client
   accepted is answered, at most `deadlines.operation_ms` and inside the
   same deadline (owner decisions on #109 and #118), then asks every
   instance to shut down at once, again each round, passive, controlling,
   partially started and failing alike, so a slow one holds up no other.
   It keeps draining and releasing deliveries, releases each instance's
   handles once it is over, lets its owner destroy it and joins that owner,
   and joins every release load thread; the UI thread stops discovery and
   joins its thread once its pass ends. The supervisor's waits and drains
   stop at the deadline, as far as the platform timer allows (Windows wakes
   a wait at its scheduler tick, about 15.6 ms), and the UI thread waits a further 200 ms
   handover grace for its report, since a loaded host can schedule its
   thread late (owner decision on #455). A supervisor that has not
   reported by then is `client supervisor unfinished`, and nothing is
   claimed of the sessions it waited for.
3. Meanwhile the host is still read: input, repeated prefix+x and further
   signals are discarded and never restart the deadline or change the
   status. The screen reads `Exiting: waiting for N sessions` (`1 session`
   for one; owner decision on #118), the sessions shutdown still waits for.
4. The host is restored: the documented normal-shell baseline (from
   `climon2-vt`) on the monitor's alternate screen, which it then leaves,
   so the shell's cursor returns, then termios, then, once the monitor
   has started, the root OSC 7501 `clear` (`ESC ] 7501;state=clear ESC \`;
   #114). Only then is any
   diagnostic printed; nothing is printed on a clean exit (owner decision
   on #118). A watchdog bounds this final phase by
   `deadlines.final_host_ms`: a host that cannot be restored by then ends
   the whole process with the climon2 error code, logged as `final host
   restoration unfinished`, never described as restored.

Each kind of evidence is kept on its own and logged per instance, content
free (`client shutdown evidence session=<id> leave=… detached=… closed=…
failed=… workers=… released=… destroyed=… joined=…`): the daemon's answer
to Leave and whether it detached an attachment, connection closure and the
client outcome from the library's terminal evidence, worker cleanup from
run's report, handle release, destroy and the owner thread's join. None
stands in for another.

**Exit statuses.** The monitor never returns a session's own exit status.

- **0** after prefix+x, and **128 + signal** (130, 143, 129) after a
  catchable signal, only when every current Leave is confirmed or not
  required, every instance is cleaned up and joined, and the host is
  restored. A session's command failing, a client error resolved on its
  row and an earlier loss whose instance is already cleaned up do not
  fail it.
- **125**, the climon2 error code, with one diagnostic after the host is
  restored: a render, host input or host output failure (which also takes
  this path), and a client whose connection may have been active when
  shutdown began but whose Leave was rejected, failed or lost, whose client
  failed, or that ended with no terminal evidence: `monitor shutdown
  unconfirmed: session <id>: <what>`.
- **125 by fail-stop** when cleanup cannot be confirmed: an instance not
  cleaned up and joined by the deadline, one whose worker cleanup or
  destroy failed (now or earlier in the run), a release load or discovery
  pass still running. The monitor restores the host, sends the root clear
  and prints the same one-line diagnostic, naming each session and what
  is unconfirmed, then ends the whole process with `_exit`: it never kills
  a thread, unloads a library, runs library destructors under running
  client code, or claims a join, release or detach that did not happen.

A native crash or abort in a client library ends the whole monitor and
every client it hosts, with no cleanup promise, and so do uncatchable
signals. Ordinary sessions then keep their own leave policy: the daemon
treats the lost clients as it treats any client that disappears, and
nothing promises an ordinary session survives; headless sessions allow zero
clients.

The general log records `monitor started version=<folder>`, then
`monitor configuration effective=<JSON>` with every effective setting in the
file's shape, then `monitor preference unusable problem=<reason>` if the
preference file could not be used, `monitor preference saved
suppressed=<bool>` or `monitor preference not saved problem=<reason>` for each
save, the shutdown evidence above, `shutdown work unfinished work=<what>`,
and `monitor stopped status=<n>` or `monitor failed error=<diagnostic>`,
never terminal content or input.

With `test-hooks` only, `CLIMON2_MONITOR_FAULT=draw` makes drawing fail,
`CLIMON2_MONITOR_FAULT=poll` fails the wait for host input with EFAULT, as
the kernel fails a wait given no descriptor table,
`CLIMON2_MONITOR_FAULT=discovery` stalls discovery before its first pass for
good, and `CLIMON2_MONITOR_FAULT=release`
stalls the host's release for good. `cargo xtask test` first runs
`cargo test -p climon2-monitor --test build` alone, which builds the plain
binary; the workspace tests that follow unify `test-hooks` through
`climon2-test-term` into the same sibling path.

## On-screen strings

Owner decision on #73 (2026-11-16): every user-visible monitor string lives
in this one table, for the owner to review once. Tickets add their strings
here as they implement them; `<…>` marks a value filled in at run time.

| Where | String | Source |
|---|---|---|
| Top row, nothing selected | `No session selected` | `screen.rs` |
| Body, no rows | `No sessions to show.` | `screen.rs` |
| Body, rows but nothing selected | `No session selected.` | `screen.rs` |
| Body, key hint | `Press <prefix> then x to exit.` | `screen.rs` |
| Body, startup progress while the initial collection lasts (#266, owner wording 2026-11-16) | `Connecting to <N> sessions (<M> ready)`; for one, `Connecting to 1 session (<M> ready)` | `initial.rs` |
| Body, incomplete discovery pass | `Discovery incomplete: the pass deadline passed`, `Discovery incomplete: cannot list sessions: <error>` | `lib.rs`, `screen.rs` |
| Body, or the top row's error slot with a row selected: initial collection ended with sessions still starting (#266) | `Discovery incomplete: some sessions are still starting` | `screen.rs` |
| Host too small | `Terminal too small` | `screen.rs` |
| Placeholder | `This session is being viewed elsewhere.` (taken over) or `This session is not controlled here.` (#400), then `Press <key> to take control` | `screen.rs` |
| Selected without a connected client | `No terminal content is available.`, then the row status | `screen.rs` |
| Selector machine column, while the monitor remembers a relay (#502; #199, item 1) | `local`, otherwise the relay's name as the monitor remembers it | `registry.rs` |
| Row and top-row statuses (#107; `Awaiting resources` #120) | `Needs attention`, `Acknowledged`, `Running`, `Idle`, `Succeeded`, `Failed (exit <code>)`, `Failed (signal <number>)`, `Starting`, `Awaiting resources`, `Unavailable`, `Disconnected` | `registry.rs` |
| Top-row control (#108) | `Acquiring`, `Restoring`, `Controlling`, `Not controlling`, `Ended` | `terminal.rs` |
| Top-row control slot while the canvas is scrolled back into history (#112, owner decision 2026-11-16) | `History` | `screen.rs` |
| Canvas, above the oldest history row kept once older rows were discarded (#112; the glossary's truncation notice, owner decision 2026-11-16) | `[climon2: older session scrollback was discarded]` | `screen.rs` |
| Top-row label or selector row that overflows its cells (#112, owner decision 2026-11-16) | `<text, truncated>…` | `registry.rs` |
| Top row's error slot: a paste the input queue refused partway (#111, owner decision 2026-11-16) | `Paste truncated` | `terminal.rs` |
| Selector footer, key hints on (#113, owner decision: only select, cancel and help) | `Enter: select  Esc: cancel  <prefix> then ?: help`; with Ctrl+[ as the prefix `Enter: select  <prefix> then ?: help`; with Ctrl+M as the prefix `<prefix> then <prefix>: select  Esc: cancel  <prefix> then ?: help` | `selector.rs` |
| Key introduction (#105; prefix+j by owner decision on #114) | `<prefix> then x  Exit the monitor`, `<prefix> then s  Open the session list`, `<prefix> then j  Jump to the session that needs attention`, `<prefix> then ?  Show this dialog`, `[ ] Don't show again (Space)` (`[x]` once saved), `Continue (Enter)`, titled `Keys` | `keys.rs`, `introduction.rs` |
| Key introduction, the selector's keys (#113, owner decision: help lists what the footer leaves out) | `In the session list:`, `Type or paste  Search by status and title`, `Left, Right, Home, End, Backspace  Edit the search`, `Up, Down, PageUp, PageDown  Move the highlight`, `Enter  Select the highlighted session` (`<prefix> then <prefix>  …` with Ctrl+M as the prefix), `Esc  Cancel` (left out with Ctrl+[ as the prefix), `Click a row  Highlight it`, `Click outside  Cancel`, `Wheel  Scroll the list` | `selector.rs` |
| Selector footer's row, search refused for its length (#113) | `Search is limited to <N> bytes` | `selector.rs` |
| Selector rows and footer too long for the box (#113, owner decision on #112) | the text that fits, then `…` | `selector.rs` |
| Top row's error slot, or the body with nothing selected, for 3 seconds after prefix+j finds no session (#114, owner decision 2026-11-16) | `No sessions need attention` | `attention.rs` |
| Toast, a needs-attention toast's hint row while key hints are on (#115) | `<prefix> then j: jump to attention` | `lib.rs` |
| Toast standing for those a full queue left out (#115, owner wording 2026-11-16) | `More events occurred. Check sessions list` | `toast.rs` |
| Over a not-connected row's retained history or its no-content state while a passive retry is scheduled or running (#116) | `Reconnecting...` | `screen.rs` |
| Over the same once recovery is permanently rejected (#116) | `Not reconnecting: <diagnostic>` | `screen.rs` |
| Over the same, and in the top row's error slot, once discovery proved the session's daemon gone (#116, owner decision 2026-11-16) | `Session no longer exists` | `screen.rs` |
| The whole host while shutdown waits (#118, owner decision 2026-11-16) | `Exiting: waiting for <N> sessions`; for one, `Exiting: waiting for 1 session` | `screen.rs` |
| After exit, on the shell, with the climon2 error code (#118; only on failure, owner decision 2026-11-16) | `climon2: error: monitor shutdown unconfirmed: <part>; <part>… (logs: <path>)`, each part `session <id>: <what>, …` or `<work> unfinished`; `<what>` among `Leave rejected`, `Leave failed`, `Leave unconfirmed`, `client failed: <error>`, `connection closure unconfirmed`, `no terminal evidence`, `handles not released`, `run not returned`, `worker cleanup unconfirmed: <why>`, `not destroyed`, `not destroyed: <error>`, `thread not joined`; `<work>` among `release <version> load`, `discovery pass`, `client supervisor` | `shutdown.rs`, `failstop.rs` |
| Top row's error slot: a lost or failed client's diagnostic (#116, owner decision 2026-11-16) | the library's or loader's own message, or the monitor's: `cannot poll the client: <error>`, `cannot drain the client: <error>`, `the client stopped without terminal evidence`, `the client stopped before it opened`, `cannot confirm the client's cleanup: <error>` | `supervisor.rs` |
| Top row's error slot: a client failed after its delivery congestion outlasted `deadlines.delivery_stall_ms` (#119) | `delivery congestion lasted past <N> ms` | `supervisor.rs` |
| Top row's error slot: an update the session's model refused, its client dropped and retried (#119) | `terminal model over its limit: <needed> bytes needed, <limit> allowed` | `terminal.rs` |
| Top row's error slot, and `Not reconnecting: <why>` over the retained cache: a session whose connection generations are exhausted (#119) | `connection generations are exhausted` | `supervisor.rs` |
| Top row's error slot: a resource-blocked row, what admission found short (#120) | `not enough memory: <needed> bytes needed, <available> bytes available`, `not enough file descriptors: <needed> needed, <available> available` | `climon2-embed`'s `admission.rs` |
| Top row's error slot, and `Not reconnecting: <why>`: a release whose qualified requirements the limits never admit (#120) | `the release needs <n> worker threads for each client; monitor.limits.resources.max_workers_per_client allows <m>`, `the release needs <n> bytes for each client library; its share of monitor.limits.memory.client_bytes is <m>` | `climon2-embed`'s `admission.rs` |
| Top row's error slot: a row whose release has loaded past `deadlines.load_ms` (#120, owner wording 2026-11-16) | `library loading slowly` | `registry.rs` |
| Top row's error slot: a request after the client's last request ID (#119) | `the client's request IDs are exhausted` (after `Input not sent (<N> bytes; earlier input was accepted): ` for input) | `supervisor.rs`, `terminal.rs` |
| Delete confirmation (#117, owner decision: Cancel first and focused) | titled `Delete session`; the row's label and status; `[ Cancel ]`, `[ Terminate and remove ]`, or `[ Remove ]` for a final or gone session | `delete.rs` |
| Status column and top row while a confirmed Delete is under way (#117) | `Terminating`, `Removing` | `registry.rs` |
| Top row's error slot after a Kill fell short (#117) | `Termination refused: <why>`, `Termination not confirmed: <why>`; why may be the library's or daemon's message, `the client ended before the Kill was answered` or `cannot confirm the client's cleanup: <error>` | `registry.rs`, `supervisor.rs` |
| Key introduction, Delete (#117, owner decision: fn+⌫ on macOS) | `Del  Delete the highlighted session`; on macOS `Del (fn+⌫)  Delete the highlighted session` | `selector.rs` |

## Pairing with a relay (#501)

`links::pair(home, address, pin, largest_record)` is the hub monitor's side
of pairing, which `c2 monitor pair` runs ([ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md)
puts the hub monitor's side of monitor links here, on `climon2-link`). Unix
only, like the seams it uses. In order:

1. It reads `links/relays.json`; a damaged store fails before any dial.
2. It makes a fresh keypair for this pairing (P15), dials the address as
   typed (a host name is resolved now, CM9) within 10 seconds, and runs the
   pairing connection, suggesting the host name as its name.
3. Once paired, it dials the first pinned-key link, and closes it with
   "hub monitor disconnected" as soon as it is established (ST15).
4. Only then does it record the relay, connected, under the name the relay
   suggested without control characters, replacing an earlier record of the
   same relay (P14, P16, P20), and report its name and key fingerprint.

Each failure is an `Unpaired` that names the address: unreachable, no link
invitation open, pairing failed (wrong PIN or interference), no shared link
revision, or a first link that did not succeed (P25). Nothing is recorded of
a relay that was not paired with. `tests/pair.rs` drives it against a relay
built from the link code on loopback.

## Remote sessions in the selector (#502)

A hub monitor lists the sessions each relay advertises beside its local
ones ([ADR 0011](../../docs/adr/0011-monitor-library-and-client-embedding.md);
the advertisement resolution of #195; the selector resolution of #199).
Unix only, like the link seams.

- **Identity** (`machine.rs`). A row is a `SessionAt`: `Machine::Local` and
  the session ID, or `Machine::Relay(key)`, the relay's pinned public key,
  and the session ID on that relay. The two are never compared, so one
  session ID on three machines is three rows. Rows, the selection,
  terminals and toasts are keyed by it, and so are `View::id`,
  `Choice::Select`, `Choice::Delete` and `Event::Removed`.
- **Links** (`remote.rs`). At start the monitor reads `links/relays.json`,
  if `links/` exists; a damaged store fails the start. Each relay that is
  not disconnected gets a thread that dials it within 10 seconds, holds
  the link and hands the monitor `Linked` and each advertisement. A link
  that ends is dialled again after `monitor.limits.retry.initial_ms`;
  backoff, its reset and the slower redial against a refusing relay are
  #504's. Each dial reads the relay's record afresh, so a changed address
  is dialled and a relay forgotten or disconnected meanwhile is not.
  Shutdown closes every link with "hub monitor stopped".
- **Rows** (`registry.rs`). A Session record adds a remote row or replaces
  what it lists; Withdrawn applies the local rule (a row never connected is
  dropped, one that was stays Disconnected); a new link connection discards
  what the relay held before its fresh snapshot. Every advertised field is
  shown literally. A remote row waits as `Unavailable` with no content until
  its session stream exists (#503): Enter selects it without acquiring, and
  it offers no Delete. `Supervisor::route` sends a request to a local
  session's client only.
- **Machine column.** While the monitor remembers at least one relay, each
  row's `View::machine` is `local` or the relay's name, the selector draws it
  between status and title, as wide as the widest, and search matches the
  status, the machine, then the title. A monitor that remembers no relay
  has no column, so its selector is unchanged.

`tests/remote.rs` drives the library against relays made from the link code
on loopback; the Pair layer is in `climon2-e2e/tests/remote.rs`.

## Specification mapping

Monitor-layer tests drive the installed product from `climon2-e2e`'s
`tests/monitor.rs`. The terminal interface keeps the build proof; both crates
keep their pure unit tests, and this crate's `tests/monitor.rs` drives the
library through its own interface.

| Scenario | Named test |
|---|---|
| ENT-01 (Monitor): `c2 monitor` and `c2 m` open the same installed monitor; top row, empty state, exact host modes; prefix+x exits 0 with the host released, the prefix read alone | `installed_monitor_and_m_open_the_same_monitor` (e2e) |
| INP-01 (Monitor): configured prefix hint and exit; null take-control key accepted; once read, Ctrl+C and the default prefix+x are not exits | `installed_monitor_hints_and_exits_with_the_configured_prefix` (e2e) |
| INP-01 (Monitor): null escape prefix and invalid keys refused before terminal changes | `installed_monitor_refuses_a_null_escape_prefix_and_invalid_keys` (e2e) |
| INP-01 (Monitor: takeover, #187): the placeholder names the configured take-control key, which takes control back | `the_placeholder_names_the_configured_take_control_key` (e2e) |
| INP-01 (Monitor: takeover, #187): with a null take-control key the placeholder shows only its first line; the selector still takes control back | `without_a_take_control_key_the_placeholder_shows_only_its_first_line` (e2e) |
| CFG-01 (Monitor): unreadable and malformed JSONC, null, wrong type, duplicate, overflow, out-of-range and inconsistent settings each refused with one diagnostic and 125 before terminal changes | `installed_monitor_refuses_every_invalid_configuration_before_the_terminal_changes` (e2e) |
| CFG-01 (Monitor): limits one byte short of the monitor's parser and one 500x200 session's current model and recent page (total), of the model and page (client budget) or of its recent page (history total, then per-session history budget) each refused with one diagnostic naming that limit and 125 before terminal changes; at those limits the monitor opens and prefix+x exits 0 (#167) | `installed_monitor_refuses_limits_that_cannot_hold_the_largest_session_model` (e2e) |
| CFG-01 (Monitor): `show_key_hints` false hides the empty state's hint, prefix+x still exits; unknown and unrelated settings ignored | `installed_monitor_without_key_hints_shows_none_and_still_exits` (e2e) |
| CFG-01 (Monitor): no live reload: after an edit, a repaint keeps the started settings and the edited prefix does not act | `editing_the_configuration_while_the_monitor_runs_changes_nothing` (e2e) |
| CFG-01 (Monitor): every effective setting reported in the general log | `installed_monitor_logs_its_effective_configuration` (e2e) |
| INP-03 (Monitor, empty monitor): pastes never exit, never complete a pending prefix and leave none pending | `pastes_never_exit_or_leave_a_pending_prefix` (e2e) |
| Derived scenario: monitor argument refusal (2, usage) | `installed_monitor_arguments_are_a_usage_error` (e2e); core `issue74_monitor_and_m_take_no_arguments_and_mon_is_gone` |
| Derived scenario: monitor non-TTY stdin or stdout refusal | `installed_monitor_refuses_a_non_terminal_stdin_or_stdout` (e2e) |
| Spec #73 entry: a missing monitor names the version | `missing_installed_monitor_fails_naming_its_version` (e2e) |
| Spec #73 startup: the executable home is resolved as the CLI does | `a_monitor_outside_a_version_folder_refuses_to_start` (e2e); core `issue74_monitor_resolves_the_same_layout_as_the_cli` |
| EXT-03 (Monitor, empty monitor): SIGINT, SIGTERM and SIGHUP exit 130, 143 and 129 with baseline and exact termios | `signals_release_the_host_with_signal_style_status` (e2e) |
| EXT-03 (Monitor, empty monitor): hung-up host input, read as its end or as EIO, releases the host before its one diagnostic, logged, 125 | `a_hung_up_host_input_releases_the_host_before_its_diagnostic`, `host_input_failing_with_eio_releases_the_host_before_its_diagnostic` (e2e); unit `only_eio_ends_host_input` |
| EXT-03 (Fault, empty monitor): a render failure, or a failed wait for host input, releases the host before its one diagnostic, 125 | `debug_hook_render_failure_restores_the_host_before_its_diagnostic`, `debug_hook_input_wait_failure_restores_the_host_before_its_diagnostic` (e2e) |
| Spec #73 host terminal; TRM-04 (Monitor, empty monitor): SIGWINCH repaints without leftovers, erasing or showing the cursor | `resizing_the_host_redraws_the_monitor_at_the_new_size` (e2e) |
| Spec #73 exit: content-free general log | `installed_monitor_logs_content_free_records_to_its_general_log` (e2e) |
| DSC-03 (Monitor, including its selection case), empty start (confirmed 2026-11-08), CTL-01 (Monitor), COM-01 (Monitor), LIB-01 (Monitor, Linux maps), HDL-01 (Monitor rows): sessions created after an empty start appear unasked, one passive client and library log each, reused across passes; the first eligible arrival is the one-time initial selection, acquired, and a later arrival neither moves it nor is acquired; the daemon's Idle status, never a fabricated one; titles and command fallback; exactly the version's `libclimon2_client.so` | `sessions_appear_as_they_are_created_each_with_one_passive_client` (e2e) |
| DSC-04 (Monitor, Fault): malformed and oversized metadata, mismatched hello, silent probe, inaccessible discovery, uncertain disappearance; no metadata deleted, no client torn down; a stale session nobody knew adds no row and gets no client (owner decision on #106); DSC-01 confirmed exclusion: with only not-connected rows and a connected row without `status-v1`, nothing is selected (#266) | `uncertain_discovery_is_explicit_and_never_deletes_metadata_or_clients` (e2e) |
| DSC-01 (Monitor, start order): bounded starts newest created first, ascending ID ties, each after the previous one failed, as the fake daemons heard them | `clients_start_one_at_a_time_newest_first_with_ascending_id_ties` (e2e) |
| DSC-03 (Monitor, cadence): immediate first pass, each later pass a second after the previous one ended, no overlap, the selector opened during a pass | `discovery_passes_start_a_second_after_the_previous_one_ends_without_overlap` (e2e) |
| Client supervisor (Fault), LIB-05 (Monitor, Fault): a failed notification lookup, a run failing without terminal evidence, or a failed drain ends that instance cleanly (deliveries and handles released, destroyed) and frees its start slot | `a_failed_notification_lookup_ends_its_client_cleanly_and_frees_its_start_slot`, `a_failed_run_without_terminal_evidence_ends_its_client_cleanly_and_frees_its_start_slot`, `a_failed_drain_ends_its_client_cleanly_and_frees_its_start_slot` (`src/supervisor.rs`) |
| Client supervisor (Fault), EXT-02 (Fault, #118): exit lasts until `deadlines.shutdown_ms` and no longer, apart from the 200 ms handover grace while the supervisor's report is late (#455), says whom it waits for, logs any client it could not confirm, and never destroys or claims an instance with unreleased handles; a refused release is loaded once and its sessions are Unavailable | `exit_is_bounded_and_never_destroys_an_instance_with_unreleased_handles`, `a_round_waits_no_further_than_the_shutdown_deadline`, `draining_stops_at_the_shutdown_deadline`, `a_flood_does_not_starve_the_shutdown_command`, `many_floods_still_let_the_exit_be_read_within_about_a_tick`, `a_flood_does_not_starve_a_busy_instance_after_it`, `an_idle_instance_is_drained_once_a_round`, `a_refused_release_is_loaded_once_and_its_sessions_are_unavailable` (`src/supervisor.rs`), `only_a_missing_report_is_waited_for_past_a_tick` (`src/monitor.rs`) |
| EXT-01 (Monitor, Fault; #118): one shutdown asks passive, partially started, failing, stalled and unreleasable instances at once; each kind of evidence (Leave, client outcome, closure, worker cleanup, release, destroy, join) proven alone; a lost Leave fails the exit, unconfirmed cleanup fail-stops it, naming exactly what; an instance published only once shutdown has begun is asked before that round's poll, and a flooding instance starves no other (#462) | `shutdown_asks_every_instance_at_once_and_proves_each_kind_of_evidence_alone`, `an_instance_published_during_shutdown_is_asked_before_the_poll`, `a_flood_in_shutdown_does_not_starve_another_instance` (`src/supervisor.rs`) |
| EXT-02 and LIB-06 (Fault, #118): a release load still running at the deadline is reported unfinished, never joined | `a_load_unfinished_at_the_deadline_is_reported_and_never_joined` (`src/supervisor.rs`) |
| EXT-01, EXT-02, EXT-04 and LIB-06 (pure supplements, #118): the verdict over separate evidence (clean, failed, fail-stop) and its diagnostic; Leave codes; terminal evidence and run reports read each kind alone; an earlier failure leads the diagnostic; the exit screen's count | `confirmed_or_unneeded_leaves_with_complete_cleanup_are_clean`, `an_unconfirmed_current_leave_fails_the_exit_naming_it`, `unconfirmed_cleanup_or_unfinished_work_is_a_fail_stop_naming_each_piece` (`src/shutdown.rs`), `leave_codes_read_as_their_leave`, `terminal_evidence_proves_each_kind_of_departure_evidence_alone`, `only_a_successful_run_or_a_complete_report_confirms_worker_cleanup` (`../climon2-embed/src/instance.rs`), `a_shutdown_failure_follows_any_earlier_failure` (`../climon2-monitor-tui/src/lib.rs`), `the_exit_screen_counts_the_sessions_it_waits_for` (`../climon2-monitor-tui/src/screen.rs`) |
| EXT-01, EXT-03 (orderly exit) and EXT-04 (Monitor, #118): prefix+x leaves a passive and a controlling client on one path, each kind of evidence logged alone, the controlling Leave detaching; exit 0 with the host restored and cleared, nothing printed, though the command never exits 0 | `exit_leaves_every_client_and_records_each_kind_of_evidence_alone` (e2e) |
| EXT-02 and LIB-06 (Monitor, Fault; #118): a Leave never answered stalls past the deadline; the exit screen counts the session; repeated prefix+x and signals neither restart the deadline nor reach the shell; the host is restored and cleared, then a fail-stop with 125 names what is unconfirmed, claiming no join | `a_stalled_leave_fail_stops_at_the_deadline_however_often_exit_is_repeated` (e2e) |
| EXT-02 and LIB-06 (Fault; #118; hooked): worker cleanup past `cleanup_ms` and a busy destroy fail-stop without claiming them; a discovery pass that never ends fail-stops at the deadline; a host release that never ends exits 125 after `final_host_ms`, never described as restored | `unconfirmed_worker_cleanup_fail_stops_without_claiming_it`, `debug_hook_stalled_discovery_fail_stops_at_the_deadline`, `debug_hook_unfinished_host_restoration_fails_after_the_final_host_allowance` (e2e, hooked only) |
| LIB-06 (Monitor, Fault; #118; hooked library): a native abort in a client library, in this disposable monitor process, ends the whole monitor (134) | `a_native_abort_in_a_client_library_ends_the_whole_monitor` (e2e, hooked only) |
| EXT-04 (Monitor, #118): an earlier transport loss, cleaned up and reconnected, does not fail the later clean exit | `a_lost_connection_keeps_its_history_read_only_until_a_passive_reconnect` (e2e) |
| Client supervisor: `scheduling.starts` at its bound starts that many clients and holds the next session (no client, no log directory) until one opens | `client_starts_never_exceed_the_start_lane` (`src/supervisor.rs`) |
| Client supervisor: terminal evidence before Open's response makes the row Unavailable with its message, or a stated reason when it is empty | `terminal_evidence_before_open_says_why_the_session_is_unavailable` (`src/supervisor.rs`) |
| Own-session scenario (confirmed 2026-11-08; hidden, owner decision on #110), DSC-06 (Monitor), HDL-01 (Monitor rows), CTL-01 (controlled elsewhere), TRM-06 (wide labels), SEL-01 (cell-safe clipping): the own session is never listed and gets no client, while nested and controlled sessions appear | `the_own_session_is_hidden_while_nested_and_controlled_sessions_appear` (e2e) |
| SEL-01 (Monitor, basic cases): border, search line, separator, columns, footer, highlight on the selected session, Up/Down without wrap, Escape, no application resize or acquisition (only the initial selection is acquired and resized, #266); many frames, so only the title itself is checked afterwards | `the_selector_lists_rows_moves_its_highlight_and_closes_without_resizing` (e2e) |
| SEL-01 (Monitor: search line, separator, footer, empty matches), SEL-02 (Contract and Monitor: exact, subsequence, short, spaced, status-crossing, Unicode and wide queries in row order, never by score; edit keys; paste as one line, its prefix+x and CR inert; Enter) | `the_selector_search_filters_by_status_and_label_in_row_order` (e2e) |
| SEL-01 (Monitor: narrow 40x14 and large 200x60 hosts, scrolling, `…` clipping of long and wide labels and the footer), SEL-02 (Monitor: Tab, Up, Down, PageUp, PageDown without wrapping, clicks, the wheel), INP-05 (Monitor, selector case: clicks inside and outside the box), TRM-03 (Monitor, overlay case) | `selector_navigation_mouse_and_wheel_on_narrow_and_large_hosts` (e2e) |
| SEL-02 (Monitor: asynchronous title and status updates clear the highlight for good), SEL-03 (Monitor and Fault: Enter pins the highlighted session through a reorder and another client's takeover) | `async_updates_clear_the_highlight_and_enter_pins_the_highlighted_session` (e2e) |
| SEL-03 (Monitor and Fault: loss): a highlighted session lost (its daemon killed) turns Disconnected and moves with the highlight; Enter there acquires neither it nor the row that took its place; Up then Enter acquires exactly that row | `enter_pins_the_highlighted_session_while_it_is_lost` (e2e) |
| SEL-03 (Monitor: Escape and a click outside cancel, keeping selection and control; with Ctrl+[ as the prefix Escape cannot cancel, the footer omits it, prefix+x exits) | `cancelling_keeps_the_selection_and_an_escape_prefix_cannot_cancel` (e2e) |
| HLP-02 (Monitor, over the selector): the key introduction lists the selector's keys (owner decision on #113); input isolation while the selected session writes output, search and highlight restored, cursor hidden then back in the search, prefix+x exits | `help_over_the_selector_isolates_input_and_restores_search_and_highlight` (e2e) |
| Operational limits `ui.search_bytes` (Monitor): typed and pasted text at the limit taken, one byte past (single- and multi-byte) refused visibly, nothing reaching the session | `search_text_past_its_byte_limit_is_refused_visibly` (e2e) |
| SEL-02 (Contract, pure supplement): `SkimMatcherV2` ordered-subsequence matching over status, space and label, ignoring case beyond ASCII; editing, control characters and invalid UTF-8; paste as one line; `ui.search_bytes` at 4096 and 4097 bytes, single- and multi-byte, typed and pasted; the search line keeps its cursor in view | `search_matches_ordered_subsequences_of_status_and_label`, `search_ignores_unicode_case_and_matches_wide_text`, `typing_backspace_and_cursor_keys_edit_at_the_cursor`, `control_characters_and_invalid_bytes_insert_nothing`, `typing_past_the_search_limit_is_refused`, `a_paste_is_inserted_as_one_line`, `a_paste_past_the_search_limit_is_refused_whole`, `the_search_line_keeps_the_cursor_in_view` (`src/search.rs`) |
| SEL-01, SEL-02, DSC-05 (pure supplement): layout, footer variants and reclaimed row, navigation and scrolling, highlight rules for edits and live updates, opening, mouse, paste and refusals, search cursor, the smallest boxes, input other than keys, pastes and mouse reports | `the_selector_draws_search_separator_and_two_columns`, `the_footer_names_the_actual_bindings_and_its_row_is_reclaimed_without_hints`, `navigation_moves_without_wrapping_and_keeps_the_highlight_in_view`, `search_edits_keep_a_matching_highlight_or_take_the_first_match`, `a_highlighted_row_that_leaves_or_stops_matching_is_unhighlighted_for_good`, `opening_highlights_the_selected_session_or_the_first_row`, `clicks_highlight_rows_or_cancel_outside_and_the_wheel_scrolls`, `pastes_fill_the_search_and_refusals_show_in_the_footer`, `the_selector_takes_keys_pastes_and_mouse_reports_only`, `search_edits_and_reopening_scroll_back_to_the_start`, `the_search_cursor_follows_the_text`, `the_selector_draws_only_what_fits`, `help_lists_every_selector_key_with_the_actual_prefix` (`../climon2-monitor-tui/src/selector.rs`) |
| INP-05 (pure supplement): SGR mouse reports parsed; clicks and wheel notches whatever the modifiers | `sgr_reports_parse_and_malformed_ones_do_not`, `left_presses_click_and_wheel_notches_scroll_with_any_modifiers` (`../climon2-monitor-tui/src/mouse.rs`) |
| DSC-03 (pure registry supplement): one client request per live session; rows tied on their entry time in ascending full session ID | `live_sessions_need_one_client_each` (`src/registry.rs`) |
| DSC-01 (Monitor, status order), COM-06 (Monitor labels), STA-07 (Monitor, hidden change), DSC-05 (Monitor, reorder case): real sessions at known daemon clock times show Needs attention, Running, Idle, Failed (signal 15), Failed (exit 143) and Succeeded in group and daemon-time order, finals kept after their sessions end; working under pending attention moves nothing, even with its daemon clock moved past a newer attention row's time; a reorder keeps the highlighted row | `real_sessions_show_their_effective_status_and_final_outcomes_in_order` (e2e) |
| DSC-01 (Monitor, status order: every status group, a tied change time broken by ascending full session ID, never creation order, delayed Initial metadata Starting until it arrives; initial-selection case, #266: the startup progress `Connecting to 9 sessions (8 ready)` until the delayed Initial metadata, then the first eligible row, neither the newest created nor the first started, selected and acquired under the open selector), DSC-05 (Monitor, reorder case: a row moving in above the highlight, and the highlighted row's own status change) | `rows_order_by_status_group_daemon_change_time_and_full_session_id` (e2e, Fault peers) |
| Not-connected rows (confirmed 2026-11-08), final rows without a status time (derived), STA-07 (Monitor, disconnection case): the own session never listed; Unavailable (no `status-v1`, uncertain) and Disconnected after every status and final row, newest condition first; a lost connection never shows its stale status or a fabricated outcome; untimed finals from Initial metadata and Command final sort by when learned, between newer and older daemon times | `not_connected_rows_and_untimed_finals_sort_by_when_the_monitor_recorded_them` (e2e, Fault peers) |
| DSC-01 status order, STA-07 (pure supplement): group, newest daemon change time, ascending full session ID; only a new change time moves a row; stale status of a disconnected row | `rows_order_by_group_then_newest_daemon_change_then_session_id` (`src/registry.rs`) |
| COM-06 labels, final rows (pure supplement): exact exit and signal labels, daemon time or learned time, finals surviving disconnection, later statuses and repeats | `final_outcomes_are_labelled_exactly_and_sort_by_their_time` (`src/registry.rs`) |
| Not-connected rows (pure supplement): every not-connected label in the disconnected group by entry time | `not_connected_rows_sort_by_when_they_entered_their_condition` (`src/registry.rs`) |
| STA-07, COM-06 (pure supplement over native records): Initial metadata status or final outcome, `CM1_STATUS` states and exact times, `CM1_FINAL` exit apart from signal; nothing for final, unknown or unreadable values | `the_initial_metadata_reports_its_status_or_final_outcome`, `status_records_keep_their_state_and_exact_change_time`, `command_final_reports_its_exact_result` (`src/supervisor/client.rs`) |
| Own-session scenario (pure supplement): no row and no client for the own session | `the_own_session_is_hidden_and_never_needs_a_client` (`src/registry.rs`) |
| DSC-01 (initial-selection case, pure supplement), confirmed exclusion: only a connected row with a known nonfinal status is eligible, the first in row order; Starting until a client opened with `status-v1` reports its status | `only_a_connected_row_with_a_known_nonfinal_status_is_eligible` (`src/registry.rs`) |
| DSC-01 (initial-selection case, bounded collection, pure supplement), empty start and the owner decisions of 2026-11-16: collection waits for the first pass and each of its clients until ready or `deadlines.start_ms` (at the limit and one nanosecond before), then takes the first eligible row once; with none, the first row to become eligible later, once; an explicit selection ends it; late clients reported while still starting | `collection_waits_for_the_first_pass_and_its_clients_then_takes_the_first_eligible_row`, `with_nothing_eligible_the_first_row_to_become_eligible_is_taken_once`, `an_explicit_selection_ends_initial_selection`, `a_client_past_its_start_deadline_no_longer_holds_collection_and_is_reported_late` (`src/initial.rs`) |
| Startup progress and incompleteness (pure supplement): the body reads the progress while collecting, otherwise the empty or unselected state, then any incomplete pass and late collection; with a row selected, late collection fills the top row's error slot unless the session has its own error; one session in the singular (1 and 2) | `the_body_says_the_startup_progress_or_the_state_and_any_incompleteness`, `a_late_collection_shows_in_the_error_slot_unless_the_session_has_its_own` (`../climon2-monitor-tui/src/screen.rs`), `the_progress_says_one_session_in_the_singular` (`src/initial.rs`) |
| DSC-04 (pure supplement): uncertainty never removes rows or tears down clients | `uncertainty_never_removes_rows_or_tears_down_clients` (`src/registry.rs`) |
| Stale rows (owner decision on #106): a session proven gone drops its row unless it was ever connected | `a_session_proven_gone_drops_its_row_unless_it_was_ever_connected` (`src/registry.rs`) |
| COM-01 and not-connected labels (pure supplement) | `connection_statuses_are_labelled_as_specified` (`src/registry.rs`) |
| TRM-06 label cases (pure supplement): latest nonempty title, command fallback, literal, never IDs; static, cell-safe truncation with `…` at the limit and one past it, in the top row (beside its error) and the selector (owner decision on #112) | `labels_are_the_latest_nonempty_title_or_command_and_arguments_literally`, `overflowing_text_is_truncated_display_cell_safe_with_an_ellipsis` (`src/registry.rs`), `an_error_is_clipped_first_and_never_squeezes_out_the_label` (`../climon2-monitor-tui/src/screen.rs`), `the_selector_draws_search_separator_and_two_columns` (`../climon2-monitor-tui/src/selector.rs`) |
| SEL-01 basic cases (pure supplement) | `the_selector_draws_search_separator_and_two_columns`, `navigation_moves_without_wrapping_and_keeps_the_highlight_in_view`, `opening_highlights_the_selected_session_or_the_first_row`, `the_selector_draws_only_what_fits` (`../climon2-monitor-tui/src/selector.rs`) |
| CTL-05 (Monitor), TRM-01 (Monitor, basic cases), initial selection (#266): the one Idle session selected and acquired once collection ends; plain, ANSI-coloured, bold-on-blue, wide and combining text and the cursor painted as written, live; a later arrival never moves the selection; the open selector covers the canvas and the control still reads Controlling | `the_initial_selection_is_acquired_and_painted_live` (e2e) |
| TRM-02 (Monitor): only the daemon answers session queries; no session byte or title reaches the host; the host is never queried | `only_the_daemon_answers_session_queries_and_no_session_byte_reaches_the_host` (e2e) |
| TRM-03 (Monitor) at 80x24, 200x60, 500x200 and a larger host: one top row; the desired area is the host less one row; canonical bytes at the effective geometry with blank margins | `the_selected_session_fills_the_host_below_one_top_row_at_its_effective_geometry` (e2e) |
| CTL-05 (Monitor, resize), TRM-03 (real host resizes): Resize only to the selected controller, never to a background one | `host_resizes_reach_only_the_selected_controller` (e2e) |
| TRM-04 (Monitor, too-small cases): 3 rows and 4 columns too small, 4 rows and 5 columns enough; the session reads its size while too small and was never resized; recovery; prefix+x still exits | `a_host_too_small_for_a_session_says_so_sends_no_resize_and_recovers` (e2e) |
| Top-row key hints (#108): none with `monitor.show_key_hints` false | `without_key_hints_the_top_row_shows_no_hint` (e2e) |
| Spec #73 Enter, never exposing cached output; CTL-05 (revoked): a revoked session shows only the placeholder, never its cache; its screen changes while revoked, and Enter takes control again and paints only the new restoration | `a_revoked_session_shows_no_cached_output_until_enter_takes_control_again` (e2e) |
| #94 rule for #108 and COM-06 (Monitor labels, #107): a finished session's view freezes at Command final, its top row showing `Failed (exit 3)` after its connection closes; Enter does not acquire a final row | `a_finished_session_keeps_its_last_screen_and_cannot_be_acquired` (e2e) |
| CTL-02 (Monitor), CTL-03 (Monitor), CTL-05 (Monitor: background and revoked), DSC-05 (Monitor: revocation), INP-01 (Monitor: takeover and the placeholder screen) (#110), titles: switching keeps background control and input reaches only the selection; titles update in the background, under the selector and after revocation; another client's takeover wins and the selection shows only the placeholder with its hint, no input or resize reaches any session, the take-control key does nothing in modals and the monitor never takes control back; the key acquires and is consumed, then is ordinary input; prefix+s and prefix+? act from the placeholder | `a_revoked_selection_shows_only_the_placeholder_until_the_take_control_key` (e2e) |
| CTL-03 (Monitor, status-value case, moved from #107 to #266): effective status keeps updating for a background controller, a selected row covered by the open selector (and its top row once uncovered) and a row revoked by another client's takeover | `status_keeps_updating_for_background_covered_and_revoked_rows` (e2e) |
| DSC-01 (Monitor, bounded collection; uncertainty explicit), owner decisions of 2026-11-16, spec #73 initial selection: progress while two first-pass clients start; at `deadlines.start_ms` collection ends selecting nothing, and `Discovery incomplete: some sessions are still starting` shows while one still starts, in the top row's error slot once a row is selected; the late row, once eligible, is selected and acquired once; an explicit read-only selection made meanwhile prevents it | `a_late_session_ends_collection_at_its_deadline_and_is_selected_once_eligible` (e2e, Fault peers) |
| Spec #73 Enter on Starting and Unavailable rows without a connected client (derived scenarios), CTL-05 (Monitor: unavailable) (#110): selected without acquiring, with no terminal content and the condition, keys reach no session; once the client connects, the placeholder until the take-control key acquires | `enter_selects_starting_and_unavailable_rows_read_only_until_the_user_acquires` (e2e) |
| CTL-04 (Monitor) (#110): promotion keeps a background model live without selection, resize or input; Enter then shows it at once and only then resizes | `promotion_keeps_a_background_model_without_selection_resize_or_input` (e2e) |
| INP-01 (Monitor: takeover on a too-small host; owner decision on #110): the take-control key acquires with no placeholder showing, nothing is resized until the host has room; prefix+x exits from the placeholder | `the_take_control_key_acquires_on_a_too_small_host` (e2e) |
| CTL-05 (Monitor: restoring) (#110): while a restoration is applied but its Ready not yet accepted, keys and a host resize reach no session, the background controller included; once ready, the session is sized to the host and keys reach it | `keys_and_resizes_during_a_restoration_reach_no_session` (e2e) |
| DSC-05 (Monitor, Fault: failed acquisition and loss) (#110): a timed-out Acquire keeps the selection, shows the failure, and input reaches no session; the resumed daemon still serves that Acquire, so the other controller loses control and gets it back | `a_failed_acquisition_keeps_the_selection_and_redirects_no_input` (e2e) |
| DSC-05 (Monitor: completion) | `a_finished_session_keeps_its_last_screen_and_cannot_be_acquired` (e2e) |
| CTL-02, INP-01 takeover, spec #73 placeholders (pure supplement, #110): only an uncontrolled connected row shows the placeholder and takes the key; without a view, the placeholder, the condition or nothing; the placeholder's hint only with a bound key and hints on | `only_an_uncontrolled_connected_row_shows_the_placeholder`, `a_selected_row_without_a_view_shows_the_placeholder_its_history_or_no_content` (`src/session.rs`), `the_placeholder_and_the_no_content_state_say_why_nothing_is_painted` (`../climon2-monitor-tui/src/screen.rs`) |
| CTL-05, COM-05 (pure supplement): Ready only after the apply; applied, Ready accepted and painted distinct, input needs all three; only Ready for the applied restoration counts | `ready_applied_and_painted_stay_distinct_and_input_needs_all_three`, `only_ready_accepted_for_the_applied_restoration_opens_input` (`src/terminal.rs`) |
| TRM-04, COM-05 (pure supplement): staged restorations replace the model whole once applied; superseded ones never become current; a cache is never shown | `a_restoration_replaces_the_model_whole_only_once_applied`, `a_cache_from_an_earlier_period_is_never_shown` (`src/terminal.rs`) |
| Carried from #76 (owner decision 2026-11-13): a Display at a geometry the model lacks marks it stale until the next restoration | `a_display_at_a_geometry_the_model_lacks_marks_it_stale_until_restored` (`src/terminal.rs`) |
| #94 rule (pure supplement): the view freezes at Command final, only if the period held then had applied a restoration; a finished session gets no Resize; painted is not sticky; the control label ignores visibility | `the_view_freezes_at_the_command_final`, `a_command_final_without_a_live_model_shows_no_view`, `a_finished_session_is_sent_no_resize`, `a_frame_that_stops_showing_the_restoration_closes_input`, `the_control_label_ignores_whether_the_canvas_shows` (`src/terminal.rs`) |
| CTL-05 (pure supplement): Resize once per grant and desire, never without control; superseded refusals are not errors and a refused Resize is resent; Acquiring and errors | `resize_follows_every_grant_and_each_new_desire_only_while_controlling`, `work_refused_under_a_superseded_restoration_is_not_an_error_and_resize_is_resent`, `acquiring_ends_at_the_grant_or_the_answer_and_errors_are_shown`, `each_control_has_its_label` (`src/terminal.rs`) |
| LIM-03 (pure supplement): a restoration over the model's limit is an error without Ready | `a_restoration_over_the_model_limit_is_an_error_without_ready` (`src/terminal.rs`) |
| LIM-03 (Monitor, Fault: rejected updates), LIM-01 (Monitor: the budget split), REC-01, REC-02 (#119): a session over its model's share is dropped Unavailable, its cache read-only, retried passively; others go on | `a_session_over_its_memory_limit_becomes_unavailable_read_only_and_is_retried` (e2e, Fault peers) |
| LIM-03 (pure supplement, #119): refused output or restoration drops the client once with the error shown, keeping the last safe cache; a dropped client is reported Unavailable and retried passively in a fresh generation once destroyed | `refused_output_drops_the_client_once_and_keeps_the_last_safe_cache`, `a_restoration_over_the_model_limit_is_an_error_without_ready` (`src/terminal.rs`), `a_client_dropped_at_its_memory_limit_is_retried_passively` (`src/supervisor.rs`) |
| LIM-03 (Monitor: history bounded by bytes, explicit truncation, the newest page; #119) | `history_past_its_byte_budget_is_discarded_oldest_first_with_its_notice` (e2e) |
| LIM-06 (Monitor: render cadence, input, selector and exit respond; #119) | `rendering_keeps_to_its_frame_rate_while_input_the_selector_and_exit_respond` (e2e) |
| LIM-06 (Monitor: notification work), NOT-02 (#119): a storm of 200 episodes from eight sessions stays within the toast bound while the selector and exit respond | `a_notification_storm_stays_within_the_toast_bound_while_input_responds` (e2e, Fault peers) |
| LIM-06 (Monitor: a host that stops reading, #285): the backed-up host receives only the newest frame once it drains; keys, records and exit within its deadline go on while it reads nothing | `a_backed_up_host_receives_only_the_newest_frame_once_it_drains`, `a_host_that_stops_reading_stalls_only_its_own_output` (e2e) |
| LIM-06 (pure supplement, #285): queued host output is written in order as the host takes it, refusals wait and other failures are returned; only the newest frame waits for the host, one it was handed last cancels a waiting one, and a frame showing a toast's full title is told once it has been handed over and taken (#115) | `queued_bytes_are_written_in_order_as_the_host_takes_them`, `a_failed_host_write_is_returned` (`../climon2-monitor-tui/src/host.rs`), `only_the_newest_frame_waits_for_the_host`, `a_frame_showing_a_title_is_told_once_it_is_written` (`../climon2-monitor-tui/src/screen.rs`) |
| LIM-02 (pure supplement over the fake library, #119): reports charged by capacity per client; a client is drained while its unapplied reports are under `queues.event_bytes` (at the allowance and one past) and failed after `deadlines.delivery_stall_ms` of congestion | `each_report_is_charged_by_the_capacity_it_holds` (`src/supervisor/backlog.rs`), `deliveries_are_held_per_session_until_processed` (`../climon2-embed/src/backlog.rs`), `congestion_stalls_at_the_deadline_and_not_before`, `a_drain_ends_congestion_and_each_client_is_timed_apart` (`../climon2-embed/src/stall.rs`), `a_client_whose_reports_stay_unprocessed_is_failed_after_its_delivery_stall`, `reports_the_ui_processes_leave_room_to_drain_the_client` (`src/supervisor.rs`) |
| LIM-06 (pure supplement, #119): frames at most once per interval at 1, 30 and 120 fps; fair turns within both quanta at their boundaries | `frames_are_due_at_most_once_per_frame_interval`, `an_earlier_instant_is_never_due` (`../climon2-monitor-tui/src/frames.rs`), `clients_take_fair_turns_within_both_quanta`, `a_lone_client_is_bounded_by_the_background_quantum` (`src/cadence.rs`) |
| LIM-03 (pure supplement over real terminal models, #119): the pressure order; the per-session, history-total and total budgets at the limit and one past, caches yielding in order down to their newest page and connected ones never; a history shortened a chunk at a time down to its newest page; a connection's end ends its errors | `unselected_retained_caches_yield_first_least_recently_inspected_first`, `a_budget_is_exceeded_only_past_it`, `caches_yield_in_order_until_the_budgets_hold` (`src/pressure.rs`), `history_is_shortened_a_chunk_at_a_time_down_to_its_newest_page`, `the_end_of_the_connection_ends_its_errors` (`src/terminal.rs`) |
| LIM-05 (Monitor, Fault: generation exhaustion; hooked only), REC-02 (#119): the protected cache survives and no attempt follows the last generation | `a_session_whose_generations_are_exhausted_keeps_its_cache_and_stops` (e2e, Fault peers) |
| LIM-05 (pure supplement over the fake library, #119): generations u64::MAX - 1 and u64::MAX, then rejected for good; request IDs u64::MAX - 1 and u64::MAX, then refused without submitting | `a_session_whose_generations_are_exhausted_is_never_attempted_again`, `requests_after_the_last_request_id_are_refused` (`src/supervisor.rs`) |
| TRM-01 (pure supplement): exact SGR for the 16 ANSI, palette, direct and default colours, underline colour (colon form, underlined cells only) and renditions; wide and combining text; each cell covers its given width; the cursor | `colours_and_renditions_are_painted_with_their_own_sgr`, `wide_and_combining_text_and_the_cursor_are_painted_in_place`, `each_cell_covers_the_width_it_was_given`, `each_row_starts_with_its_own_cursor_position` (`../climon2-monitor-tui/src/paint.rs`) |
| TRM-01 (pure supplement; #108's note on #112): each underline style with its own SGR 4 subparameter, single as plain 4, none on a cell not underlined; each style reaches its cell from the model | `each_underline_style_is_painted_with_its_own_sgr` (`../climon2-monitor-tui/src/paint.rs`), `each_underline_style_reaches_its_cell` (`../climon2-monitor-tui/src/screen.rs`) |
| TRM-01 (pure supplement, synchronized updates): while the session's update is open the canvas painted before it stays, cursor and underline styles included, never for another session or host size; the whole update shows once it ends | `an_open_synchronized_update_keeps_the_last_coherent_canvas` (`../climon2-monitor-tui/src/screen.rs`) |
| TRM-05, TRM-01 (pure supplement, #112): a scrolled canvas shows history rows above the screen's top rows; the cursor moves down and hides once scrolled out of view (the canvas's last row and one past it); the alternate screen shows its own rows; an evicted view shows the truncation notice, clipped to the session, above the oldest kept row; `History` in the control slot while scrolled back | `a_scrolled_canvas_shows_history_and_hides_the_cursor_out_of_view`, `an_evicted_view_shows_the_notice_above_the_oldest_kept_row`, `while_scrolled_back_the_control_slot_says_history` (`../climon2-monitor-tui/src/screen.rs`) |
| TRM-05 (pure supplement, #112): a vertical notch scrolls history a row without mouse reporting, whatever its modifiers; on the alternate screen a cursor key with alternate scroll, nothing without; horizontal notches, presses, releases, motion and any report under mouse reporting are the application's; input returns the view live | `the_wheel_scrolls_history_or_sends_cursor_keys_only_without_mouse_reporting` (`../climon2-monitor-tui/src/mouse.rs`), `the_view_scrolls_into_history_and_input_returns_it_live` (`src/terminal.rs`) |
| TRM-03, TRM-04 (pure supplement): the desired area; 4 rows and 5 columns enough, one fewer of either too small | `the_application_area_is_the_host_less_its_top_row_unless_too_small` (`../climon2-monitor-tui/src/screen.rs`) |
| Top row (owner decision 2026-11-14, pure supplement): fixed slots for status, control and hint whatever the control word; the error clipped first, never squeezing out the label | `status_and_control_keep_their_slots_whatever_the_control`, `an_error_is_clipped_first_and_never_squeezes_out_the_label` (`../climon2-monitor-tui/src/screen.rs`) |
| TRM-01 (pure supplement): the cursor only inside the host; a wide character the edge would cut left blank; each rendition reaches its cell; DECSCUSR for every shape | `the_cursor_shows_only_inside_the_host`, `a_wide_character_the_host_edge_would_cut_is_left_blank`, `each_rendition_reaches_its_cell`, `each_colour_reaches_its_cell_as_the_same_kind`, `each_cursor_shape_has_its_decscusr` (`../climon2-monitor-tui/src/screen.rs`) |
| Spec #73 Enter (pure supplement, #108, #110): each row's kind by condition (connected, unconnected, retained, own); Enter chooses the highlighted row | `enter_kinds_follow_each_row_condition` (`src/registry.rs`), `search_edits_keep_a_matching_highlight_or_take_the_first_match` (`../climon2-monitor-tui/src/selector.rs`) |
| Client supervisor (#108): control and display records reach the terminal as delivered; Acquire, Ready and Resize submitted in order and answered as themselves; a request refused at submit is not pending and consumes no ID; the pending ledger is bounded; a request before the client opens is refused as not connected | `control_and_display_records_reach_the_terminal_as_delivered`, `requests_are_submitted_in_order_and_answered_as_themselves`, `a_request_before_the_client_opens_is_refused_as_not_connected`, `a_request_refused_at_submit_is_not_pending_and_consumes_no_id`, `the_pending_ledger_is_bounded_by_the_requests_the_library_may_hold` (`src/supervisor.rs`) |
| SEL-01, DSC-05 (pure supplement): rows found after an empty open, or losing the highlighted row, leave nothing highlighted until Up or Down highlights the first row | `opening_highlights_the_selected_session_or_the_first_row`, `a_highlighted_row_that_leaves_or_stops_matching_is_unhighlighted_for_good` (`../climon2-monitor-tui/src/selector.rs`) |
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
| INP-02, HLP-02 (Monitor, Escape; owner decisions on #105, #106 and #109): a cursor key split right after its Escape or after its introducer never closes the selector or key introduction; a lone Escape closes it when the next key arrives, which reaches the session; Escape then a key in one read closes the selector or key introduction and the key reaches the session, painted first | `split_escape_sequences_never_close_dialogs_and_escape_then_a_key_is_both` (e2e), `only_closing_the_last_modal_reveals_the_canvas` (`../climon2-monitor-tui/src/lib.rs`) |
| Spec #73 exit (owner decisions on #109 and #118): keys read with prefix+x reach the session before shutdown; a stuck session holds exit at most the operation deadline, inside the shutdown deadline; EXT-02 and EXT-04 (#118): its connection failing during shutdown with no confirmed Leave exits 125 naming it | `typed_bytes_reach_the_selected_session_exactly_however_reads_split`, `a_stuck_session_cannot_hold_exit_beyond_the_operation_deadline` (e2e); `stopping_waits_for_accepted_input_at_most_the_operation_deadline` (`src/supervisor.rs`) |
| COM-03 (Monitor, queue full), COM-04 (Monitor, chunks): a refused chunk is shown with the bytes not sent, earlier input accepted; only accepted input arrives | `a_full_input_queue_is_shown_and_only_accepted_input_arrives` (e2e) |
| INP-03 (Monitor), COM-04 (Monitor, paste cases): pastes exact, binary, prefix commands, nested and broken markers included, with application markers only while it has bracketed paste on, off and on again; a pending prefix as one literal before the paste, none pending after it; markers split across reads | `pastes_reach_the_selected_session_exactly_with_markers_only_when_it_asks` (e2e) |
| INP-03 (Monitor, Fault; owner decision on #111), COM-04 (Monitor, paste chunks): a paste refused partway shows `Paste truncated`, its later read is discarded, and the application gets the start marker, whole chunks and the end marker before the next key | `a_paste_refused_partway_is_truncated_and_its_application_gets_the_end_marker` (e2e) |
| INP-04 (Monitor, Fault): revocation in the middle of a paste (the rest discarded even once ready again after a hand-back), revocation with a prefix pending (the take-control key then takes control), a new restoration from a host resize in the middle of a paste (the end marker once that restoration is ready, before later keys; owner decision on #111), and a too-small host in the middle of a paste (the end marker at once; the rest discarded once there is room again) | `a_target_change_discards_the_rest_of_a_paste_and_a_pending_prefix` (e2e) |
| INP-04 (Monitor, Fault: disconnect): the selected session's daemon killed with a prefix pending discards the prefix, so the next key is no exit and reaches no other session | `a_disconnect_discards_a_pending_prefix` (e2e) |
| INP-05 (Monitor: application, chrome and selector cases), INP-04 (Monitor: a modal in the middle of a gesture): clicks, a drag onto the top row and the wheel translated; a gesture begun on the top row stays local; the selector opening mid-drag discards the rest; selector clicks reach no application; single-byte encoding; reporting off | `mouse_events_reach_the_application_translated_and_keep_their_gesture` (e2e) |
| INP-05 (Monitor; owner decision on #111): the host mirrors the application's mouse tracking level (1003, 1000), back to the monitor's own (1002) while the application reports nothing, the selector covers it or another client's takeover has it, and again once it takes input | `the_host_mirrors_the_application_mouse_tracking_level` (e2e), `the_host_mouse_tracking_is_written_when_it_changes` (`../climon2-monitor-tui/src/screen.rs`) |
| FOC-01 (Monitor: application focus): focus follows menus, host focus and switching, once per change, the current focus once when reporting is enabled | `application_focus_follows_selection_menus_and_host_focus_once_each` (e2e) |
| FOC-02 (Monitor; owner decisions on #111): a host that never reports focus leaves sessions usable, and a session without `focus-v1` shows nothing about it in its top row | `without_focus_support_sessions_stay_usable_and_nothing_is_shown` (e2e) |
| TRM-05 (Monitor; owner decisions on #112), FOC-01 (Monitor, history case): the wheel scrolls history a row a notch while the application does not report the mouse, `History` in the control slot, the cursor hidden out of view; output keeps the view anchored; typing returns it live and reaches the application with no focus change from scrolling; at the oldest row of an untruncated history no notice; 210 rows with 200 kept evict the anchor to the notice above the oldest kept row, one notch down leaving it; on the alternate screen the wheel sends nothing, then cursor keys (normal, then application cursor mode) with alternate scroll on; no native host scrollback | `the_wheel_scrolls_anchored_history_and_typing_returns_live` (e2e) |
| TRM-06 (Monitor; owner decision on #112): a label filling its 55 cells exactly is whole and one a cell longer truncated with `…`; a wide character crossing the limit leaves a blank; combining marks stay with their base; an empty title keeps the latest nonempty one; a status change moves neither label nor slots | `overflowing_titles_are_truncated_statically_with_an_ellipsis` (e2e) |
| TRM-01 (Monitor; #108's note on #112): each underline style, cursor shape and blink, a hidden cursor, the 1049 alternate screen entered and left with the normal screen and cursor restored, and a synchronized update spanning two writes shown whole | `underline_styles_cursor_styles_screens_and_synchronized_updates_reach_the_host` (e2e) |
| TRM-04 (Monitor): under continuous output, resizes to a narrower host, one too small and back show only coherent frames (the last frame clipped or padded, `Terminal too small` alone, the session at its new size), and the session gets each size sent and none while too small | `resizes_under_continuous_output_show_only_coherent_frames` (e2e) |
| INP-01 (pure input-router supplement): the global actions with default and custom prefixes; a doubled prefix; unknown suffixes; a lone prefix waits | `the_prefix_and_lowercase_s_x_j_or_question_mark_are_the_global_actions`, `a_doubled_prefix_is_one_literal_prefix`, `an_unknown_suffix_is_the_prefix_and_that_key`, `a_lone_prefix_waits_indefinitely` (`../climon2-monitor-tui/src/router.rs`) |
| INP-02 (pure input-router supplement): every split routes identically, and sends an application the same bytes even with a released Escape; a split cursor key is never an Escape; Escape and a key; a lone Escape waits for the next byte, and one bound for an application is released at once, once, never as or after a prefix; Ctrl+[ as the prefix; broken and overlong (64 and 65 bytes) sequences; cursor and keypad encodings | `every_split_of_the_reads_routes_identically`, `a_split_cursor_key_is_never_an_escape`, `escape_and_a_key_in_one_read_are_escape_then_that_key`, `a_lone_escape_waits_for_the_next_byte`, `a_lone_escape_for_an_application_is_released_at_once`, `an_application_receives_the_same_bytes_for_every_split`, `with_escape_as_the_prefix_bare_escape_waits_and_alt_keys_are_actions`, `a_broken_sequence_is_its_bytes`, `an_overlong_sequence_is_discarded`, `cursor_keys_follow_the_application_cursor_mode`, `keypad_keys_follow_the_application_keypad_mode` (`../climon2-monitor-tui/src/router.rs`); `the_host_keypad_mode_is_written_when_it_changes` (`../climon2-monitor-tui/src/screen.rs`) |
| TRM-02, INP-03, INP-05, FOC-01 (pure input-router supplement): replies, mouse and focus reports and pastes are never keys and never complete a pending prefix; a paste discards one, as a prefix it begins after | `replies_reports_and_pastes_are_never_keys`, `mouse_and_focus_reports_never_complete_or_clear_a_pending_prefix`, `a_paste_discards_a_pending_prefix_and_leaves_none` (`../climon2-monitor-tui/src/router.rs`) |
| INP-03 (pure input-router supplement): pasted bytes exact, binary and broken end markers included, however reads split; each read's part, holding back only what may begin the end marker | `pasted_bytes_are_exact_however_reads_split`, `each_read_brings_its_part_of_a_paste` (`../climon2-monitor-tui/src/router.rs`) |
| INP-04 (pure supplement): a target change discards a pending prefix; a gesture whose owner stops taking input is discarded through its release; a bracketed paste cut short owes its end marker once under the current authority of the same control period, never after it (owner decision on #111) | `a_target_change_discards_a_pending_prefix` (`../climon2-monitor-tui/src/router.rs`), `a_gesture_whose_owner_stops_taking_input_is_discarded_through_its_release` (`../climon2-monitor-tui/src/mouse.rs`), `a_cut_paste_owes_its_end_marker_under_the_current_authority` (`src/terminal.rs`), `a_closing_part_ends_a_cut_paste_under_the_current_authority` (`../climon2-embed/src/stream.rs`) |
| INP-05 (pure supplement): SGR reports parsed, malformed ones not; clicks translated below the top row at the edges and one past; gesture ownership; the events each reporting mode asks for; SGR, UTF-8 and single-byte encodings at their limits and one past | `sgr_reports_parse_and_malformed_ones_do_not`, `a_click_inside_the_application_is_translated_below_the_top_row`, `each_gesture_keeps_the_target_its_press_found`, `an_application_receives_only_the_events_its_mode_asks_for`, `each_encoding_is_written_as_the_application_set_it` (`../climon2-monitor-tui/src/mouse.rs`) |
| INP-03, COM-03, COM-04 (pure supplement; owner decision on #111): input in chunks with every unsent byte carried; a paste exact between markers only when bracketed; a truncated paste reported once, its rest discarded, its application owed exactly the rest of the start marker and the end marker, paid when the library has room and before later input | `input_goes_in_chunks_and_a_refusal_carries_every_unsent_byte`, `a_paste_is_exact_between_the_markers_only_when_bracketed`, `a_truncated_bracketed_paste_owes_its_end_marker_and_discards_the_rest`, `a_truncated_paste_owes_only_what_its_application_lacks`, `a_paste_truncated_after_one_byte_of_its_start_marker_owes_the_rest`, `a_new_paste_waits_behind_what_an_earlier_one_owes` (`../climon2-embed/src/stream.rs`); `a_refused_paste_shows_that_it_was_truncated` (`src/terminal.rs`) |
| FOC-01, FOC-02 (pure supplement): Focus once per desire under ready authority, a newly ready restoration's current desire once, none without `focus-v1`, control or a running command; `focus-v1` lasts until the client ends; Open says whether `focus-v1` was selected; Focus and paste parts as their native requests | `focus_follows_each_new_desire_under_ready_authority_once`, `focus_support_lasts_until_the_client_ends` (`src/terminal.rs`), `opening_says_whether_focus_and_status_were_selected` (`src/supervisor/client.rs`), `focus_and_paste_parts_become_their_native_requests` (`../climon2-embed/src/instance.rs`) |
| COM-04, EXT-03 (pure input-router supplement): typed bytes reach an application exactly | `typed_bytes_reach_the_application_exactly` (`../climon2-monitor-tui/src/router.rs`) |
| COM-03, COM-04 (pure supplement): chunks of at most `chunk_bytes`; the first refusal carries every unsent byte and consumes no request ID; a refusal is shown | `input_is_submitted_in_chunks_and_a_refusal_carries_every_unsent_byte` (`src/supervisor.rs`), `refused_input_shows_what_was_not_sent` (`src/terminal.rs`) |
| REC-01 (Monitor, Fault: transport loss), REC-02 (Monitor, Fault: full history; `Reconnecting...` while scheduled and while running; the placeholder after the reconnect), COM-03 and COM-05 (Monitor: indeterminate work never replayed, a fresh attempt sent nothing old), DSC-05 (Monitor, Fault: loss keeps the selection), Enter on disconnected and unconnected Starting rows (derived) (#116) | `a_lost_connection_keeps_its_history_read_only_until_a_passive_reconnect` (e2e, Fault peers) |
| REC-02 (Monitor: full history, no history), REC-01 (Monitor: a gone daemon and a known outcome stop retry), HDL-01 (Monitor case: `c2 kill` from another terminal), COM-06 (Monitor), DSC-05 (Monitor: every row final or gone, discovery continues), Enter on final rows (derived), global actions from the read-only screens (#116) | `killed_and_ended_sessions_stay_read_only_and_are_never_retried` (e2e) |
| REC-01 (Monitor, Fault: missing library, repaired installation, permanently rejected protocol, visible exponential retry with jitter and cap), LIB-02 (Monitor), DSC-05 (Monitor, Fault: every row unavailable) (#116) | `failed_clients_retry_with_backoff_until_repaired_or_rejected_for_good` (e2e, Fault peers) |
| REC-01 (Monitor, Fault: contained panic), LIB-05 (Monitor: the panicking instance stays on its row) (#116) | `a_contained_panic_is_retried_on_its_own_row` (e2e, hooked only) |
| REC-01, COM-05 (pure supplement over the fake library): a failure is retried in the next generation only after the old instance is destroyed and its owner joined, with backoff doubling to the cap, from the start again after an open; permanent rejections, invalid configuration, known outcomes and unconfirmed cleanup retry nothing, scarce resources are retried, also a client the monitor stopped itself, and an Open refusal is Unavailable with the refusal; a missing library is loaded again when due, a wrong release never; a session proven gone is never retried, a retry already scheduled dropped; each loader refusal, create refusal and terminal evidence's retry class (#116) | `a_failed_client_is_retried_after_its_cleanup_in_a_fresh_generation`, `rejections_known_outcomes_and_unconfirmed_cleanup_retry_nothing`, `a_missing_library_is_loaded_again_when_its_retry_is_due`, `a_session_proven_gone_is_never_retried`, `a_scheduled_retry_is_dropped_once_its_session_is_gone`, `only_a_refusal_a_later_load_may_not_repeat_is_retried` (`src/supervisor.rs`), `a_retry_class_tells_the_row_whether_it_is_retried_or_rejected` (`src/supervisor/client.rs`), `create_refusals_and_terminal_evidence_say_whether_to_retry` (`../climon2-embed/src/instance.rs`), `only_a_refusal_a_later_load_may_not_repeat_is_retried` (`../climon2-embed/src/load.rs`) |
| REC-01 (pure supplement): the delay doubles from `retry.initial_ms` to `retry.cap_ms` (at the cap and one doubling past it), saturating; jitter over the whole spread at its ends and one past, within the cap; fresh random values (#116) | `the_delay_doubles_from_its_start_to_its_cap`, `jitter_spreads_each_delay_either_way_within_the_cap`, `each_random_value_is_fresh` (`src/supervisor/backoff.rs`) |
| REC-02 (pure supplement): rows keep recovery through a retry's Starting, never a first start; a reconnect clears recovery and the old stale status (#107's review); rejection replaces retrying; a gone session stops recovery for good, not for connected or final rows; the last applied content is retained read-only across generations, none before one; only rows neither connected nor final show it; the placeholder, history or no content under the overlay; the overlays (one wider than the canvas clipped) and diagnostic; the wheel scrolls read-only history on its normal screen only; a gone session says so in its error slot over its terminal's own error, such as a request its lost connection cancelled (#116) | `recovery_follows_a_lost_client_until_it_reconnects_or_its_session_is_gone` (`src/registry.rs`), `the_last_applied_content_is_retained_read_only` (`src/terminal.rs`), `a_selected_row_without_a_view_shows_the_placeholder_its_history_or_no_content`, `only_a_row_neither_connected_nor_final_shows_its_retained_history`, `a_gone_session_says_so_in_its_error_slot_over_a_cancelled_request` (`src/session.rs`), `the_wheel_scrolls_read_only_history_only_on_its_normal_screen` (`../climon2-monitor-tui/src/lib.rs`), `a_lost_session_shows_its_history_read_only_under_its_overlay` (`../climon2-monitor-tui/src/screen.rs`) |
| NOT-03 (Monitor and Fault), INP-01 (Monitor: prefix+j from the empty, selector, help, placeholder and live screens), STA-08 (Monitor, jump case): the newest connected attention chosen at the keypress, overlays closed, control taken only when not held, acknowledged once ready; acknowledged and disconnected rows never chosen; with none, `No sessions need attention` in the body for 3 seconds, or the top row's error slot with the selection kept | `prefix_j_jumps_to_the_newest_connected_attention_and_acknowledges_it_once_ready` (e2e) |
| STA-08 (Monitor: acknowledgement timing), SEL-03 (Monitor, acknowledgement case), FOC-01 (Monitor, viewing cases), NOT-03 (deferred acknowledgement cancelled), INP-01 (prefix+j from history and too-small screens): visibility, closing help and host focus-in never acknowledge; cancelling the selector while the host is unfocused or the view scrolled back acknowledges only once focus returns or the view is live; a modal opening cancels a deferred acknowledgement; prefix+j on a too-small host acknowledges once there is room, from history once live | `acknowledgement_waits_until_viewed_and_closing_help_or_focus_in_never_acknowledges` (e2e) |
| SEL-03 and STA-08 (Monitor, cancel case): cancelling the selector back to a covered session that needs attention acknowledges it once ready | `status_keeps_updating_for_background_covered_and_revoked_rows` (e2e) |
| NOT-04 (Contract and Monitor): exact root OSC 7501 bytes, the initial state and each aggregate change only, disconnected rows not counted, no session report or bell forwarded, the root clear on exit; every released scenario checks the host heard only those | `the_host_hears_the_attention_state_once_per_change_and_the_clear_on_exit` (e2e); `assert_host_reports` in every `assert_released` |
| STA-01, STA-03, STA-04, STA-05 (Monitor: representative sequences through a real daemon), INP-01 (Monitor: uppercase J and a pasted prefix+j are input): root `working`, `idle`, `blocked`, `done`, `error` and `clear`, OSC 133 boundaries and accepted input show the right row states | `representative_status_sequences_show_the_right_row_states` (e2e) |
| NOT-03, NOT-04 (pure supplement): the jump target by newest change then full session ID, excluding acknowledged, final, disconnected and not-connected rows | `the_attention_target_is_the_newest_connected_row_needing_attention` (`src/registry.rs`) |
| NOT-03, NOT-04, STA-08 (pure supplement): host reports' exact bytes, initial state and changes only; the notice up to 3 seconds and not at them; a deferred acknowledgement submitted once viewed, kept across restorations of its period, cancelled on a context change | `the_host_hears_the_initial_attention_state_and_changes_only`, `the_notice_shows_for_three_seconds`, `a_deferred_acknowledgement_submits_once_its_target_is_viewed`, `a_deferred_acknowledgement_is_cancelled_when_its_target_context_changes` (`src/attention.rs`) |
| FOC-01, STA-08 (pure supplement): Set viewing on each change within a control period under the current authority, only with `status-v1`; a superseded Acknowledge owed again; Set viewing and Acknowledge as their native requests; Open says `status-v1` was selected | `viewing_is_reported_on_each_change_within_a_control_period`, `an_acknowledge_a_newer_restoration_superseded_is_owed_again` (`src/terminal.rs`), `viewing_and_acknowledge_become_their_native_requests` (`../climon2-embed/src/instance.rs`), `opening_says_whether_focus_and_status_were_selected` (`src/supervisor/client.rs`) |
| FOC-01, STA-08 (pure supplement; owner decision on #114, 2026-11-17): Focus and Set viewing coalesce per session: none of a kind while one is unanswered, then the latest desire once if it differs from the last sent, each kind waiting only for its own answer; a revocation keeps waiting for the answer, the end of the client does not; an Acknowledge follows only Set viewing true sent in the held control period | `focus_and_viewing_wait_for_their_answers_and_then_send_the_latest_desire`, `only_the_end_of_the_client_forgets_unanswered_focus_and_viewing`, `an_acknowledge_follows_only_viewing_true_submitted_in_the_period` (`src/terminal.rs`) |
| NOT-03 (pure supplement): the notice in the top row's error slot before the session's own error, or in the body before the startup progress | `a_late_collection_shows_in_the_error_slot_unless_the_session_has_its_own`, `the_body_says_the_startup_progress_or_the_state_and_any_incompleteness` (`../climon2-monitor-tui/src/screen.rs`) |
| NOT-01 (Monitor), NOT-02 (Monitor: the queue), INP-05 (Monitor: toast clicks), with fake daemons: a final known at the first connection never toasts; attention pending at startup toasts once with the hint, its title held and its bell rung only once written however late the host reads the startup output; six episodes in a burst show three toasts and then `More events occurred. Check sessions list`; a final learned while running toasts once without a hint; clicks only dismiss; menus toast nothing again; one bell per toast | `toasts_announce_each_episode_and_final_once_and_queue_three_then_more` (e2e) |
| STA-06 (Monitor), CTL-03 (Monitor: notifications), NOT-01 (Monitor: a final learned while viewed), NOT-02 (Monitor: shape, ellipsis, bell), TRM-03 (Monitor: toast resize): progress and incomplete OSC 99 toast nothing; OSC 9 toasts once, yellow, bottom-right, with the hint; resized to 80x25 it is 32 cells with `…`, no second bell; OSC 777 and OSC 99 within the pending episode toast nothing; a final learned while viewed never toasts, even once a menu opens; a new episode toasts again; prefix+x exits while one draws | `notifications_toast_once_per_episode_and_progress_never_does` (e2e) |
| FOC-01 (Monitor: toast cases), NOT-01 (Monitor: menu opening), INP-05 (Monitor: toast cases): attention seen while viewed toasts once the selector opens; a covering toast defers the cancel's acknowledgement until no toast covers the session; keys reach the session under a toast; a drag begun before a toast keeps its target over it; a click on the toast only dismisses it | `a_covering_toast_stops_viewing_while_keys_and_drags_still_reach_the_session` (e2e) |
| CFG-01 (Monitor: hints and bell): with both off, no hint in the empty state, top row, selector, placeholder or toast and no key introduction or bell, while every binding works and toasts and host reports go on | `without_key_hints_or_bell_no_hint_shows_and_no_bell_rings` (e2e) |
| NOT-01, NOT-02, FOC-01, INP-05 (pure supplement, #115): once-per-run deduplication and viewed suppression; the three-toast queue and its overflow toast; the timings at each boundary; the hold and bell starting when the full title has been written; no title, so no bell, for a toast dismissed before it; early dismissal; placement and width at 40%; drawing, colour, dimming and erasing; clockwise border order; covering, hits, clicks and dismissal over a covered canvas; wide characters at the left edge; gestures under way | `each_episode_and_final_outcome_toasts_once_unless_viewed`, `the_queue_holds_three_toasts_and_one_more_stands_for_the_rest`, `the_hold_starts_when_the_title_has_been_written`, `a_toast_dismissed_before_its_title_never_titles`, `a_dismissed_toast_ends_early_and_the_next_begins` (`src/toast.rs`), `a_toast_draws_holds_fades_and_erases_on_time`, `an_early_dismissal_erases_only_what_was_drawn`, `a_toast_sits_bottom_right_at_most_forty_percent_wide`, `a_toast_waits_until_it_fits`, `a_toast_draws_its_border_label_and_hint_in_yellow`, `the_border_runs_clockwise_from_the_top_left_corner`, `the_shown_toast_covers_and_takes_only_its_own_cells`, `the_toast_takes_reports_outside_gestures_and_a_click_dismisses_it`, `a_toast_is_dismissed_over_a_canvas_it_covers`, `a_wide_character_cut_by_the_toast_is_blanked` (`../climon2-monitor-tui/src/toast.rs`); `attention_episodes_and_finals_learned_while_running_are_announced` (`src/registry.rs`); `each_gesture_keeps_the_target_its_press_found` (`../climon2-monitor-tui/src/mouse.rs`) |
| DSC-02 (Monitor), LIM-06 (Monitor, start case): stalled newer starts with one start slot and a burst of one start newest first with the oldest's turn after each newer start; every session started; the selector lists them and prefix+x exits meanwhile (#120) | `stalled_newer_starts_give_the_oldest_session_its_turn_while_the_monitor_responds` (e2e) |
| DSC-02, LIM-06 (pure supplement): newest first with ascending ID ties; the oldest's turn after exactly the burst, not before; continuous newer arrivals never starve the oldest; retries in arrival order alternate with first starts; ineligible sessions (stalled release, admission interval) keep their place and block no other (#120) | `first_starts_run_newest_first_with_ascending_id_ties`, `the_oldest_gets_a_turn_after_each_burst_of_newer_starts`, `continuous_newer_arrivals_never_starve_the_oldest`, `retries_and_first_starts_take_turns`, `ineligible_sessions_keep_their_place_and_block_no_other` (`src/supervisor/queue.rs`) |
| DSC-02 (Fault: one release stalled), Linux amendment, `deadlines.load_ms` owner decision: a FIFO-stalled load shows `library loading slowly`, never aborted; the loaded release's running client still reports, the selector and exit respond; once the open returns the release is refused and a session created meanwhile starts (#120) | `a_stalled_release_load_is_reported_and_never_holds_up_the_monitor` (e2e); `a_slow_load_is_reported_and_never_blocks_a_loaded_release` (`src/supervisor.rs`) |
| LIM-04 (Monitor, Fault: memory), relaxed qualification (raised reservation), Awaiting resources rows (not-connected order, Enter read-only with condition and diagnostic, never acquired, placeholder once admitted until the take-control key acquires) (#120) | `a_resource_blocked_session_awaits_resources_until_capacity_is_freed` (e2e) |
| LIM-04 (Monitor, Fault: operating-system limits): the soft descriptor limit less the headroom bounds admission; raising it admits the blocked session (#120, Linux) | `the_operating_system_descriptor_limit_bounds_admission_until_it_rises` (e2e); `the_descriptor_headroom_below_the_os_limit_bounds_admission` (`src/supervisor.rs`) |
| LIM-04 (pure supplement): reservations at capacity and one past, for memory and descriptors; one reservation per holder (shared release counted once, raised in place, kept when a raise fails); release and load thread together or neither; freeing; configured, loader, qualified (greater) and per-instance amounts; partially started instances held until joined; a blocked session admitted once capacity is freed; the release reserved before load and raised before create at the boundary, its handle loaded once (#120) | `a_reservation_is_admitted_at_its_capacity_and_refused_one_past_it`, `each_holder_counts_once_until_freed`, `holdings_reserved_together_are_admitted_whole_or_not_at_all`, `a_release_reserves_its_configured_amount_then_the_greater_qualified_one`, `an_instance_reserves_its_share_stacks_and_descriptors` (`../climon2-embed/src/admission.rs`), `a_session_admission_cannot_reserve_is_resource_blocked_until_capacity_is_freed`, `a_release_is_raised_to_its_qualified_reservation_before_any_create` (`src/supervisor.rs`) |
| LIM-01 (Monitor), relaxed qualification: qualified workers above `resources.max_workers_per_client` and a library share too small for the qualified instance allocation rejected explicitly, at the limit and one past, before any create (#120) | `too_many_qualified_workers_is_an_explicit_rejection_before_any_create` (e2e); `too_many_qualified_workers_is_an_explicit_rejection` (`src/supervisor.rs`), `unsupported_release_requirements_are_rejected_explicitly` (`../climon2-embed/src/admission.rs`) |
| Awaiting resources rows (pure supplement): label, not-connected order by entry time kept across reasons, diagnostic, settled for the startup wait, never eligible or the attention target; loading slowly keeps Starting and awaited (#120) | `awaiting_resources_rows_are_settled_and_never_chosen` (`src/registry.rs`), `a_resource_blocked_row_is_settled_for_the_startup_wait` (`src/initial.rs`) |
| DEL-01 (Monitor: default Cancel, nothing sent on cancel, no input leaking out of the dialog, identity), DEL-02 (Monitor: removal only after definitive termination, a gone session's Remove, dismissal against rediscovery of stale metadata over two passes, a non-acquiring fallback, logs kept) (#117; Remove for a gone session, owner decision on #116) | `delete_terminates_or_removes_only_the_confirmed_session` (e2e) |
| INP-01 and NOT-03 (Monitor, Delete-dialog cases: prefix+s and prefix+? inert, prefix+j cancels and jumps, prefix+x exits), cancelling Delete never acknowledges (confirmed scenario) (#117) | `inside_the_delete_dialog_prefix_j_cancels_and_jumps_and_prefix_x_exits` (e2e) |
| DEL-01 (Fault: repeated Delete deduplicated, other sessions usable), DEL-02 (Fault: a rejected Kill keeps its row and error) (#117) | `a_held_kill_is_sent_once_and_a_refused_one_keeps_its_row_with_why` (e2e, Fault peers) |
| DEL-02 (Fault: a lost Kill never replayed, the passive client learns the outcome, cleanup and join before dismissal, dismissal against rediscovery of a still-answering daemon, a fallback never sent Acquire) (#117) | `a_lost_kill_is_never_sent_again_and_removal_waits_for_the_outcome_and_cleanup` (e2e, Fault peers) |
| DEL-01 (Monitor and Fault: finishing while asked switches to Remove for the same session through a reorder), DEL-02 (Fault: an already-final row's cleanup before dismissal, no Kill) (#117) | `a_session_ending_while_asked_is_only_removed_after_its_cleanup` (e2e, Fault peers) |
| DEL-01 (Fault: the dialog closes with its dropped row, retargeting nothing) (#117); EXT-02 (Monitor, Fault: a client still in its Open at exit fail-stops, naming its session) (#118) | `a_dialog_whose_row_is_dropped_closes_without_retargeting` (e2e, Fault peers) |
| DEL-01, DEL-02 (pure supplement over the fake library): one Kill through an open client or a Kill-only instance (no Open, request 2, its release loaded first), removed only after cleanup and join; a refused Kill (the client's, a Kill-only instance's or its release's) ends the Delete and restarts a held retry; a lost one never resent, learned by a passive retry; a final or gone session removed without a Kill; unconfirmed cleanup and an unanswered Kill reported (#117) | `a_confirmed_kill_removes_its_session_only_after_cleanup`, `a_refused_kill_ends_its_delete_and_a_lost_one_is_never_sent_again`, `a_final_or_gone_session_is_removed_without_a_kill_after_its_cleanup` (`src/supervisor.rs`) |
| DEL-01, DEL-02, LIM-04, LIM-05 (pure supplement over the fake library; #117 with #119 and #120): a Kill-only instance is reserved by admission and takes a start slot like any start, ahead of the queue; one admission refuses is reconsidered once per `retry.admission_ms`; a Kill takes the client's last request ID and is refused unsent after it; a Kill-only instance with no generation left refuses its Kill and frees its reservation | `a_kill_only_instance_waits_for_admission_like_any_instance`, `a_blocked_kill_only_instance_waits_out_its_admission_interval`, `a_kill_only_instance_waits_for_a_start_slot`, `a_kill_after_the_last_request_id_is_refused_unsent`, `a_kill_only_instance_without_a_generation_refuses_its_kill` (`src/supervisor.rs`) |
| DEL-01, DEL-02 (pure supplement): offers, confirmation, Terminating and Removing, refused and unconfirmed Kills, run-local dismissal; answers' verdicts and a Kill's carried result; Kill requires no family; the dialog's keys, rendering and Delete key; fallback selection and highlight (#117) | `delete_offers_remove_once_final_or_gone_and_dismissal_is_run_local` (`src/registry.rs`), `answers_say_how_they_ended_and_carry_a_kills_command_result` (`src/supervisor/client.rs`), `a_kill_requires_no_family` (`../climon2-embed/src/instance.rs`), `only_forward_delete_asks_for_a_confirmation`, `cancel_has_the_focus_until_tab_or_the_arrows_move_it`, `the_confirmation_shows_the_label_status_and_offer_with_cancel_focused` (`../climon2-monitor-tui/src/delete.rs`), `a_removed_row_falls_back_to_its_place_then_the_row_before` (`src/delete.rs`), `a_dismissed_highlighted_row_falls_back_among_the_matching_rows`, `help_lists_every_selector_key_with_the_actual_prefix` (`../climon2-monitor-tui/src/selector.rs`) |
| #74 risk proof: plain and hooked sibling binaries without nested builds | `cargo_builds_the_plain_or_hooked_monitor_binary_for_its_tests` (`tests/build.rs`, plain and hooked runs); fixtures `workspace_binaries_contain_test_hooks`; xtask `release_check_builds_unhooked_binaries_in_a_private_target` |
| #74 release checks: version, architecture and test-hook marker | xtask `version_check_rejects_a_missing_or_wrong_version_monitor`; Linux `linux_packaging_rejects_a_non_x86_64_or_hooked_monitor`; macOS `binary_check_rejects_a_single_architecture_monitor`, `binary_check_rejects_a_hooked_monitor` |

Not yet claimed: NOT-01's reconnect case (the
toast ledger lasts the run, but no scenario yet reconnects a session after its toast), a toast beside a canvas narrower or shorter than the
host, which leaves the session viewed (pure supplement only, `a_toast_is_dismissed_over_a_canvas_it_covers`), FOC-02's host-native
copy (a host manual check), INP-04's disconnect case in the middle of a
paste or gesture (the selection stays on the lost session, so nothing
else could receive the rest, and a reconnect replays none of it),
`discovery.stall_ms` and mandatory identity exhaustion's coordinated failure
(#120's open questions; #118's fail-stop does not cover the latter), new clients of a loaded
release during a stalled load on glibc (#120's open question), and the universal archive's runtime on both architectures, which
only CI's `dist` and `archive` jobs build and execute.
