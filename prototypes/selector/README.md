# Throwaway monitor selector

Interactive primary source for [Shape the overlaid session selector](https://github.com/jackgeek/climon2/issues/60).
This branch is **not for merging into main**. Everything is synthetic and
in-memory: no shipped monitor, real session discovery, daemon/client integration,
configuration parser, VT renderer or library interface.

From this worktree:

```sh
cargo run --manifest-path prototypes/selector/Cargo.toml
```

The original A/B/C comparison is preserved in commit `1aa0124`. The developer
chose B as the starting point and requested this replacement: double outer
border without captions, search on the first interior line, a single separator
joined by U+255F/U+2562, and borderless status/title columns. There is no details
pane/view, buttons, current-session marker or bottom monitor hints.
Search always owns keyboard focus. Tab does nothing. Arrows/PageUp/PageDown
change the highlight; Enter explicitly selects it; Escape cancels.
Mouse clicks highlight only; wheel scrolls the list.

**Ctrl+] then s** opens the selector; **Ctrl+] then x** exits;
**Ctrl+] then j** selects the newest connected attention request.
Search resets on opening and matches case-insensitive literal title or command
arguments while preserving status/time order. Paste normalizes whitespace and
drops controls without running shortcuts. Reordering preserves highlighted
identity; asynchronous removal/nonmatching titles clear the highlight rather
than redirecting Enter to another session.

Enter on a connected nonfinal session acquires control if needed and shows the
synthetic live terminal. Enter on a disconnected/final session opens its retained
history read-only without acquisition. Chrome explicitly shows Disconnected,
Succeeded or Failed (with the known exit code/signal). Missing cache is explicit,
never an empty pretend terminal. Wheel browses retained rows; read-only input
never reaches the synthetic application.

Future user-assigned names will augment titles and search when naming ships;
this prototype adds no naming or persistence. Removing details means duplicate
titles are no longer distinguished by working-directory information here.
Visual review and final specification confirmation remain open.

An initial synthetic attention toast appears bottom-right. Its yellow border
draws clockwise from the top-left over 400 ms, then the title appears for 2 s,
fades to the black background over 300 ms, and the border erases clockwise over
400 ms. Click to dismiss without takeover/acknowledgement. Distinct episodes
queue; repeated attention while already pending does not duplicate a toast.
Toast width is at most 40% of host width; overflow ellipsizes in display cells.
It has 3 rows without hints or 4 rows with its configured-prefix+j hint.
On narrow hosts the hint uses `^]+j` / `^[+j` control-key notation.
The selector reserves a stable toast lane even while no toast is present.
These timings and dimensions are prototype trials, not final release budgets.

Developer controls, outside the selector and toast:

| Key | Scenario |
| --- | --- |
| F3 | Toggle the toast hint row |
| F4 | Request attention for highlighted/selected session; queue a toast for a new episode |
| F5 | Change its daemon title; try this while filtering |
| F6 | Toggle connection, revoking synthetic control |
| F7 | Remove the row; Enter cannot select a replacement automatically |
| F8 | Reset fixtures and initial attention toast |

Launch with `-- --no-hints` to start without the toast hint row.
The two escape-prefix presets are default Ctrl+] and
`-- --escape-prefix 'Ctrl+['`. Ctrl+[ is Escape: bare Escape waits and legacy
Alt+s/x/j is indistinguishable from the corresponding shortcut.
The developer explicitly accepted that bare Escape cannot cancel the selector
with this preset; prefix+x still exits and prefix+j navigates attention.
No alternate Cancel action is supplied.

The yellow developer panel reports selection, highlight, menu/history state,
synthetic input count and last action. Synthetic acquisition/restoration/paint
completes immediately: this does not validate real readiness, Focus, attention
delivery or takeover races. The prototype omits native crash isolation,
monitor-wide lifecycle fallback, signal handling, marquee, exact VT/history
fidelity, production budgets and configuration validation. Synthetic toasts
demonstrate presentation/coalescing, not real daemon notification delivery.

Automated checks:

```sh
cargo test --manifest-path prototypes/selector/Cargo.toml
cargo clippy --manifest-path prototypes/selector/Cargo.toml --all-targets -- -D warnings
```

Behavior tests exercise selector identities, input isolation, read-only
selection, exact Ratatui borders/columns and deterministic toast animation.
Host PTY tests use the repository harness's Alacritty 0.26.0 / portable-pty
components and inspect displayed screens, not raw differential repaint text.
They drive launch, paste, cancellation, fragmented shortcuts, resize, legacy
Alt exit and normal terminal cleanup. This is prototype evidence, not automated
acceptance of a real monitor.
