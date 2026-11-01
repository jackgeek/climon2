# climon2-vt

The session daemon's virtual terminal (ADR 0007): a pure, platform-neutral,
infallible adapter around unchanged `alacritty_terminal` 0.26.0. No I/O, no
`cfg(target_os)`, and no Alacritty type in the public API. Only
`climon2-daemon` may depend on it (ADR 0004).

## Public boundaries

- `Geometry::for_host(cols, rows)`: the controller's host size clamped to
  2..=500 columns and 1..=200 rows; `None` for a zero dimension (callers keep
  their previous size or the 80x24 fallback). `push_bound()` is the most rows
  one engine step can push into history; the engine keeps twice that as
  counting headroom beyond `max_history_rows`.
- `Vt::new(geometry, VtConfig { max_history_rows, default_title })`.
- `Vt::feed(bytes, now) -> Fed { live, replies, dropped }`: the front filter
  holds partial tokens across reads (4096-byte body cap; oversized tokens are
  dropped and only counted), feeds complete tokens to the engine in steps of
  at most 64 bytes, and applies the live output policy: write `replies` to
  the PTY; `live` is the filtered output for a Direct controller (queries,
  host-dependent requests, keyboard negotiation, OSC 52, OSC 8 and every
  other OSC/DCS/SOS/PM/APC stripped; titles re-encoded; `CSI ? 47/1047`
  removed because the engine implements only 1049).
- `Vt::sync_pending(now)` / `Vt::tick(now)`: the first-begin synchronized
  update latch (150 ms). Take no snapshot and derive nothing while pending;
  when `tick` returns true, commit `Presentation::end_sync()` for hosts that
  still have an update open.
- `Vt::resize(geometry)`: equal geometry is a no-op.
- `Vt::bracketed_paste()`: the application's mode 2004, for paste cleanup.
- `Presentation::new(host_cols, host_rows)`: one per attachment, owned by the
  daemon. Every byte string returned for it (`grant`, `produce`, `derive`,
  `present`, `end_sync`, `release`) must be committed; committed Direct live
  output must be reported with `commit_live`. `mode()` is Direct or Derived.
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
  screen only if this attachment entered it.
- `token::Tokenizer`: the front filter's tokenizer, public for differential
  testing against vte.

## Limitations

Scroll region, tab stops, character sets, palette, keyboard-protocol modes,
hyperlinks and the host title are not restored. A restoration taken while the
application is on the alternate screen sets the normal saved cursor to the
normal current cursor (E1). Combining marks per cell are unbounded in
Alacritty 0.26.0.
