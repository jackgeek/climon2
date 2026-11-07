# Throwaway monitor selector

Interactive primary source for [Shape the overlaid session selector](https://github.com/jackgeek/climon2/issues/60).
This branch is **not for merging into main**. Everything is synthetic and
in-memory; there is no shipped monitor, real session discovery, daemon/client
integration, configuration parser, VT renderer or library interface here.

From this worktree:

```sh
cargo run --manifest-path prototypes/selector/Cargo.toml
```

It opens the selector over a changing synthetic terminal. Compare layouts with
**F2**: **A** list above details, **B** list beside details, **C** details above
two-line session cards. All retain the confirmed centered-overlay interaction.
Start directly with `-- --variant B` or `-- --variant C`.
Narrow terminals collapse to a list with an explicit Details action. Visual
proportions and compact breakpoints are experimental, not final release budgets.

**Ctrl+] then s** opens the selector; **Ctrl+] then x** exits;
**Ctrl+] then j** selects the newest connected attention request.
Use arrows/PageUp/PageDown to navigate, typing or paste to search, Enter to
select, Escape to cancel, and Tab/Shift+Tab to reach explicit actions.
Clicking a row only highlights; clicking Select confirms. Wheel scrolls the
list without changing the highlighted identity. The `*` marker is the currently
selected session, cyan is the highlight. Repeated selector-open is a no-op.
Search resets on opening and matches case-insensitive literal title or command
arguments without changing status/time ordering. Paste normalizes whitespace
and drops controls; it never runs shortcuts.

Details show only title/command fallback, working directory and status, including
known exit code or signal for failed sessions. The explicit Details view supports
Up/Down or wheel scrolling for long text. Back returns to the list.
Identical titles are distinguished by their directories in details, not GUIDs.
Future user-assigned names will augment title display/search when naming ships;
this prototype adds no naming or persistence.

Inspect opens cached content read-only without acquiring control. Escape or
Tab then Enter on Return to session leaves inspection without takeover.
Disconnected normal selection stays disconnected; final sessions display
retained output directly read-only. A passive session without cache cannot be
inspected. Control retained by background synthetic sessions keeps their output
changing under the selector.

Developer-only scenario controls always remain visible, separate from monitor hints:

| Key | Scenario |
| --- | --- |
| F3 | Toggle all implicit monitor hints |
| F4 | Highlighted/selected session requests attention and reorders |
| F5 | Change its daemon title; try this while filtering |
| F6 | Toggle its connection, revoking synthetic control |
| F7 | Remove the row; Enter cannot select a replacement automatically |
| F8 | Reset synthetic fixtures |

Launch with `-- --no-hints` to start without implicit hints. The prototype offers
only two escape-prefix presets, default Ctrl+] and
`-- --escape-prefix 'Ctrl+['`. Ctrl+[ is Escape: bare Escape waits and legacy
Alt+s/x/j is indistinguishable from the corresponding monitor shortcut. Use
explicit Cancel/Back/Return actions via Tab/Enter or mouse instead of Escape.

No real application input is sent. A yellow developer panel reports selection,
highlight, menu/cache state, synthetic input count and the last action.
Synthetic acquisition/restoration/paint completes immediately; it does not
validate real readiness, Focus, attention protocol delivery or takeover races.
The removal/disconnect scenarios validate selector identity handling only.
The prototype does not implement native terminal-crash isolation, monitor-wide
lifecycle fallback, signal handling, notification presentation, marquee, exact
VT/history fidelity, production memory/input budgets or config validation.
These remain with the existing contracts and owning decision tickets.

Automated checks:

```sh
cargo test --manifest-path prototypes/selector/Cargo.toml
cargo clippy --manifest-path prototypes/selector/Cargo.toml --all-targets -- -D warnings
```

Rust tests exercise selector decisions, keyboard/mouse isolation and Ratatui
frames. Host PTY tests use the same Alacritty 0.26.0 engine and portable-pty
components as the repository's real-terminal harness, checking the displayed
screen rather than searching differential repaint bytes.
The PTY checks drive the actual executable's launch, paste, cancellation,
fragmented shortcuts, resize, legacy Alt exit and normal terminal cleanup.
They are prototype evidence, not automated acceptance of a real monitor.
