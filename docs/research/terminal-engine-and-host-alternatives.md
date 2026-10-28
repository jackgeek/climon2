# Terminal engine and host alternatives: does another choice change the answer?

Research for [#55](https://github.com/jackgeek/climon2/issues/55), under
[map #40](https://github.com/jackgeek/climon2/issues/40). Checked **2026-10-28**.
Triggered by the driving developer's question — "are there other terminal emulators we
could try?" — after prior research found gaps in the provisional `alacritty_terminal`
pin and left Terminal.app's extended-keyboard support unqualified. The user has approved
a fixed per-session keyboard capability contract and compatible-only later attachments,
and a bounded client input codec with daemon-side encoding and no client emulator. They
have **not** accepted a qualified-profile limitation or an engine switch. This research
feeds the next grilling round on [#43](https://github.com/jackgeek/climon2/issues/43); it
does not choose policy, switch the engine, or downgrade host support.

> **Correction (same research date, same branch):** the first published version of
> this report contained internal contradictions, corrected below in place rather than
> by re-running the research. Specifically: it falsely implied xterm defines no
> MOK query/reply (it does; [#54](https://github.com/jackgeek/climon2/issues/54)
> already verified `CSI ?4m` → `CSI >4;L m`) and used the wrong kind of absence
> (Ghostty's unrelated DECRQSS reply enum) as evidence; it wrongly treated Kitty's own
> deliberate MOK no-op as disqualifying Kitty-as-host under the user's **already
> accepted** daemon-side translation architecture, which does not require the outer
> host to speak the same protocol as the virtual VT; it overstated WezTerm's
> `StableRowIndex` as lineage "lost only on eviction" without checking reflow
> specifically; it stated an unconditional Zig toolchain requirement without
> distinguishing prebuilt-artifact consumption from source build/patching, and invented
> an ADR conflict no ADR text supports; and it cited Ghostty's native key encoder at an
> unpinned HEAD commit when the same file and lines already resolve cleanly at the
> `v1.4.0` tag. Each correction is made at its exact location below, with new
> primary-source citations; nothing else in the report was touched, and the 8192/256-byte
> OSC 8 limit was already stated as the model requirement but is now made explicit as a
> concrete *native-host* gap too. No new candidate sweep, no product code, and no GUI
> action was performed to make these corrections — only source re-reading at the already
> identified pins.

## Two different questions this report keeps separate

1. **Embedded daemon engine** — the in-process Rust library climon2's daemon would use
   to hold the source-of-truth VT (today, provisionally `alacritty_terminal`). Compared
   here against WezTerm's `term` crate and Ghostty's `libghostty-vt`, with `libvterm`
   checked only to confirm it does not change the answer.
2. **Outer native host** — the real terminal application (Terminal.app, iTerm2, VS Code,
   and here additionally Kitty, Ghostty, WezTerm) that a climon2 client runs inside and
   that owns actual OS key events. **Changing the embedded engine cannot reconstruct
   input information an outer host never emitted.** A faithful Kitty/MOK *model* inside
   the daemon still depends on the outer host's real native key-event encoder to supply
   press/repeat/release, base-layout/alternate keys, lock modifiers and associated text
   in the first place. No source inspection below treats a query/flag echo as proof of
   native event fidelity, and no GUI automation, installation, or live keyboard probing
   was performed for any product.

**V** = verified from pinned primary source. **D** = documented/spec claim without
located implementing source. **I** = inference. **U** = explicitly unestablished/
unexecuted (native qualification, not disproof). This is source and documentation
inspection plus a handful of already-published content checks (below); it is not a
build, benchmark, fuzz run, or native keyboard/OSC8 qualification of any product.

## Embedded engine comparison

Baseline (not repeated, cited for contrast): `alacritty_terminal` at
`bbb2cbf9470a6707b1205fbddbdb25109b14e2fd` has no event encoder in the embeddable
library, a Kitty direct-set/query mismatch (query reads `keyboard_mode_stack.last()`,
direct-set changes `TermMode` only, so an honest query can report flags=0 while all five
enhancement bits are active), MOK parsed but not implemented (`Term`'s `Handler` leaves
`set_modify_other_keys`/`report_modify_other_keys` as no-ops), no public inactive-grid
getter, no row-lineage/eviction notifications, and an always-`std` unbounded OSC buffer.
See [#46](https://github.com/jackgeek/climon2/issues/46) and
[#53](https://github.com/jackgeek/climon2/issues/53) for the full prior evidence.

### `libghostty-vt` (Zig, `ghostty-org/ghostty`)

Pin: tag **`v1.4.0`**, commit **`83fb95624b0439ee660c2679b43b38c8b63769cd`** — verified
by dereferencing the GitHub tag object via the Git Data API to this exact commit
(`git/refs/tags/v1.4.0` → annotated tag `ce787fc1…` → commit `83fb9562…`); re-verified
here independently of the delegated research pass. A handful of the cited blob URLs at
this pin were spot-checked for HTTP 200 and matching content (LICENSE, `vt.h`,
`kitty/key.zig`, `osc.zig`) during this report's own verification pass.

| Requirement | Status | Evidence |
|---|---|---|
| VT core is a real module tree, separable from GUI/renderer | **V** | `src/terminal/{Terminal,Screen,PageList,page,osc,kitty,dcs,hyperlink,style}.zig` ([tree](https://github.com/ghostty-org/ghostty/tree/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal)) |
| Shipped, versioned embeddable library target (not just an announced goal) | **V**, but header explicitly unstable | `build.zig` defines shared/static/WASM/xcframework targets ([L128-225](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/build.zig#L128-L225)); public header says *"incomplete, work-in-progress API... definitely going to change"* ([vt.h#L10-L26](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/include/ghostty/vt.h#L10-L26), content-verified above). The 2025 ["libghostty is coming"](https://mitchellh.com/writing/libghostty-is-coming) post is **D**; the header/build target is now **V**-shipped, a materially different and more mature state than "announced." |
| Normal+alternate screen with inactive-screen read access | **V** at the Zig layer; **U** at the public C ABI | `ScreenSet.get(.alternate)` returns a live pointer while primary is active ([ScreenSet.zig#L17-L21,65-67](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/ScreenSet.zig#L17-L21)); no equivalent *input*-selectable accessor was located in `include/ghostty/vt/terminal.h` ([L371-379](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/include/ghostty/vt/terminal.h#L371-L379)) — contrast with Alacritty, where the inactive grid has **no** safe getter at any layer (only destructive `swap_alt()`). |
| Width reflow of screen and scrollback, with row lineage | **V** | `PageList.resize` dispatch and `ReflowCursor` ([L1261-1321](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/PageList.zig#L1261-L1321), [L1465-1654](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/PageList.zig#L1465-L1654)); lineage is pointer-pin remapping (`Pin`/`tracked_pins`), not a stable queryable row ID ([L7209-7219](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/PageList.zig#L7209-L7219)) — a different, **not obviously superior**, lineage model than a sequence number; still needs its own proof for climon2's delivery-position contract. |
| OSC 8, bounded | **V** | Per-cell `Hyperlink`/`Id` map ([hyperlink.zig#L15-L37](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/hyperlink.zig#L15-L37)); `MAX_BUF = 2048`, `MAX_ALLOCATING_BUF = 8MiB` ([osc.zig#L359-365](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/osc.zig#L359-L365)) — the engine itself enforces a bound, unlike Alacritty's unbounded `std`-mode OSC `Vec<u8>`. 2048 bytes is still below climon2's agreed 8192-byte URI ceiling and would need adapter-level handling either way. |
| Kitty keyboard: set/push/pop/query consistency | **V — no Alacritty-class mismatch found** | Set/push/pop dispatch and the query path operate on the **same** field, `screens.active.kitty_keyboard` ([stream_terminal.zig#L715-719,1959-1970](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/stream_terminal.zig#L715-L719)). This is a materially different, more trustworthy contract than Alacritty's split stack/mode state — content-verified in this report's own pass is limited to the surrounding file resolving at the pin; the specific line semantics are the delegated pass's finding, not independently re-derived here. |
| modifyOtherKeys | **V: real state; query (`CSI ?4m`) confirmed unimplemented** | `modify_other_keys_2` field set from the real `CSI >4;Lm` set/reset sequence and consumed by the encoder ([stream.zig#L2206-2271](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/stream.zig#L2206-L2271), [stream_terminal.zig#L720-726](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/stream_terminal.zig#L720-L726)). **Correction:** the query form xterm itself defines, `CSI ?4m`, reaches Ghostty's own `'m'`-dispatch fallback branch with its own source comment admitting uncertainty — *"`vim --clean` uses `CSI ? 4 m` and I don't know what that means"* — and is explicitly logged as `"ignoring unimplemented CSI m with intermediates"` ([stream.zig#L2280-2291](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/stream.zig#L2280-L2291)). This is the correct evidence for "no MOK query reply," not the unrelated `DECRQSS` reply enum cited in the first version of this report (`DECRQSS` answers a *different* DEC request-status protocol entirely; its not listing MOK proves nothing about `CSI ?4m`). Real state is still strictly better than Alacritty's no-op; the query gap is confirmed absent by direct dispatch-table inspection, not inferred from an unrelated code path. |
| Parser/history admission | **V, materially tighter than Alacritty** | OSC 2048B/8MiB, DCS `max_bytes = 1MiB` ([dcs.zig#L17-19,137](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/dcs.zig#L17-L19)); history `max_size`/`max_lines`, nullable = unlimited ([PageList.zig#L602-612](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/PageList.zig#L602-L612)) — an explicit cap exists, but "unlimited" is a legal configuration, so climon2 would still need to enforce its own ceiling, not merely trust defaults. |
| Combining-mark handling | **V** | `grapheme_max_len = 64` codepoints via a bitmap-chunk allocator ([page.zig#L84-95](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/page.zig#L84-L95)) — a materially higher cap than Alacritty's fixed inline 9, and a different allocation shape (chunked, not per-cell `ArrayVec`) that needs its own memory-accounting proof, not an assumption of being "better" by default. |
| Licence / build portability | **V — single hard new cost** | MIT ([LICENSE](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/LICENSE#L1-L4), content-verified above). **Zig toolchain is required unconditionally**, even through the CMake wrapper (`find_program(ZIG_EXECUTABLE zig REQUIRED)`, [CMakeLists.txt#L1-5,94-95](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/CMakeLists.txt#L1-L5)). This is a cross-language FFI/build-chain dependency climon2 does not have today (ADR 0004's crate layout assumes a Rust-only build). Only one rolling GitHub Release (`tip`) ships prebuilt artifacts, and only as WASM + Apple xcframework — **no Linux/Windows prebuilt library**, so other platforms need a from-source Zig build, cutting against ADR 0003's stated intent to add Windows/Ubuntu without a redesign. |

### `wezterm-term` (Rust, `wezterm/wezterm`)

Pin used by the delegated pass: `main` HEAD **`e68a8ec27727fbbda15e07f0a0737cd5e16481f6`**
(2026-10-26) for most structural facts, because WezTerm has never tagged a release for
`term` on its own and the whole `term`/`wezterm-cell`/`wezterm-escape-parser` split
postdates its only relevant historical tag; cross-checked for continuity against tag
`termwiz-0.23.3` → `4bf28e253c0167102f07bfc7e7199c13eed98012` (2025-03-20). This report
independently re-fetched and content-checked `term/Cargo.toml`, `term/src/screen.rs`,
`LICENSE.md`, and `vtparse/src/lib.rs` at the HEAD SHA and confirms **MIT**, not
GPL-3.0 as this ticket's brief assumed — a correction, not a new finding.

| Requirement | Status | Evidence |
|---|---|---|
| Headless, GUI-free construction | **V** | `Terminal::new(size, config, term_program, term_version, writer: Box<dyn Write+Send>)` ([terminal.rs#L143-171](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminal.rs#L143-L171)); `term/Cargo.toml` has no GUI dependency (no winit/wgpu/cocoa), unlike `wezterm-gui` ([term/Cargo.toml#L12-39](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/Cargo.toml#L12-L39), content-verified above). |
| Normal+alternate screen, inactive-screen read access | **V** model; **U, absent** public getter | `ScreenOrAlt{screen, alt_screen, alt_screen_is_active}` ([terminalstate/mod.rs#L150-178](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/mod.rs#L150-L178)); exhaustive `pub fn` search of `terminalstate/mod.rs` finds only `screen()`/`screen_mut()`/`is_alt_screen_active()` — no `alt_screen()`/`inactive_screen()` ([L711-720,772-775](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/mod.rs#L711-L720)). **Same class of gap as Alacritty** — this is not a free win; it would need the same kind of narrow upstream extension/patch either way. |
| Reflow of screen and scrollback | **V — real reflow, the headline strength** | `Screen::rewrap_lines` ([screen.rs#L104-186](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L104-L186)) reflows width changes for retained history, not just truncate/pad — this is the concrete capability vt100 (the prior engine-research alternative) was shown to lack by design, and the one Alacritty's `master` currently has but 0.26.0 released state needed separate verification for. |
| History structure / lineage | **V** | `VecDeque<Line>`, capacity `physical_rows + scrollback_size()`, pop-on-overflow ([screen.rs#L16-46,174-221](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L16-L46)); `StableRowIndex` is a monotonic `phys_index + eviction_count` offset that is correctly maintained across plain top-of-screen scroll-eviction and `erase_scrollback` ([L734,769](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L734)). **Correction:** the offset is mutated *only* at those two sites; `rewrap_lines`, the width-reflow path, rebuilds `self.lines` into a fresh `VecDeque` without touching the offset at all ([L104-182](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L104-L182)) — so lineage across reflow/width-change is **not demonstrated to be preserved**, contrary to the first version of this report's "lost only on eviction" claim. `StableRowIndex` is a real, verified eviction-lineage primitive, strictly better than Alacritty's (none), but it is **unproven, not proven,** for the reflow-lineage case climon2 actually needs; no public row-count/capacity accessor was found alongside it either. |
| OSC 8, bounded | **V** model; **U, no length cap found at this layer** | Arc-shared `Hyperlink{params,uri,implicit}` per cell ([hyperlink.rs#L9-17,76-99](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-escape-parser/src/hyperlink.rs#L9-L17), [wezterm-cell/src/lib.rs#L92-97](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-cell/src/lib.rs#L92-L97)); no explicit URI/ID byte-length check in `Hyperlink::parse` — the only indirect bound is the general OSC admission below, which is itself effectively unbounded in the `std` build WezTerm uses. **Materially weaker than Ghostty's explicit 2048-byte OSC cap on this specific axis.** |
| Kitty keyboard: set/push/pop/query consistency | **V — no mismatch found** | Per-screen `keyboard_stack: Vec<KeyboardEncoding>`, push capped at 128 frames ([screen.rs#L35-36](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L35-L36), [performer.rs#L523-571](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/performer.rs#L523-L571)); the encoder's `effective_keyboard_encoding` reads the identical `keyboard_stack.last()` the query reports ([keyboard.rs#L6-15](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/keyboard.rs#L6-L15)) — same single-source-of-truth property as Ghostty, materially better than Alacritty. |
| modifyOtherKeys | **V: real state; query (`CSI ?4m`) confirmed unimplemented** | `modify_other_keys: Option<i64>` set via `CSI>4;Pv m`, consumed by the encoder ([terminalstate/mod.rs#L2069-2077](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/mod.rs#L2069-L2077), [keyboard.rs#L26-38](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/keyboard.rs#L26-L38)). **Correction:** the CSI parser's `xterm_key_modifier` match arm only accepts the `'>'`-intermediate set/reset forms (`CSI > Pr ; Pv m` and `CSI > Pr m`); there is no `'?'`-intermediate arm anywhere in the `'m'` dispatch table, so `CSI ?4m` (xterm's real MOK query) is not recognized as this command at all and falls through to the generic unhandled-CSI path ([csi.rs#L1873](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-escape-parser/src/csi.rs#L1873), [csi.rs#L2134-2168](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-escape-parser/src/csi.rs#L2134-L2168)). This parser-level absence is stronger, more direct evidence than the original report's inference from "no report path found in `performer.rs`" and supersedes it. |
| Parser/history admission | **V — mixed** | `MAX_PARAMS=256` hard CSI cap with a `parameters_truncated` flag, `MAX_OSC=64` field cap with a `full` flag ([vtparse/src/lib.rs#L313-360,508-520,576-580,608](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/vtparse/src/lib.rs#L313-L360)) — these are **field/count** bounds, not byte-length bounds; the OSC payload itself is an unbounded `Vec<u8>` under the `std`/`alloc` feature WezTerm actually builds with (a fixed `heapless::Vec<u8,1024>` exists only in its `no_std` configuration) — **the same class of unbounded-OSC-buffer gap Alacritty has**, not resolved by switching engines. |
| Combining-mark/grapheme handling | **V — no fixed cap found** | `TeenyString` inlines ≤7 bytes, else spills to a heap `TeenyStringHeap{bytes:Vec<u8>}` with no fixed per-cell combining-character count ([wezterm-cell/src/lib.rs#L561-624](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-cell/src/lib.rs#L561-L624)) — this trades Alacritty's hard-but-finite 9-mark cap (a known, bounded fidelity loss) for an apparently **unbounded** per-cell growth path, which is a regression on the admission axis this research cares about, not a free improvement. |
| Licence / build portability | **V — pure Rust, no publication** | MIT, confirmed both at repo root and the `term` subcrate's own `LICENSE.md` ([LICENSE.md](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/LICENSE.md#L1-L22), content-verified above; [term/LICENSE.md](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/LICENSE.md#L1-L23)) — same licence family as Alacritty, no FFI cost, fits ADR 0004 cleanly. **But `wezterm-term` is not published to crates.io** (confirmed 404 via the crates.io API) and has never been independently versioned or release-tagged; pinning it means pinning an upstream monorepo commit with no semver/API-stability contract at all, a materially different and weaker supply-chain posture than Alacritty's or Ghostty's own tagged releases. |

### `libvterm` — checked only to confirm it does not change the picture

**libvterm** (Neovim's maintained fork, branch `nvim`, pinned
`934bc2fbf21800ac3458a499df8820ca5fb45fd3`, content-verified above) has **no OSC 8, no
Kitty keyboard protocol, and no modifyOtherKeys** anywhere in its source — a search for
`hyperlink`/`"OSC 8"`/`kitty`/`modifyOtherKeys` across the whole repository returns zero
matches; where Neovim itself renders OSC 8 links, that logic lives in Neovim's own TUI
front-end, not in libvterm. Reflow exists but is **opt-in and off by default**
(`screen->reflow = false`, toggled via `vterm_screen_enable_reflow()`,
[screen.c#L519-676,891,1050-1058](https://github.com/neovim/libvterm/blob/934bc2fbf21800ac3458a499df8820ca5fb45fd3/src/screen.c#L519-L676)).
This does not materially change the recommendation: libvterm lacks the entire
modern-protocol surface climon2 requires regardless of its reflow quality, so it is not
examined further.

## Embedded-engine recommendation

**Neither alternative is a clean win; each trades one Alacritty gap for a different new
one, and none closes every required gap.**

| Axis | Alacritty (baseline) | Ghostty `libghostty-vt` | `wezterm-term` |
|---|---|---|---|
| Inactive-screen read access | None (destructive `swap_alt()` only) | Internal yes, public C ABI unestablished | None found (same gap class) |
| Reflow (screen + history) | Yes (screen); history reflow exists, needs qualification | Yes, with pointer-pin lineage | Yes, reflow itself works; `StableRowIndex` lineage **not proven across it** (see correction below) |
| Kitty set/query consistency | **Mismatch found** | Consistent | Consistent |
| MOK | Parsed, not implemented | Real state, no query-reply | Real state, no query-reply |
| OSC8 length admission | Unbounded `std` OSC buffer | Explicit 2048B/8MiB cap | Unbounded, same gap as Alacritty |
| Combining-mark cap | Fixed, bounded (9) | Higher, bounded (64) | **None found — unbounded** |
| Build/licence | Rust, Apache-2.0, fits ADR 0004 | **Zig toolchain required to build from source or patch** (confirmed unconditional for that path via `CMakeLists.txt`'s `find_program(ZIG_EXECUTABLE zig REQUIRED)`); not required merely to link the one published prebuilt (Apple-only xcframework); MIT | Rust, MIT, fits ADR 0004, **not published/versioned anywhere** |
| Release/supply-chain posture | Tagged releases exist | Tagged releases exist | No `term`-specific tag ever; pin is a bare monorepo commit |

**Worth a later throwaway prototype: `wezterm-term`, narrowly, on the reflow/lineage
question only — but to *test*, not confirm, lineage.** Its `rewrap_lines` width-reflow
of retained history is real and shipped, but source inspection shows `StableRowIndex`'s
offset is untouched by that path (see correction above), so the question a prototype
would need to answer is whether reflowed rows' identity is recoverable *at all* through
some other means, not whether `StableRowIndex` already solves it — **it does not appear
to, on current evidence.** A small isolated read/reflow/lineage probe is still the
cheapest way to settle this for climon2's specific requirement, rather than accepting
either "solved" or "unsolved" on documentation/naming alone. It is **not** worth
prototyping for Kitty/MOK/OSC8 alone, since it trades Alacritty's bounded
combining-mark and (nominal) OSC cap for unbounded versions of both, and has no public
inactive-screen getter either — i.e. it does not close the keyboard-fidelity gap that
motivated this question at all.

**Ghostty's `libghostty-vt` is not recommended as a prototype candidate yet, pending
the tradeoffs below — this is a recommendation, not settled evidence that it is
disqualified.** Its Kitty consistency and real (if incomplete) MOK state are genuine
improvements, and its explicit OSC/DCS byte caps are the only engine here that already
enforces *any* control-string admission bound. The costs to weigh explicitly in #43:
building from source (needed for any of the narrow API-surface patches this and the
embedding research identified, e.g. inactive-screen access) unconditionally requires a
second, non-Rust toolchain (confirmed via `CMakeLists.txt`'s
`find_program(ZIG_EXECUTABLE zig REQUIRED)` — consuming only the published prebuilt
avoids this but is Apple-only and offers no patch path); its public C header is
explicitly marked unstable by its own authors; and no Linux/Windows prebuilt exists.
**Correction:** the first version of this report claimed these costs conflicted with
ADR 0003/0004 and called the Zig dependency unconditional outright — neither is
accurate. No exact ADR text forbids a second build toolchain, and the Zig requirement
applies to source-build/patching, not to prebuilt consumption. The user has also
already approved source-level pinning/patching elsewhere (the Alacritty embedding
research), so an unstable upstream API is a cost to weigh, not an automatic
disqualifier on its own. A six-months-later recheck of `include/ghostty/vt/*.h`
stability remains a reasonable future action.

**None of this closes or narrows the still-open daemon-side questions.** Inactive-grid
access, history-lineage/eviction notification, and byte-accounted memory admission
remain open regardless of which embedded engine is chosen — only their *shape* differs
per candidate above, not their presence. **Changing the embedded engine cannot by
itself repair the outer-host keyboard-fidelity question that prompted this research** —
that depends entirely on the native host section below.

## Outer macOS host comparison: Kitty, Ghostty, WezTerm added alongside Terminal.app/iTerm2/VS Code

This **adds** Kitty, Ghostty, and WezTerm as additional native-host candidates. It does
**not** replace or relax the existing required qualification of Terminal.app, iTerm2, or
VS Code from [#54](https://github.com/jackgeek/climon2/issues/54); that remains
necessary regardless of how many additional hosts are evaluated here, and none of those
three prior hosts' status changes.

| Host (pin) | OSC 8 | Kitty keyboard native encoder | modifyOtherKeys |
|---|---|---|---|
| **Kitty `v0.49.2`** (`b2cb871254fbdac19a085188813c8dfb258b95cd`) | **V** — `MAX_KEY_LEN=2048`, `MAX_ID_LEN=256`, id+url grouping ([hyperlink.c#L12-13,136-160](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/hyperlink.c#L12-L13), content-verified above) | **V — reference implementation.** Full press/repeat/release enum, set/push/pop/query share one backing array ([key_encoding.c#L12-88](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/key_encoding.c#L12-L88), [screen.c#L2092-2152](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/screen.c#L2092-L2152)); macOS repeat is computed OS-independently in its own GLFW-derived input layer, not read from `NSEvent.isARepeat` ([glfw/input.c#L266-299](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/glfw/input.c#L266-L299)) | **V — deliberately a no-op stub, by design.** `screen_modify_other_keys` only logs a warning telling the application to use Kitty's own protocol instead ([screen.c#L2082-2090](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/screen.c#L2082-L2090), content-verified above). **Correction:** this is *not* disqualifying for a session that needs virtual MOK under the accepted daemon-translation architecture — see "what this changes" below. It only means Kitty-as-host cannot answer a literal MOK set/query on its own wire; it still supplies a fully-implemented, genuine Kitty-protocol key-event stream the daemon can decode and re-encode as MOK. |
| **Ghostty `v1.4.0`** (`83fb95624b0439ee660c2679b43b38c8b63769cd`) | **V** — `.fixed` capture, **2048-byte total cap on the whole `8;id=...;URI` payload** ([osc.zig#L360-365,1060-1066](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/osc.zig#L1060-L1066), [osc/parsers/hyperlink.zig#L1-52](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/osc/parsers/hyperlink.zig#L1-L52)). **This is below climon2's required 8192-byte URI / 256-byte ID model ceiling — a concrete native-host capture gap, not just a model/parser detail: switching the embedded engine cannot raise an outer host's own native OSC8 capture limit**, since the limit is enforced in the terminal application that receives the escape sequence from the real program, independent of which VT library climon2's daemon embeds. | **V** — standalone native encoder `src/input/key_encode.zig` with distinct press/repeat/release, confirmed present with identical content and line numbers at the `v1.4.0` tag itself ([key_encode.zig#L250-252,294-310,336-337](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/input/key_encode.zig#L250-L252)). **Correction:** the first version of this report cited this file only at an unpinned HEAD commit and flagged it "non-pinned"; the same file and line numbers resolve cleanly at the already-pinned `v1.4.0` tag, so no HEAD caveat is needed. macOS `keyDown`/`keyUp` map `event.isARepeat` to a distinct repeat action at the same tagged pin ([SurfaceView_AppKit.swift#L1120-1172,1291-1292](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/macos/Sources/Ghostty/Surface%20View/SurfaceView_AppKit.swift#L1120-L1172)) | **V — state only, no query-reply** (same absence confirmed in the embedded-engine table above, since Ghostty's native host and embedded library share the same `terminal/` core) |
| **WezTerm `main`** (`e68a8ec27727fbbda15e07f0a0737cd5e16481f6` — **no recent dated release tag**; last application tag `20240203-110809-5046fc22`, ~2.7 years stale as of this research date) | **V** model; **U** no explicit length cap found ([hyperlink.rs#L1-17](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-escape-parser/src/hyperlink.rs#L1-L17)) | **V, with a confirmed gap.** `encode_kitty()` encodes release (`:3`) but **never emits repeat (`:2`)**, despite a `repeat_count` field existing on the input event and going unused by the encoder ([wezterm-input-types/src/lib.rs#L1325,1492,1711-2020,1767-1773](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-input-types/src/lib.rs#L1711-L2020)). The macOS layer does read `nsevent.isARepeat()`, but only to drive an unrelated IME workaround — it never reaches the Kitty encoder, and `repeat_count` is hard-coded to 1 ([window/src/os/macos/window.rs#L2548-2550,2612,2655,2771-2784](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/window/src/os/macos/window.rs#L2548-L2550)). **This is a concrete, source-verified repeat-identity loss for WezTerm-as-outer-host, directly analogous to the VS Code associated-text repeat-identity defect already found in prior research**, not a hypothetical. | **V — state only, no query-reply** (same shape as the embedded-engine finding, same source) |

**Unexecuted for all three:** no live keyboard probing, GUI automation, or app launch was
performed for Kitty, Ghostty, or WezTerm. All claims in this table are source- or
doc-reading only. None of this qualifies any host end-to-end; it only establishes
whether a genuine native event-encoding path exists in source, as distinct from a VT-side
parser or query echo.

### What this changes and does not change about host qualification

- **Kitty's deliberate MOK no-op does not disqualify Kitty-as-host under the accepted
  daemon-translation architecture.** The first version of this report wrongly concluded
  that a session needing MOK "cannot rely on Kitty-as-host for it." Kitty's own
  maintainer does deliberately stub MOK set/query in favor of Kitty's own protocol, and
  that stub is source-confirmed and will not change with more probing — but the user has
  already approved **observed-input decoding plus daemon-side encoding**, not protocol
  passthrough: the daemon can take Kitty's fully-implemented, reference-quality
  Kitty-protocol key events (real press/repeat/release, modifiers, associated text) and
  re-encode them into MOK-style sequences for a command that expects MOK. Kitty-as-host
  is disqualified only for a design that required the outer host to answer a literal
  `CSI ?4m` on its own wire, which is not this architecture.
- **Ghostty is the strongest additional native-host candidate found**: genuine
  OS-level `NSEvent.isARepeat` plumbing to a distinct repeat action, a standalone
  key-encoder module, and the same real-but-incomplete MOK state as its embedded library.
  It remains source-verified only, not natively qualified — the usual gap between "an
  encoder function exists and does X" and "proven across real OS delivery, IME, layouts,
  and focus edge cases."
- **WezTerm-as-outer-host has a concrete, newly-found repeat-identity defect** in its
  Kitty encoder, independent of the embedded-engine question. This is the same *class*
  of problem already documented for VS Code's packaged xterm.js (repeat indistinguishable
  from press), now also confirmed for WezTerm, for a different underlying reason
  (unused field vs. unified associated-text encoding path). It does not disqualify
  WezTerm outright, but any required-profile fidelity check should test for this
  specifically rather than assume "has Kitty support" implies full event-type coverage.
- **Terminal.app, iTerm2, and VS Code's existing qualification status is unchanged.**
  This report adds comparison data; it makes no claim about those three that
  [#54](https://github.com/jackgeek/climon2/issues/54) did not already establish, and
  Terminal.app's extended-keyboard support remains unqualified exactly as before.

## Outstanding embedding/accounting/lineage seams (explicit, for the next grilling round)

These persist across **every** engine examined, including both alternatives:

1. **Inactive-screen read access** is missing or unestablished in all three Rust/Zig
   candidates at the public-API layer (Alacritty: none; WezTerm: none found; Ghostty:
   internal yes, public C ABI unestablished). No candidate offers a safe, non-mutating
   way to read the screen not currently displayed.
2. **No candidate has a complete, bounded OSC/DCS byte-admission story that already
   matches climon2's agreed 64 KiB control-string / 4 KiB title ceilings out of the box.**
   Ghostty is closest (explicit 2048B/8MiB caps, still below climon2's 8192-byte URI
   ceiling); Alacritty and WezTerm both default to effectively unbounded OSC accumulation
   in their actually-used build configuration.
3. **Combining-mark/grapheme admission varies by an order of magnitude and direction**
   across candidates (Alacritty: hard 9; Ghostty: higher bounded 64; WezTerm: apparently
   unbounded) — this is a real admission-policy difference an engine switch would
   introduce, not a wash.
4. **Correction: xterm itself does define a real MOK query/reply (`CSI ?4m` →
   `CSI >4;L m`), already verified from xterm's own patch-411 source in
   [#54](https://github.com/jackgeek/climon2/issues/54); the first version of this
   report wrongly claimed the opposite.** What holds is narrower: neither Ghostty nor
   WezTerm *implements* that query, each confirmed here by direct inspection of their
   respective `CSI m` dispatch tables (Ghostty: an explicit "ignoring unimplemented"
   fallback with a source comment showing the authors don't know what the sequence
   means; WezTerm: no `'?'`-intermediate match arm exists for `'m'` at all). This is an
   implementation gap in both candidates, not a protocol-design limitation — a future
   engine or a source patch to either could add it, unlike a genuinely protocol-absent
   feature.
5. **History-lineage models differ in shape and are weaker than first claimed.**
   Alacritty has none; Ghostty uses pointer-pin remapping; WezTerm's `StableRowIndex` is
   a monotonic `phys_index + eviction_count` offset, correctly maintained across plain
   top-of-screen scrolling and `erase_scrollback` ([screen.rs#L734,769](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L734)), but **`rewrap_lines` (the width-reflow path) rebuilds the entire physical row sequence from scratch into a new `VecDeque` without recomputing, compensating, or otherwise touching `stable_row_index_offset`** ([screen.rs#L104-182](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L104-L182)) — the offset is modified only at the two scroll/erase sites above, never during resize. **Correction: the original claim that lineage is "lost only on eviction" was wrong; it is not demonstrated to survive reflow at all**, which is exactly the regime climon2 most needs. No candidate has been proven against climon2's requirement (stable identity across reflow, reset, *and* viewport transitions together), and WezTerm is weaker on this specific axis than first reported, not stronger.
6. **Supply-chain/versioning risk differs sharply**: Alacritty and Ghostty have real
   tagged releases; `wezterm-term` has never been independently tagged or published,
   meaning a pin is a bare monorepo commit with no semver contract at all — a genuinely
   new risk class relative to the current baseline, not a neutral alternative.
7. **A Zig/C toolchain dependency is real but narrower than first stated.** It is
   required for building Ghostty's library from source or applying any patch to its
   explicitly-unstable C header (both would be needed for the narrow seams this report
   and the prior Alacritty embedding research identified, e.g. inactive-screen access);
   it is **not** required merely to link the one published prebuilt artifact (an Apple
   xcframework — macOS-only, no Linux/Windows prebuilt). **Correction: the first version
   of this report invented a conflict with ADR 0003/0004 that neither ADR's text
   actually states** — those ADRs describe platform-`cfg` seams and crate/dependency
   direction, not a single-toolchain constraint. The real, narrower cost is: a second
   build toolchain for anyone building climon2 from source on non-macOS platforms, plus
   patching against an interface its own authors say will change. That is a tradeoff to
   weigh explicitly in #43, not an automatic disqualifier, and not newly forbidden by any
   existing ADR.
8. **Kitty-as-host's deliberate MOK no-op does not disqualify it under the user's
   accepted daemon-translation architecture.** The first version of this report
   wrongly treated "Kitty doesn't implement MOK" as blocking a command that needs
   virtual MOK. Under the approved design, the daemon does not require the outer host to
   speak the same wire protocol the command sees: if Kitty-as-host supplies genuine,
   qualified Kitty-protocol key events (press/repeat/release, modifiers, associated
   text — which Kitty's own reference encoder does provide, see the host table above),
   the daemon can decode those real key/modifier facts and re-encode them as MOK-style
   sequences for a command that asked for MOK. This is **translated acquisition**, not
   protocol passthrough, and is categorically different from requiring Kitty itself to
   answer a MOK query. Kitty's MOK stub remains relevant only if the architecture ever
   needed Kitty to answer a literal `CSI ?4m` on the outer wire, which it does not under
   the accepted design.

## Primary-source references

Pinned revisions used in this report, independently reverified via the GitHub Git Data
API (tag → commit dereference) and raw-blob HTTP 200 + content spot-checks during this
report's own verification pass, in addition to the delegated research pass that located
them:

- Ghostty: tag `v1.4.0` → commit `83fb95624b0439ee660c2679b43b38c8b63769cd`
  ([API dereference](https://api.github.com/repos/ghostty-org/ghostty/git/refs/tags/v1.4.0)).
- Kitty: tag `v0.49.2` → commit `b2cb871254fbdac19a085188813c8dfb258b95cd`
  ([API dereference](https://api.github.com/repos/kovidgoyal/kitty/git/refs/tags/v0.49.2)).
- WezTerm: `main` HEAD `e68a8ec27727fbbda15e07f0a0737cd5e16481f6` (2026-10-26) — **not a
  tagged release**, used because `term`/`wezterm-cell`/`wezterm-escape-parser` postdate
  WezTerm's last dated application tag `20240203-110809-5046fc22` (Feb 2024); also
  cross-checked against tag `termwiz-0.23.3` → `4bf28e253c0167102f07bfc7e7199c13eed98012`
  (2025-03-20) where structurally unchanged.
- libvterm: Neovim fork, branch `nvim`, commit `934bc2fbf21800ac3458a499df8820ca5fb45fd3`.

Key per-engine source citations (full set, including those not independently
re-verified in this pass, is in the delegated research transcript this report
synthesizes):

- [Ghostty `build.zig` embeddable targets](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/build.zig#L128-L225)
- [Ghostty `include/ghostty/vt.h` — unstable warning](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/include/ghostty/vt.h#L1-L26)
- [Ghostty `ScreenSet.zig` — inactive-screen accessor](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/ScreenSet.zig#L17-L21)
- [Ghostty `PageList.zig` — reflow and `Pin`/`tracked_pins` lineage](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/PageList.zig#L1261-L1321)
- [Ghostty `hyperlink.zig` / `osc.zig` — OSC8 model and 2048B/8MiB caps](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/osc.zig#L359-L365)
- [Ghostty `stream_terminal.zig` — Kitty set/push/pop/query on one field, MOK state](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/stream_terminal.zig#L715-L726)
- [Ghostty `dcs.zig` — DECRQSS enum has no MOK entry (a different protocol; not evidence about `CSI ?4m`), DCS 1MiB cap](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/dcs.zig#L17-L267)
- [Ghostty `stream.zig` — `CSI ?4m` MOK query reaches an explicit "ignoring unimplemented CSI m with intermediates" fallback](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/stream.zig#L2280-L2291)
- [Ghostty `page.zig` — 64-codepoint grapheme cap](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/terminal/page.zig#L84-L95)
- [Ghostty `LICENSE` — MIT](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/LICENSE#L1-L4)
- [Ghostty `CMakeLists.txt` — Zig required for source build, not for prebuilt consumption](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/CMakeLists.txt#L1-L5)
- [Ghostty macOS `SurfaceView_AppKit.swift` — `isARepeat` → repeat action](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/macos/Sources/Ghostty/Surface%20View/SurfaceView_AppKit.swift#L1120-L1172)
- [Ghostty `key_encode.zig` — native encoder press/repeat/release, confirmed at the `v1.4.0` tag itself (corrects prior non-pinned HEAD citation)](https://github.com/ghostty-org/ghostty/blob/83fb95624b0439ee660c2679b43b38c8b63769cd/src/input/key_encode.zig#L250-L337)
- [WezTerm `term/Cargo.toml` — crate identity, no release publication](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/Cargo.toml#L1-L39)
- [WezTerm `term/src/terminal.rs` — headless constructor](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminal.rs#L143-L171)
- [WezTerm `term/src/terminalstate/mod.rs` — screen/alt model, no public inactive-screen getter](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/mod.rs#L150-L178)
- [WezTerm `term/src/screen.rs` — `rewrap_lines` reflow rebuilds `self.lines` without touching `StableRowIndex`'s offset; offset mutated only at scroll-eviction/`erase_scrollback`](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L104-L182)
- [WezTerm `term/src/screen.rs` — the two `stable_row_index_offset` mutation sites (scroll-eviction, `erase_scrollback`)](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/screen.rs#L734)
- [WezTerm `wezterm-escape-parser/src/hyperlink.rs` — OSC8 model, no length cap](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-escape-parser/src/hyperlink.rs#L9-L99)
- [WezTerm `wezterm-cell/src/lib.rs` — `TeenyString`, no combining-mark cap](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-cell/src/lib.rs#L561-L624)
- [WezTerm `term/src/terminalstate/performer.rs` / `keyboard.rs` — Kitty stack and MOK real-state consistency](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/term/src/terminalstate/keyboard.rs#L6-L38)
- [WezTerm `wezterm-escape-parser/src/csi.rs` — `'m'` dispatch table and `xterm_key_modifier` only match `'>'`-intermediate forms; `CSI ?4m` is not recognized as this command](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-escape-parser/src/csi.rs#L2134-L2168)
- [WezTerm `vtparse/src/lib.rs` — param/OSC-field caps, unbounded `std` OSC byte buffer](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/vtparse/src/lib.rs#L313-L360)
- [WezTerm `LICENSE.md` — MIT, corrects this ticket's GPL-3.0 premise](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/LICENSE.md#L1-L22)
- [WezTerm `wezterm-input-types/src/lib.rs` — `encode_kitty()` never emits repeat](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/wezterm-input-types/src/lib.rs#L1711-L2020)
- [WezTerm macOS `window.rs` — `isARepeat()` read but not forwarded to the Kitty encoder](https://github.com/wezterm/wezterm/blob/e68a8ec27727fbbda15e07f0a0737cd5e16481f6/window/src/os/macos/window.rs#L2548-L2784)
- [libvterm `screen.c` — reflow opt-in/off by default, no OSC8/Kitty/MOK anywhere](https://github.com/neovim/libvterm/blob/934bc2fbf21800ac3458a499df8820ca5fb45fd3/src/screen.c#L519-L1058)
- [Kitty `hyperlink.c` — OSC8 `MAX_KEY_LEN=2048`, `MAX_ID_LEN=256`](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/hyperlink.c#L12-L160)
- [Kitty `key_encoding.c` / `screen.c` — reference Kitty-protocol encoder](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/key_encoding.c#L12-L88)
- [Kitty `screen.c` — `screen_modify_other_keys` deliberate no-op stub](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/kitty/screen.c#L2082-L2090)
- [Kitty `glfw/input.c` — repeat computed independently of `NSEvent.isARepeat`](https://github.com/kovidgoyal/kitty/blob/b2cb871254fbdac19a085188813c8dfb258b95cd/glfw/input.c#L266-L299)
- [Kitty keyboard protocol specification (canonical)](https://sw.kovidgoyal.net/kitty/keyboard-protocol/)
- ["libghostty is coming" (Mitchell Hashimoto, Sept 2025) — documented intent, superseded by shipped `build.zig`/headers above](https://mitchellh.com/writing/libghostty-is-coming)

Inherited, not repeated: baseline Alacritty/`vt100` engine facts
([session-continuity-terminal-engine.md](https://github.com/jackgeek/climon2/blob/research/session-continuity-terminal-engine/docs/research/session-continuity-terminal-engine.md)),
Alacritty embedding seams
([alacritty-embedding-contract.md](https://github.com/jackgeek/climon2/blob/research/alacritty-embedding-contract/docs/research/alacritty-embedding-contract.md)),
and OSC8/keyboard host facts for Terminal.app/iTerm2/VS Code
([terminal-hyperlink-keyboard-capabilities.md](https://github.com/jackgeek/climon2/blob/research/terminal-hyperlink-keyboard-capabilities/docs/research/terminal-hyperlink-keyboard-capabilities.md)).

## What this research does and does not authorize

No product implementation, dependency change, engine switch, or host-support downgrade
is authorized by this report. It does not accept a qualified-profile limitation and does
not resolve [#43](https://github.com/jackgeek/climon2/issues/43), which remains the live
decision on engine, module boundary, and failure policy. It adds one narrow, bounded
prototype candidate (`wezterm-term`'s reflow/lineage model) and one deferred recheck
(Ghostty's C ABI stability at a future tagged release) as actionable follow-ups, and
leaves every other open seam from prior research exactly as open as before.
