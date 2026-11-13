# climon2-vt

The session daemon's virtual terminal (ADR 0007): a pure, platform-neutral,
infallible adapter around unchanged `alacritty_terminal` 0.26.0. No I/O, no
`cfg(target_os)`, and no Alacritty type in the public API. Only
`climon2-daemon` and `climon2-monitor` may depend on it (ADR 0004); the monitor
uses it today only for the documented normal-shell baseline in its release.

**Agreed, not implemented:** the
[#63 terminal contract](../../docs/protocol/terminal-integration.md) plans
monitor-owned models and read-only Rust cell/mode/cursor/history views for Ratatui.
This would extend the monitor's baseline-only use; it is not a current public
cell API or native cross-release cell schema. Monitor-mode daemon presentation must be
serialized at effective session geometry; ordinary host direct/derived
presentation below is unchanged. The daemon remains the sole query authority;
the monitor never forwards parser replies or session bytes directly to its host.

## Public boundaries

- `Geometry::for_host(cols, rows)`: the controller's host size clamped to
  2..=500 columns and 1..=200 rows; `None` for a zero dimension (callers keep
  their previous size or the 80x24 fallback). `push_bound()` is the most rows
  one engine step can push into history; the engine keeps twice that as
  counting headroom beyond `max_history_rows`.
- `Vt::new(geometry, VtConfig { max_history_rows, default_title })`.
- `Vt::feed(bytes, now) -> Fed { live, replies, dropped, titles, focus_enabled }`: the front filter
  holds partial tokens across reads (4096-byte body cap; oversized tokens are
  dropped and only counted), feeds complete tokens to the engine in steps of
  at most 64 bytes, and applies the live output policy: write `replies` to
  the PTY; `live` is the filtered output for a Direct controller (queries,
  host-dependent requests, keyboard negotiation, OSC 52, OSC 8 and every
  other OSC/DCS/SOS/PM/APC stripped; titles re-encoded; `CSI ? 47/1047`
  removed because the engine implements only 1049). A token is classified
  by the sequence vte dispatches, whatever C0 controls vte executes before
  its introducer (CAN and SUB cancel the sequence instead, and ESC restarts
  it), and the controls vte executes in a stripped or rewritten token still
  reach the host, in order. `titles` lists each change of the application's
  own title in order, one per change even within one read, by value: `None`
  is unset (the default title shows, including after `ESC c`), and an
  unchanged title is not listed again. The body cap ignores a title sequence
  longer than 4,096 bytes, so with its `2;` the longest OSC 0/2 title is
  4,094 bytes. `focus_enabled` says a token of this feed turned focus
  reporting (mode 1004) on and it is still on, however the tokens were split
  across reads.
- `Vt::application_title()`: the application's current title, `None` while
  unset; the session daemon reports it as inert core metadata.
- `Vt::sync_pending(now)` / `Vt::tick(now)`: the first-begin synchronized
  update latch (150 ms). Take no snapshot and derive nothing while pending;
  when `tick` returns true, commit `Presentation::end_sync()` for hosts that
  still have an update open.
- `Vt::resize(geometry)`: equal geometry is a no-op.
- `Vt::bracketed_paste()`: the application's mode 2004, for paste cleanup.
- `Vt::focus_reporting()`: the application's mode 1004, for the daemon's
  revision-1 Focus reports.
- `Presentation::new(host_cols, host_rows)`: one per attachment, owned by the
  daemon. Every byte string returned for it (`grant`, `produce`, `derive`,
  `present`, `end_sync`, `release`) must be committed; committed Direct live
  output must be reported with `Vt::commit_live(&mut presentation, bytes)`
  after feeding that output. `mode()` is Direct or Derived.
- `Vt::grant(&mut presentation) -> (phase A, Restoration)` takes the
  snapshot point; `Vt::produce(&mut restoration, &mut presentation, budget)
  -> Chunk { bytes, done }` yields whole history rows up to about `budget`
  bytes, then the screen suffix. Commit `RestoreEnd` after the `done` chunk.
- `Vt::derive(&mut presentation)`: the live update for a Derived controller
  after `RestoreEnd`; empty when nothing changed.
- `Vt::present(&mut presentation, host_cols, host_rows)`: after a ready
  controller's resize, the repaint (or nothing, Direct to Direct) that must
  precede further live output.
- `Presentation::release()`: the normal-shell baseline, leaving the alternate
  screen only if this attachment entered it, then restoring the normal-screen
  cursor from this attachment's committed presentation. Resetting the scroll
  region and origin mode can home the cursor, so release ends with absolute
  positioning, not a cursor save/restore that could reinstate application modes.
- `token::Tokenizer`: the front filter's tokenizer, public for differential
  testing against vte.

## Limitations

Scroll region, tab stops, character sets, palette, keyboard-protocol modes,
hyperlinks and the host title are not restored. A restoration taken while the
application is on the alternate screen sets the normal saved cursor to the
normal current cursor (E1). Combining marks per cell are unbounded in
Alacritty 0.26.0.

Titles are decoded by Alacritty 0.26.0: it trims surrounding whitespace,
drops a title parameter that is not valid UTF-8, and keeps at most the first
16 OSC parameters, so a title with more than 14 semicolons loses its tail.

## Acceptance scenarios

Crate tests supplement, and never replace, the session daemon's Contract
layer and the monitor's evidence.

| Scenario | Named tests |
|---|---|
| CTL-03 (crate case): application title changes are reported by value, in order and one per change; unset, empty and a value stay distinct; an oversized title is ignored | `application_title_changes_are_reported_in_order_by_value` |
| FOC-01 (crate case): the application's focus reporting mode (1004) is reported, and each feed says whether it newly enabled the mode, however its tokens were split across reads | `focus_reporting_reports_the_mode_and_each_feed_that_newly_enabled_it` |
