# climon2-vt

The session daemon's virtual terminal (ADR 0007): a pure, platform-neutral,
infallible adapter around unchanged `alacritty_terminal` 0.26.0, and the
allocation-bounded `Model` a monitor keeps for each session. No I/O, no
`cfg(target_os)`, and no Alacritty type in the public API. Only
`climon2-daemon` may depend on it (ADR 0004).

**Agreed, not implemented:** the
[#63 terminal contract](../../docs/protocol/terminal-integration.md) plans
read-only Rust cell/mode/cursor/history views of monitor models for Ratatui.
This is a future exception to daemon-only use, not a current public cell API or
native cross-release cell schema. Monitor-mode daemon presentation must be
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

## Monitor models

`Model` is a monitor's copy of one session's terminal, bounded by allocation
(ADR 0007, LIM-03). It never answers queries and never resizes or reflows in
place.

- `Model::new(geometry, Limits { bytes, history_rows })` admits a blank model
  only if its mandatory allocation fits `bytes`: the engine for that geometry,
  vte's 2 MiB synchronized-update buffer, a 416 KiB reserve for the front
  filter, and the first 1000-row chunk of history, which holds the protected
  newest page. Otherwise it returns `Rejected { needed, limit }`.
- `Model::feed(bytes)` applies terminal bytes token by token. Only what the
  live output policy forwards to a Direct host reaches the engine; a stripped
  token is dropped whole. Each token's worst-case growth is charged before the
  engine sees it (72 bytes per combining mark, 56 per cell printed under an
  underline colour, three cells per wide character, `n` worst characters for
  `CSI n b`). When the charge would exceed `bytes`, the active screen is measured
  first; a token that still does not fit is rejected with the rest of the
  update, and the model refuses output until a staged restoration is applied.
- History grows in the engine's 1000-row chunks, each admitted before it is
  allocated; when one does not fit, the oldest row is discarded and the update
  is still accepted. `Model::history()` shows at most `history_rows` rows.
- `Model::stage(geometry)` starts a restoration in a separate model, admitted
  only if it fits beside the current one; later `feed` calls build it.
  `Model::apply()` replaces the current model and history whole;
  `Model::discard()` abandons it. A new effective geometry arrives this way.
- `Model::footprint()` is the allocation capacity charged against `bytes`, the
  engine's allocation plus `Model::RESERVE` and growth charges;
  `Model::screen()` and `Model::history()` are text views of the current model.

## Scenario-to-test mapping

Acceptance-matrix IDs are cited in each test's doc comment.

| Scenario | Test (`tests/bound.rs`) |
|---|---|
| LIM-03 Contract: mandatory model admitted at its bytes, rejected one below | `a_model_is_admitted_at_its_mandatory_bytes_and_rejected_one_below` |
| LIM-03 Contract: plain text grows nothing beyond held cells | `plain_text_fills_held_cells_without_growth` |
| LIM-03 Contract: history chunk at the bound, oldest row discarded one past it | `history_grows_at_the_bound_and_discards_its_oldest_row_one_byte_past_it` |
| LIM-03 Contract: history never grows past its configured rows; none over the alternate screen | `history_never_grows_past_its_configured_rows`, `the_alternate_screen_shows_no_history` |
| LIM-03 Contract: wide characters at the bound and one past it | `wide_characters_are_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: combining marks at the bound and one past it | `combining_marks_are_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: auxiliary cell data (underline colour) at the bound and one past it | `underline_coloured_cells_are_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: restoration staging at the bound and one past it | `restoration_staging_is_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: resize staged at the bound and rejected one past it | `resizing_stages_a_replacement_at_the_bound_and_is_rejected_one_byte_past_it` |
| LIM-03 Contract: discarded and superseded restorations never become current | `discarded_restorations_never_become_current` |
| LIM-03 Contract: a rejected model recovers only through an applied restoration | `a_rejected_model_accepts_output_again_after_a_restoration_is_applied` |
| LIM-03 Contract: released marks reclaimed by measuring | `released_marks_make_room_before_anything_is_rejected`, `cleared_history_releases_its_marks`, `a_full_reset_releases_marks_and_history` |
| LIM-03 Contract: alternate-screen marks stay charged | `alternate_screen_marks_stay_charged_until_that_screen_is_cleared` |
| LIM-03 Contract: charges match engine allocation and history storage | `a_fresh_model_charges_exactly_what_its_engine_allocated`, `a_long_history_stays_within_the_smallest_limit_that_keeps_it`, `history_storage_is_allocated_a_chunk_at_a_time` |
| LIM-03 Contract: growth charged by what printing can hold; accumulating marks measured | `growth_is_charged_by_what_printing_can_hold`, `marks_accumulating_on_every_cell_stay_within_the_limit` |
| LIM-03 Fault: `CSI 65535 b` of a mark at the bound and one past it, and repeated | `repeated_marks_are_accepted_at_the_bound_and_rejected_one_byte_past_it`, `repeated_repeats_stay_within_the_limit` |
| LIM-03 Fault: hostile titles, title stack, hyperlinks, DCS, clipboard, queries | `sequences_no_host_receives_never_grow_the_model` |
| LIM-03 Fault: a cancelled sequence releasing 4096 held controls | `a_cancelled_sequence_releasing_held_controls_stays_within_the_limit` |

## Limitations

Scroll region, tab stops, character sets, palette, keyboard-protocol modes,
hyperlinks and the host title are not restored. A restoration taken while the
application is on the alternate screen sets the normal saved cursor to the
normal current cursor (E1). Combining marks per cell are unbounded in
Alacritty 0.26.0; only `Model` bounds them, by allocation. A model's bound is
tracked allocation capacity, not resident memory, and its charges are upper
bounds, so it can reject output before memory is strictly exhausted.
