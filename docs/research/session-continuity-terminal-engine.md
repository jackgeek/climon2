# Terminal engines for session continuity

Research for [#46](https://github.com/jackgeek/climon2/issues/46), under
[map #40](https://github.com/jackgeek/climon2/issues/40). Checked **2026-10-28**.

## Scope and evidence status

Planning only: macOS/bash release, arbitrary PTY programs, daemon-lifetime screen and
scrollback retention, including when no clients exist. The session daemon alone owns the
source-of-truth virtual terminal; clients keep no emulator. Version-matched clients,
frozen invocation/hello, write-once additive metadata, blank non-controlling clients,
space takeover and most-recent-controller hand-back remain constraints. No monitor,
port implementation, crash/reboot recovery or persistent completed-command history.
Read root glossary, ADRs 0001–0005, #46's context comment and #27/#28's inherited
constraints; **#28's earlier engine comparison is not evidence for the claims below**.

**V** = verified from pinned primary source; **I** = inference/estimate;
**U** = untested gap; **R** = recommendation for a later decision, not a decision.
This is source/document inspection, not a build, benchmark, fuzz run or demonstration
of real-terminal fidelity. Upstream tests below were inspected, **not executed**.
No product or dependency changes, no engine/architecture/scrollback UX selection.
An automated documentation check verified all 33 reference definitions are used and
the pinned line ranges resolve within 23 fetched primary-source files; no engine probes ran.

### Revisions and maintenance

| Candidate | Inspected baseline and current upstream |
|---|---|
| `alacritty_terminal` | **V:** published documentation currently resolves to **0.26.0** ([docs.rs](https://docs.rs/crate/alacritty_terminal/latest)); Alacritty release **v0.17.0**, 2026-04-06, contains that crate at [`94e7c8874e526b1e67b349d9ba30ddf81669119e`](https://github.com/alacritty/alacritty/tree/94e7c8874e526b1e67b349d9ba30ddf81669119e). Current master is **0.26.1-dev**, [`bbb2cbf9470a6707b1205fbddbdb25109b14e2fd`](https://github.com/alacritty/alacritty/commit/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd), committed 2026-10-26. Matrix references **master unless marked release**; these are not interchangeable. Crates.io API returned HTTP 403 here; published-version check used docs.rs plus release source, not a successful registry API query. |
| `vt100` | **V:** **0.18.1**, tag and current main [`fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad`](https://github.com/doy/vt100-rust/commit/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad), 2026-10-19. [Changelog][V1] records 0.18.1 fixing publication of changes intended for 0.18.0, including rewritten resize and new capture APIs. Avoid relying on 0.18.0's intended features as proof of what shipped. |

Both have recent first-party changes, not evidence of an SLA, future maintenance or
absence of bugs. Their 0.x APIs have changed: pin deliberately and budget upgrades.
No third engine is evaluated: the concrete gaps below already define useful decision
inputs; whether those gaps disqualify either candidate depends on the still-open
fidelity/UX contract. Broadening the comparison without that contract would be speculative.

## Compact feasibility matrix

| Concern | `alacritty_terminal` | `vt100` 0.18.1 |
|---|---|---|
| Headless daemon, no clients | **V:** `Term::new(config, dimensions, event_proxy)` and the ANSI processor accept dimensions/output independently of a window or client; upstream reference tests instantiate just that combination [A1][A8]. **I:** keep feeding it from the daemon's one PTY output pump even with zero clients. | **V:** `Parser::process` updates an owned `Screen`; constructor takes nonzero dimensions and history capacity, not a client [V2]. **I:** same continuous daemon-owned ingestion. |
| Normal/alternate screens | **V:** active/inactive grids, normal history configured, alternate history zero; switching swaps grids and resets alternate contents on entry [A1]. The inactive grid is private. | **V:** separate normal/alternate grids; normal capacity configurable, alternate `Scrollback::Disabled`. Active-screen getters dispatch through `grid()`. DEC 47/1049 handled; 1047/1048 fall to unhandled callbacks [V3][V4]. **U:** do not promise every application's alternate-screen convention. |
| Modes and query replies | **V:** [mode flags](https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L53-L88) include cursor/keypad, origin/wrap, mouse, focus, bracketed paste and optional kitty keyboard; DA and DSR/CPR produce `Event::PtyWrite` [A2], mode reports also exist [A13]. Color, pixel-size and clipboard queries require host-provided event handling [A3]. No event listener means no replies delivered. | **V:** mode getters and input-mode formatting cover keypad/cursor, bracketed paste, mouse and cursor style [V5]. CSI `n`/`c` are not handled as DSR/DA: the unhandled callback is available, but **no built-in query responder** [V6][V7]. **I:** an adapter can answer selected queries from daemon state; it must implement the semantics, not merely forward a cursor getter. |
| Redraw/serialization | **V:** public grid cells, cursor, mode, colors and damage/render APIs [A1][A4]. Grid/Cell have serde, but `Grid` serde **skips current and saved cursors**, and `Term` does not derive full-state serialization [A1][A5]. **I:** a custom terminal-output serializer is required; serde JSON is neither ANSI redraw nor a complete terminal checkpoint. | **V:** built-in `contents_formatted`, `state_formatted`, corresponding diffs and row formatting; `state_formatted` = visible contents + supported input modes [V5]. **Not full engine serialization:** it does not emit both grids, all history, margins/origin/saved-cursor state or parser partial state. See continuity caveat below. |
| Unicode/wide characters | **V:** `unicode-width`, wide/spacer/wrap flags; zero-width marks attached to base cells [A6]. Master retains at most **9** zero-width chars/cell, dropping excess [A7]. **Release 0.26.0 differs materially:** its unbounded `Vec<char>` permits growth on one cell; fixed upstream 2026-08-26 [A10]. | **V:** fixed 32-byte cells, bounded UTF-8 combining content, wide/continuation cells, VS16 emoji-presentation widening [V8]. Excess combining marks are silently dropped. **U, both:** grapheme/ZWJ behavior, ambiguous-width choices and agreement with each outer terminal/font are not proven by a width library or these inspections. |
| Resize and reflow | **V:** resizes both grids, enabling reflow for normal and disabling it for alternate, regardless of which is active; wrapped rows/wide spacers have dedicated reflow logic [A1][A9]. Resets scroll region on resize. **U:** fidelity to the intended outer terminals across resize still needs tests. | **V:** height shrink/grow moves eligible rows into/out of bounded normal history. Width change **truncates/pads rows, does not reflow**, explicitly documented as unlike most terminals [V2]. This is a material contract constraint, not just missing glue. |
| History access/bounds | **V:** configured maximum history rows; dynamic capped growth, `update_history` can shrink/change cap [A5], negative-line indexing/`iter_from`, `history_size`, `scroll_display` [A15]. Alternate grid has zero history. Logical row cap is **not allocated row count**: storage grows in batches of at least 1,000 rows and caches extra rows [A16]. **I:** history traversal/redraw is adapter work. Private inactive grid complicates obtaining normal history while alternate is active through the read-only public surface. | **V:** constructor row capacity; `scrollback_len/capacity`, clamped `set_scrollback`, `clear_scrollback`, cell/row formatting of current view [V3][V9]. `on_scroll` reports rows leaving retention (or scrolling without retention), not a ready-made attach snapshot; resize can report synthetic blank rows, documented [V7]. |
| Bounds while absent | **I, both:** retained row counts can remain bounded indefinitely while ingesting/evicting with no clients. A **row limit is not a total byte/CPU bound**: dimensions, variable cell extras, parser/control-string buffers, host events, serializer buffers and pending-client queues also matter. Neither engine makes scrollback an append-only command transcript; erase/reset and alternate-screen behavior change what survives. | **V:** fixed cell payload simplifies a retained-grid budget, but allocator/container overhead and other state remain. Capacity is fixed through the public constructor, not a runtime retention-policy API [V2][V8][V9]. The internal “unbounded” history counter is bookkeeping for resize, **not unbounded retained rows** [V9]. |
| Licensing/portability | **V:** Apache-2.0 [A11]; Rust 1.85/edition 2024 [A14]. Core package has Unix and Windows PTY dependencies but no GUI/OpenGL dependency [A11]. macOS/Windows CI tests default and no-default-feature core builds [A12]. **I:** embed `Term`/processor without using its PTY loop; preserve climon2's PTY/platform seams. | **V:** MIT; Rust 1.85/edition 2024; normal dependencies `vte`, `unicode-width`, `itoa` (optional embedded I/O), no native platform dependency [V10]; optional `no_std` [V16]; Linux/musl/macOS/Windows CI [V15]. **I:** straightforward platform-neutral embedding; CI configuration is evidence of intended coverage, not proof this adapter already works on macOS. |
| Failure containment | **V:** in-process mutable grid/parser; reference tests replay recordings and compare grids [A8]. The release's combining-mark allocation issue demonstrates why capped history alone is insufficient [A10]. | **V:** upstream round-trip/resize properties exist [V11][V17]. An old open count-complexity report is **not proof of the current bug**; see below. **U, both:** no exhaustive panic/OOM/CPU assurance or independent adversarial execution here. |

## Restore is not just repainting the visible cells

**V:** vt100's serializer emits active visible rows/cursor/attributes plus its listed
input modes [V5][V9]. Upstream round-trip helpers compare cells, cursor and those modes;
they do **not** prove inactive-screen/history equality, every hidden terminal state,
or equivalence under arbitrary future input [V11]. The input-mode serializer does not
emit an alternate-screen switch, origin mode or scroll margins [V5].
Its diff implementation zips visible rows [V9]: **I:** do not assume diffs across
unequal geometries include extra rows. A full redraw after a size boundary needs its
own contract. Alacritty's renderer-oriented surface has no ready-made equivalent,
and several state fields are private [A1][A4].

**I:** there are two different fidelity obligations:

1. Reproduce the current picture, cursor and required terminal-facing input modes.
2. Ensure subsequent output still means the same thing: inactive normal contents when
   an app exits alternate screen, saved cursor, margins/origin, tab stops, pending wrap,
   character sets, palette, and any split escape/UTF-8 sequence can affect the future.

A pretty screenshot or `state_formatted` round-trip alone does not establish (2).
If clients receive raw future PTY bytes, their **real terminals** still have terminal
state even though client processes contain no emulator. Restoring while inside
alternate screen and then receiving its exit sequence is an important counterexample:
painting only the active view does not restore the outer terminal's underlying normal
screen. Starting a live suffix halfway through an escape/UTF-8 sequence is another.
**U:** neither candidate's inspected public APIs establish a turnkey, arbitrary-cut
full-machine checkpoint that a physical terminal can resume.

**I/R for #48:** choose and test a semantic restore boundary as well as a byte-order
boundary. Snapshot/history/control/resize need a single ordered cut in the daemon's
output processing; queue subsequent updates behind that cut and deliver exactly once.
Chunk large snapshots within frozen framing limits (ADR 0001), bound queues, and specify
overflow/resnapshot behavior. A thread-safe snapshot or library diff does not itself
provide this protocol. Parser partial state, terminal resets and history replay must
be considered explicitly; the research does not select raw-suffix versus redraw-driven
delivery, nor a wire schema.

**I/R for #43:** query ownership is inseparable from live delivery. Feeding a query to
the daemon engine and forwarding it verbatim to a controlling real terminal can generate
two replies; no-client execution can generate none if responses depend on the terminal.
Select one authoritative response path and terminal identity/capability policy, including
what color/pixel/clipboard requests mean without clients. Replaying a snapshot must not
accidentally repeat queries, bells or clipboard side effects. These are decision inputs,
not an instruction to choose one engine or architecture.

## Resource and failure qualifications

**V:** Alacritty's 0.26.0 release permits unbounded combining marks in a single cell.
The [upstream fix][A10] explicitly describes memory exhaustion and replaces the vector
with a cap; current master has the cap [A7]. **R:** any later selection must name the
actual release/revision and account for this difference, rather than treating “Alacritty
supports bounded history” as a total-memory guarantee.

**V:** [vt100 issue #42](https://github.com/doy/vt100-rust/issues/42) remains open, but
its reproduction pins **0.16.2**. In the inspected 0.18.1 source, character insertion
clamps to row width and rotates a slice [V12]; insert-lines/scroll-down clamp to the
region and rotate rows [V13]. **I:** the specific old repeat-count mechanism has been
changed; it would be incorrect to repeat the report as a verified current vulnerability.
**U:** no current-version timing reproduction was run. There is still an explicit
parameter-count loop for REP [V14]; throughput under repeated hostile sequences remains
a workload to measure, not a demonstrated exploit or a claim of unbounded latency.

**I/R for #43:** treat all PTY bytes as arbitrary input. An in-process panic boundary
does not contain allocation failure, abort or CPU starvation; a worker thread alone
does not isolate address space. A time budget checked after a synchronous parse call
cannot interrupt a stuck call. Process isolation would be an architecture decision with
its own ownership/ordering costs, not a conclusion of this research. Define behavior if
the source-of-truth virtual terminal fails; a silent reset would lose restore correctness.
Bound requested dimensions and accepted host events as well as retained rows. Exact
byte accounting, parser/control-string limits, long-lived allocator retention and chosen
failure behavior remain unverified/undecided.

## Approximate adapter/serializer burden — inference, not a quotation or schedule

| Work | Alacritty | vt100 |
|---|---|---|
| Headless ingestion + normal history access | **Small–medium:** processor, event sink, dimensions, history traversal; no need to adopt its PTY layer. | **Small:** parser, callbacks, history offsets/rows; take care with resize callback semantics. |
| Selected queries and modes | **Medium:** replies exist, but route them exactly once; define host-dependent values and terminal identity. | **Medium–large:** synthesize DSR/DA and required query/mode semantics via unhandled callbacks; broader fidelity may require upstream changes. |
| Basic current-view ANSI redraw | **Large relative to vt100:** custom cells/SGR/cursor/modes/palette/wrap/wide-char serializer plus tests. | **Small initial glue:** built-in formatter; **medium–large qualification work** for existing real-terminal state and mode coverage. |
| Faithful restore + arbitrary future output | **Large/high-risk:** private inactive/hidden state may require upstream API work or a different delivery approach. | **Large/high-risk:** formatter is not a full checkpoint; adding inactive-state/margins/other fidelity is not “just call `state_formatted`.” |
| Width-reflow contract | Implementation exists; qualify it. | Missing by design; adding it is **engine-level work**, not a thin adapter. |
| Shared across both | Atomic snapshot/live delivery, history presentation and de-duplication, resource/failure policy and acceptance fixtures remain climon2 work. Existing test-harness use does not select a shipped dependency or amend ADR 0004. | Same. |

“Small” means wrapping exposed operations; “medium” adds host policy/state coordination;
“large” adds serializer/emulation behavior and a significant fidelity test surface.
No LOC/person-day numbers: no adapter prototype was built, so those would imply false precision.

## Remaining human decisions and focused validation

- **#43:** engine/revision, supported terminal identity/fidelity, query authority,
  shared boundary consistent with ADR 0004, and daemon failure behavior.
- **#49:** retained rows/bytes/lifetime, erase/alternate behavior, width semantics and
  history presentation. Neither engine automatically installs its retained history into
  the outer terminal's native scrollback, nor prevents duplication on repeated attach.
  A history offset in the daemon is not a client navigation UX.
- **#47/#48:** dimensions on hand-back/attach, ordering, semantic restore/live cut,
  flow control and slow-client behavior. Non-controllers remain blank, not replica emulators.
- **#45:** automated fixtures should cover zero-client output beyond the cap; alternate
  attach followed by return to normal; custom margins/origin/saved cursor; wide/combining/VS16
  at wrap edges; shrink/grow and width changes with retained history; queries with zero/one
  controller (exactly one reply); fragmented UTF-8/CSI/OSC at restore boundaries; repeated
  attach without duplicate history; resource limits and the selected failure behavior.
  These are recommended validation targets, **not tests claimed to exist or pass**.

All newly concrete questions fit children #43/#45/#47/#48/#49. No new ticket is requested,
no inherited ADR is amended, and no engine or scrollback UX decision is made here.

## Primary-source references

All code links are immutable line references. Moving metadata/issue links were checked
on the research date; an open issue is evidence of a report, not proof of current behavior.

[A1]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L268-L735
[A2]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1255-L1345
[A3]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/event.rs#L14-L53
[A4]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L2390-L2411
[A5]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L108-L185
[A6]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1059-L1137
[A7]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/cell.rs#L13-L173
[A8]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/tests/ref.rs#L100-L137
[A9]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/resize.rs#L1-L39
[A10]: https://github.com/alacritty/alacritty/commit/ede2ac144da4dec4c075bfa803aacf3b3739bce6
[A11]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/Cargo.toml#L1-L49
[A12]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/.github/workflows/ci.yml#L1-L37
[A13]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1934-L2149
[A14]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/Cargo.toml#L1-L12
[A15]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L383-L525
[A16]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/storage.rs#L12-L137
[V1]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/CHANGELOG.md#L1-L45
[V2]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/parser.rs#L31-L113
[V3]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/screen.rs#L81-L207
[V4]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/screen.rs#L1373-L1449
[V5]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/screen.rs#L295-L586
[V6]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/perform.rs#L61-L200
[V7]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/callbacks.rs#L1-L109
[V8]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/cell.rs#L1-L108
[V9]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/grid.rs#L251-L400
[V10]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/Cargo.toml#L1-L33
[V11]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/tests/helpers/mod.rs#L150-L237
[V12]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/row.rs#L50-L79
[V13]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/grid.rs#L647-L765
[V14]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/screen.rs#L1282-L1289
[V15]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/.github/workflows/tests.yaml#L9-L56
[V16]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/src/lib.rs#L36-L42
[V17]: https://github.com/doy/vt100-rust/blob/fb0f54ec7222f4805f33cd3ca530229e0a0fe6ad/tests/quickcheck.rs#L185-L215
