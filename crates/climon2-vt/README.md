# climon2-vt

The session daemon's virtual terminal (ADR 0007): a pure, platform-neutral,
infallible adapter around unchanged `alacritty_terminal` 0.26.0, and the
allocation-bounded `Model` a monitor keeps for each session, with the
read-only views it paints from. No I/O, no `cfg(target_os)`, and no Alacritty
type in the public API. Only `climon2-daemon` and `climon2-monitor` may depend
on it (ADR 0004); the monitor uses it today only for the documented
normal-shell baseline in its release.

The [#63 terminal contract](../../docs/protocol/terminal-integration.md) has
the monitor paint each session with Ratatui from
[read-only views](#read-only-views) of its model (#83). They are Rust types,
not a native cross-release cell schema; the monitor paints its selected
session from them (#108). The daemon remains the sole query authority; the
monitor never forwards parser replies or session bytes directly to its host.

**Implemented ([#89](https://github.com/jackgeek/climon2/issues/89)):** the
canonical restoration (`Vt::canonical`) that the session daemon serializes
for monitor-mode connections at the effective session geometry. Ordinary
host Direct/Derived presentation below is unchanged.

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
- `Vt::canonical(&mut presentation) -> (phase A, Restoration)` takes the
  snapshot point of a canonical restoration, produced the same way: the
  session at its own geometry for a fresh terminal of exactly that size,
  whatever host `presentation` described. Phase A is only the prologue (whose
  scroll-region reset homes the cursor) and any truncation notice, history
  rows are printed from the top of the fresh screen, and C1 scrolls off
  exactly the lines printed (a wrapped notice counts as the rows it wraps
  onto), so the fresh terminal's history is exactly the retained rows. No
  host content is preserved and no byte depends on a host's size.
  `presentation` becomes a fresh Direct record of a host of the session
  geometry, so live output for it stays Direct; a monitor-mode connection
  parses each restoration into a fresh model.
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

## Monitor models

`Model` is a monitor's copy of one session's terminal, bounded by allocation
(ADR 0007, LIM-03). It never answers queries and never resizes or reflows in
place.

- `Model::new(geometry, Limits { bytes, history_rows })` admits a blank model
  only if its mandatory allocation fits `bytes`: the engine for that geometry,
  vte's 2 MiB synchronized-update buffer, a 32 KiB reserve for what the front
  filter and the engine hold briefly, and, when it keeps history, the first
  1000-row chunk, which holds the protected newest page. Otherwise it returns
  `Rejected { needed, limit }`.
- `Model::feed(bytes)` applies terminal bytes token by token. Only what the
  live output policy forwards to a Direct controller reaches the engine; a
  stripped token leaves only the C0 controls vte executes in it. Each token's
  worst-case growth is charged, from the policy's own vte parse, before the
  engine sees it (72 bytes per combining mark, 56 per cell printed under an
  underline colour, three cells per wide character, and for `CSI n b` the `n`
  vte parses times the worst character). When the charge would exceed
  `bytes`, the active screen is measured first; then history yields, oldest
  chunk first and never the first chunk: the current model's while a
  restoration is staged, then the updated model's own. A token that still
  does not fit is rejected with the rest of the update, and the model refuses
  output until a staged restoration is applied.
- History grows in the engine's 1000-row chunks. Before a token that could
  fill the current limit, every chunk it could fill is admitted while the
  next one fits (a token pushes at most a row per byte and per repetition, or
  a screenful, so one long repeat or released run of line feeds can fill
  several); beyond the chunks that fit, the oldest rows are discarded and the
  update is still accepted, and the history view reports
  [truncation](#read-only-views). A discarded chunk's
  rows and cell data are released at once, on either screen, and a chunk
  admitted but not yet filled yields without losing a row.
  `Model::shorten_history()` discards the oldest chunk the same way for the
  monitor's pressure policy across sessions (#119), and returns false once
  only the first chunk is left. `Model::history()` shows at most
  `history_rows` rows.
- `Model::stage(geometry)` starts a restoration in a separate model, admitted
  only if it fits beside the current one once that model's oldest history
  has yielded; later `feed` calls build it, and the current model's history
  keeps yielding to it. `Model::apply()` replaces the current model and
  history whole; `Model::discard()` abandons it. A new effective geometry
  arrives this way: the owner decided that a monitor-mode geometry change is
  always followed by a restoration at that geometry (ADR 0007), a wire
  amendment that #89 makes.
- `Model::footprint()` is the allocation capacity charged against `bytes`, the
  engine's allocation plus `Model::RESERVE` and growth charges;
  `Model::screen()` and `Model::history()` are text views of the current model,
  and `Model::view()` its [read-only views](#read-only-views).

## Read-only views

`Model::view()` borrows the current model for one frame as a `view::View`; the
model cannot change while it is held. Every type is climon2's own:
`xtask/tests/vt_interface.rs` parses this crate's sources and fails if a
public item, in any module, or a trait impl of a public type names an engine
type, whether by path, through a `use` or re-export, or through a crate type
alias of one; a glob import of the engine fails it too. It reads source
only, so items a macro generates are not checked.

- `View::screen()` is `Screen::Normal { rows, history }` or, after
  `CSI ? 1049 h`, `Screen::Alternate { rows }`, which keeps no history.
  `Rows` has `len()`, `is_empty()`, `get(index)` and `iter()`, top row first;
  `Row::cells()` yields one `Cell` per column.
- `Cell { content, foreground, background, underline, rendition }`. `content`
  is `Content::Narrow(Glyph)`, `Content::Wide(Glyph)`, which covers the next
  column too, or `Content::Spacer`, that next column, with nothing of its own
  to draw. A `Glyph` is a character and the combining marks printed after it;
  a blank cell holds a space, and so does the cell where a tab began. A
  `Colour` is `Default`, one of the 16 `Ansi` colours, `Indexed(n)` from the
  256-colour palette or `Rgb(r, g, b)`; `underline` is SGR 58's colour, and
  `Default` draws underlines in the foreground colour. `Rendition` has bold,
  dim, italic, one `Underline` style (single, double, curly, dotted, dashed),
  inverse, hidden and strikeout.
- `View::cursor()` is `Cursor { row, column, visible, shape, blinking }`,
  counted from 0 on the active screen; after a character is printed in the
  last column the cursor stays there. DECSCUSR's `CursorShape` is `Block`,
  `Underline` or `Bar`, blinking for odd `Ps`, or `Default`, the host's own
  shape (`CSI 0 SP q`), for `Ps` 0 or no DECSCUSR since the model started or
  RIS; `CSI ? 12 h` and `l` set `blinking` alone. This is the cursor style the
  monitor honours. A model's engine reports a shape DECSCUSR never selects
  while the style is unset, so `Default` is told apart from an explicit
  steady block; the daemon's engine keeps Alacritty's default.
- `View::modes()` is `Modes`: application cursor keys (DECCKM) and keypad
  (DECKPAM, DECKPNM), bracketed paste (2004), `MouseReporting` (`Off`, 1000
  `Clicks`, 1002 `Drag`, 1003 `Motion`), `MouseEncoding` (`Default`, 1005
  `Utf8`, 1006 `Sgr`), focus reporting (1004) and alternate scroll (1007). The
  latest mouse reporting mode set replaces the others, and so does the latest
  encoding.
- `History { rows, truncated }` holds the newest rows scrolled off the normal
  screen, oldest first, at most `history_rows`: the rows `Model::history()`
  shows. `truncated` says older rows were discarded since the history was last
  cleared (`CSI 3 J` on the normal screen, or `ESC c`): beyond `history_rows`,
  because the allocation bound refused the history a chunk or took one back
  (beside a staged restoration too), or by `Model::shorten_history()`.
  Shortening is exact: a chunk admitted ahead of rows that never came yields
  without truncating. Output the admitted chunks had room for discards
  nothing, even when it fills them exactly. Applying a restoration replaces
  the history and its state whole.

## Scenario-to-test mapping

Acceptance-matrix IDs are cited in each test's doc comment. Crate tests
supplement, and never replace, the session daemon's Contract layer and the
monitor's evidence.

| Scenario | Test (`tests/bound.rs` unless another file is named) |
|---|---|
| LIM-03 Contract: mandatory model admitted at its bytes, rejected one below | `a_model_is_admitted_at_its_mandatory_bytes_and_rejected_one_below` |
| LIM-03 Contract: plain text grows nothing beyond held cells | `plain_text_fills_held_cells_without_growth` |
| LIM-03 Contract: history chunk at the bound, oldest row discarded one past it | `history_grows_at_the_bound_and_discards_its_oldest_row_one_byte_past_it` |
| LIM-03 Contract: history grows only before output that could fill it, by every chunk one token could fill (a long repeat, released line feeds: at the bound and one past it), never past its configured rows, and is not shown over the alternate screen | `history_grows_only_before_output_that_could_fill_it`, `one_token_admits_every_history_chunk_it_could_fill`, `history_never_grows_past_its_configured_rows`, `the_alternate_screen_shows_no_history` |
| LIM-03 Contract: history shows at most `history_rows` rows, all of them at the limit and not the oldest one past it | `history_shows_its_rows_at_the_limit_and_not_the_oldest_one_past_it` |
| LIM-03 Contract: the largest limits never overflow the accounting | `the_largest_limits_never_overflow_the_accounting` |
| LIM-03 Contract: wide characters at the bound and one past it | `wide_characters_are_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: combining marks at the bound and one past it | `combining_marks_are_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: auxiliary cell data (underline colour) at the bound and one past it, invalid bytes included; colours changed between cells keep no more than charged | `underline_coloured_cells_are_accepted_at_the_bound_and_rejected_one_byte_past_it`, `cells_under_changing_underline_colours_keep_no_more_than_they_were_charged` |
| LIM-03 Contract: history yields its oldest chunk, never the first, before live output, the alternate screen, staging or a staged restoration is rejected: whole at the bound, oldest chunk discarded one byte below it, including the #159 review's worked cases (200x60 within 64 MiB; staging at 500x200 after 3,100 rows); an unfilled chunk yields without losing a row | `live_output_discards_the_oldest_history_instead_of_being_rejected`, `history_yields_its_oldest_chunk_to_live_output_one_byte_below_the_bound`, `history_never_yields_its_first_chunk`, `history_yields_to_the_alternate_screen`, `an_unfilled_history_chunk_yields_without_losing_a_row`, `staging_a_restoration_discards_the_current_models_oldest_history_first`, `staging_never_takes_the_current_models_first_history_chunk`, `a_staged_restoration_takes_room_from_the_current_models_history_first` |
| LIM-03 Contract: the monitor shortens history a chunk at a time, releasing it at once, never the first chunk (#119) | `shortening_history_releases_the_oldest_chunk_and_never_the_first` |
| LIM-03 Contract: restoration staging at the bound and one past it | `restoration_staging_is_accepted_at_the_bound_and_rejected_one_byte_past_it` |
| LIM-03 Contract: resize staged at the bound and rejected one past it | `resizing_stages_a_replacement_at_the_bound_and_is_rejected_one_byte_past_it` |
| LIM-03 Contract: discarded and superseded restorations never become current | `discarded_restorations_never_become_current` |
| LIM-03 Contract: a rejected model recovers only through an applied restoration | `a_rejected_model_accepts_output_again_after_a_restoration_is_applied` |
| LIM-03 Contract: released marks reclaimed by measuring | `released_marks_make_room_before_anything_is_rejected`, `cleared_history_releases_its_marks`, `a_full_reset_releases_marks_and_history` |
| LIM-03 Contract: alternate-screen marks stay charged | `alternate_screen_marks_stay_charged_until_that_screen_is_cleared` |
| LIM-03 Contract: charges match engine allocation and history storage | `a_fresh_model_charges_exactly_what_its_engine_allocated`, `a_long_history_stays_within_the_smallest_limit_that_keeps_it`, `history_storage_is_allocated_a_chunk_at_a_time` |
| LIM-03 Contract: marks accumulating on every cell are measured as they are kept | `marks_accumulating_on_every_cell_stay_within_the_limit` |
| LIM-03 Fault: `CSI 65535 b` of a mark at the bound and one past it, with controls anywhere in it or no count, and repeated | `repeated_marks_are_accepted_at_the_bound_and_rejected_one_byte_past_it`, `repeat_counts_are_the_parsers_whatever_controls_sit_in_them`, `repeated_repeats_stay_within_the_limit` |
| LIM-03 Fault: hostile titles, title stack, hyperlinks, DCS, clipboard, queries, with or without controls before the introducer, no history | `sequences_no_host_receives_never_grow_the_model` |
| LIM-03 Fault: adversarial streams of text, marks, repeats, tints, scrolls, screen switches and control strings with tricky bytes slipped in, any read sizes | `adversarial_streams_never_take_a_model_past_its_limit` |
| LIM-03 Contract: the engine sees exactly what the live output policy forwards: text under a stripped hyperlink costs nothing, and a rewritten screen-mode list keeps the alternate screen the engine implements | `the_engine_sees_only_what_a_host_would_receive` |
| LIM-03 Fault: the reserve covers brief holdings with no history: 4096 held controls released at once (as one token), the largest held bodies, resets, tinted templates | `a_cancelled_sequence_releasing_held_controls_stays_within_the_limit`, `the_reserve_covers_what_is_held_briefly`, `tests/tokenizer.rs`: `controls_held_by_an_abandoned_sequence_are_one_text_token` |
| CTL-03 (crate case): application title changes are reported by value, in order and one per change; unset, empty and a value stay distinct; an oversized title is ignored | `tests/live.rs`: `application_title_changes_are_reported_in_order_by_value` |
| FOC-01 (crate case): the application's focus reporting mode (1004) is reported, and each feed says whether it newly enabled the mode, however its tokens were split across reads | `tests/live.rs`: `focus_reporting_reports_the_mode_and_each_feed_that_newly_enabled_it` |
| TRM-03 and CTL-04 (crate case, canonical monitor-mode serialization): a canonical restoration rebuilds exactly the retained history, screen and cursor in a fresh terminal of the session geometry, with identical bytes whatever host the presentation described; no blank row is added and none lost, with a wrapped truncation notice, a gap notice during production, more rows than the screen, a soft-wrapped row and a one-row screen | `tests/restore.rs`: `a_canonical_restoration_is_the_session_at_its_geometry_for_a_fresh_terminal`, `a_canonical_restoration_scrolls_off_exactly_the_history_it_printed`, `a_canonical_restoration_counts_a_gap_notice_among_the_rows_it_scrolls_off` |
| TRM-02 (crate case): a C0 control between `ESC` and an introducer never lets a filtered sequence reach the host; each token keeps the kind vte dispatches (#152) | `tests/live.rs`: `a_control_after_esc_reaches_the_host_without_the_filtered_sequence`; `tests/tokenizer.rs`: `vte_dispatches_each_token_as_its_kind` |
| TRM-01 (crate case): wide and combining characters, a wide character wrapping at the right margin; colours (the 16 ANSI colours, palette, direct, underline colour, defaults) and rendition; a tab's cells blank, in the text views too | `tests/view.rs`: `wide_and_combining_characters_fill_the_cells_they_cover`, `cells_keep_the_colours_sgr_selected`, `sgr_selects_each_of_the_sixteen_ansi_colours`, `cells_keep_the_rendition_sgr_selected`, `a_tab_leaves_the_cells_it_passes_blank` |
| TRM-01 (crate case): cursor position, visibility, shape (the host's default for no DECSCUSR, Ps 0 or RIS) and blink; the 1049 alternate screen, entered cleared and left with the normal screen and cursor restored | `tests/view.rs`: `the_cursor_is_where_the_application_moved_it`, `the_cursor_has_the_visibility_shape_and_blink_the_application_selected`, `the_alternate_screen_replaces_the_normal_screen_until_it_is_left` |
| INP-02, INP-03, INP-05, FOC-01 (crate cases): cursor-key and keypad, bracketed-paste, mouse reporting and encoding, and focus-reporting modes, set and reset | `tests/view.rs`: `cursor_key_and_keypad_modes_follow_decckm_and_deckpam`, `bracketed_paste_follows_mode_2004`, `mouse_reporting_and_encoding_follow_their_modes`, `focus_reporting_follows_mode_1004` |
| TRM-05 (crate case): normal history rows as cells; alternate scroll mode; no history over the alternate screen; cleared history starting again complete | `tests/view.rs`: `rows_scrolled_off_the_normal_screen_are_its_history`, `alternate_scroll_follows_mode_1007`, `the_alternate_screen_shows_no_history_until_it_is_left`, `cleared_history_starts_again_with_nothing_discarded` |
| TRM-05, LIM-03 (crate cases): explicit history truncation at the row limit and one past it, when the allocation bound refuses growth (at the bound and one byte below), when shortening discards rows and not when it discards none, while the alternate screen is active, and replaced whole by an applied restoration; none when output exactly fills the admitted chunks (SU, ED 2, line feeds); a history full at its limit reporting it once output could push a row out (owner decision) | `tests/view.rs`: `history_is_truncated_one_row_past_its_limit`, `history_the_bound_stops_growing_is_truncated`, `shortened_history_is_truncated_only_when_rows_are_discarded`, `the_alternate_screen_shows_no_history_until_it_is_left`, `a_restoration_replaces_a_truncated_history_whole`, `output_filling_the_admitted_history_exactly_discards_nothing`, `a_history_full_at_its_limit_reports_truncation_once_output_could_push_a_row_out` |
| ADR 0007 (#83): no engine type in the public interface, the views included | `xtask/tests/vt_interface.rs`: `the_virtual_terminal_interface_names_no_engine_type` |

## Limitations

Scroll region, tab stops, character sets, palette, keyboard-protocol modes,
hyperlinks and the host title are not restored. A restoration taken while the
application is on the alternate screen sets the normal saved cursor to the
normal current cursor (E1). Combining marks per cell are unbounded in
Alacritty 0.26.0; only `Model` bounds them, by allocation. A model's bound is
tracked allocation capacity, not resident memory, and its charges are upper
bounds, so it can reject output before memory is strictly exhausted.

The views show what the engine holds. It ignores blinking text (SGR 5 and 6),
so no cell blinks. A fresh model has alternate scroll on, as Alacritty
starts, and the daemon's restoration sets it on hosts too, until #179 starts
the engine with it off. Once `history_rows` caps a history's growth (as at
the monitor's default 10,000, ten whole chunks) or the bound refuses it, a
full history loses its oldest row to each row pushed without the engine
reporting it, so it reports truncation as soon as output that could push a
row out leaves it full, even before one is lost (the owner's decision on
#83; exact counting is decided with #112). A model keeping no history reports
it as soon as it is fed.

Titles are decoded by Alacritty 0.26.0: it trims surrounding whitespace,
drops a title parameter that is not valid UTF-8, and keeps at most the first
16 OSC parameters, so a title with more than 14 semicolons loses its tail.
