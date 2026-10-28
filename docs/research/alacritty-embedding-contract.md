# Alacritty embedding contract: pinned facts and remaining seams

Research for [Alacritty embedding: state access, history identity, and enforceable budgets](https://github.com/jackgeek/climon2/issues/53), 2026-10-28.
This supplies evidence to [Daemon virtual terminal: engine, ownership, and failure boundaries](https://github.com/jackgeek/climon2/issues/43); it does not resolve that live decision.
Alacritty family, in-process execution, daemon-owned source-of-truth VT and sole query authority are given. No client emulator, product implementation, engine comparison, native-host qualification or new numeric product ceiling is proposed.

## Exact target and publication check

**Candidate: `alacritty_terminal` at `bbb2cbf9470a6707b1205fbddbdb25109b14e2fd`, package version `0.26.1-dev`.**
Its [manifest][manifest] explicitly enables `vte`'s `std` and `ansi` features, even with Alacritty's `default-features = false`.
`git merge-base --is-ancestor` succeeded for the [combining-character fix `ede2ac144da4dec4c075bfa803aacf3b3739bce6`][fix] and this target. The target uses an inline `ArrayVec<char, 9>` in dynamically allocated cell extras; additional combining characters are silently ignored by `try_push`, **not** reported as an engine failure ([implementation][cell]). This is bounded fidelity loss, not an unrestricted grapheme model.

The [pinned registry index snapshot][registry] ends at `0.26.0`, published 2026-04-06, not yanked. A fresh sparse-index fetch on this research date had the same latest entry. The first-party GitHub releases listing likewise returned application `v0.17.0` as latest published release; its [terminal manifest][release-manifest] is `0.26.0`. **No published successor containing this fix was found.** Application and terminal-library version numbers differ. The exact candidate above has not been silently replaced. `git ls-remote` also returned this candidate as current `master`/HEAD at observation time; that moving reference is not the pin. The existing test harness's released `0.26.0` dependency does not select a shipped pin or prove these guarantees.

For parser findings below, the candidate's lockfile selects [`vte 0.15.0`][lock]; primary-source pin is `3b3da71c34cc1256c7e20981cf03f8eb95e08ffc` (its `v0.15.0` tag). The registry crate's `src/lib.rs` and `src/ansi.rs` were byte-compared with that checkout and matched. A future shipped build still needs a reviewed dependency lock; pinning only the engine Git revision does not freeze its semver dependencies.

## Public interface / requirement matrix

“Gap” means absent from this pinned public contract, not impossible to implement.

| Requirement | Public interface and exact limitation | Result |
|---|---|---|
| Active normal **or** alternate screen; retained normal rows when normal is active | `Term::grid/grid_mut`, `Dimensions::{history_size,screen_lines,columns}`, grid row/point indexing and `iter_from`. History is relative negative `Line` indices; screen begins at zero ([grid][grid-api], [dimensions][dimensions]). | Public, no stable identity implied. |
| Underlying normal screen/history while alternate is active; inactive alternate | `Term` has both grids but `inactive_grid` is private, with no immutable getter ([fields][term-fields], [accessors][term-access]). `Term<T>`'s `T` is its event listener, not a replacement cell type. | Narrow read-only inactive-grid extension/patch is the direct seam. |
| Use `swap_alt()` to inspect inactive data | Public but entering alternate copies cursor state, overwrites normal saved cursor, clears alternate contents, swaps keyboard stacks, clears selection and damages state ([implementation][term-access]). | **Not** an observational getter; round-trip swapping can destroy the state being restored. |
| Text, colors, combining marks, rendition, wide-character and soft-wrap provenance | `Cell::{c,fg,bg,flags,extra}`, `zerowidth`, underline/hyperlink accessors; flags include `WRAPLINE`, `WIDE_CHAR`, both spacer forms ([cells][cell]). `wrapline` records `WRAPLINE`; LF takes a separate path ([wrap][wrap], [LF][linefeed]). | Sufficient ingredients for a deliberately constructed history renderer; not a ready serializer. Absence of wrap is a row boundary, **not proof** that an LF was received: erasures, resize and edits also affect rows. |
| Visible cursor, effective modes and palette | `grid.cursor`, `saved_cursor`, pending wrap, cursor template; `Term::mode`, `cursor_style`, `colors`, `renderable_content` ([cursor][grid-cursor], [accessors][term-access], [render view][render]). | Public effective rendering state. Hidden tabs, scroll region, charset selector and keyboard stacks remain private ([fields][term-fields]); not a full terminal-machine checkpoint. |
| Query authority | `Handler for Term` emits `Event::PtyWrite` for DA/DSR and mode reports; color/clipboard/area queries emit callback-bearing events ([queries][queries], [events][events], [reports][reports]). | Public event sink can route replies to the command once. Default `EventListener::send_event` does nothing: a no-op sink silently loses replies. Callback queries require daemon-provided policy/data, not forwarding to arbitrary clients. |
| Normal/history reflow while either screen is active | `Term::resize` reflows the normal grid, including when inactive, and does not reflow alternate ([resize dispatch][term-access]). Grid resize splits/merges wrapped rows with wide-spacer handling; narrowing truncates oldest rows to row limit ([algorithm][reflow]). | Public resize mechanism, no returned old-to-new position map. It returns `()`, not a fallible transactional result. |
| Stable delivered suffix across eviction, reset and resize | No row IDs, generations, sequence positions, eviction callbacks or reflow mapping in the grid/resize signatures or event enum ([grid][grid-api], [reflow][reflow], [events][events]). Damage is rendering damage, not a history mutation journal ([damage][damage]). | **Missing turnkey contract.** Relative row numbers/history length cannot serve as delivery positions. |
| Finite row retention / clear | `Config::scrolling_history`, `set_options`, `Grid::update_history/clear_history` ([config][term-fields], [set options][set-options], [history API][history-api], [row limit update][grid-api]). | Public oldest-row reduction possible on active grid; `set_options` updates normal history even when inactive, but also emits title/damage work. Byte accounting is separate. |
| Allocated history/cached-row byte introspection | `Grid::raw`, `Storage::inner`, `Row::inner` and their capacities are private ([storage][storage], [row][row], [grid fields][grid-api]). `len`/`history_size` expose logical rows, not allocations. | No exact public capacity accounting; see accounting seam below. |

Query policy remains a human decision, even with one authority: built-in DA identifies VT102-style primary attributes (`ESC[?6c`) and generates secondary attributes from the crate package version, not a configurable product identity ([queries][queries]). Color, pixel geometry and clipboard callbacks require explicit answers/denials; `WindowSize` contains `u16` fields ([events][events]). OSC52 defaults to copy-only but can be disabled; disabling its semantic handler **does not bound the parser's pre-dispatch OSC buffer** ([config][term-fields], [clipboard][clipboard], [OSC buffer][osc-buffer]). Nothing here proves that raw live bytes can follow a cell snapshot faithfully or that outer-terminal replies cannot duplicate the daemon's replies. Live rendering architecture, terminal identity and response isolation stay open.

## History identity: what a public adapter can and cannot infer

Source-only inference from [scrolling][scroll], [clear/reset][clears], [reflow][reflow] and the [event list][events]:

- History changes on top-origin scroll; a subregion not starting at the top moves cells without adding history. Plain LF is not the only cause: deferred wrap, explicit scroll, screen clear and height resize can move rows into history. At a full history cap, appending and evicting leaves `history_size()` unchanged. Repeated identical rows make content-diff matching ambiguous.
- History rows are not immutable “finalized records”: height growth can pull them back into the editable screen; width changes join/split across the screen/history boundary; history clear/reset removes them. Reset/erase generations must be semantic events, not inferred from “empty history.” A reset with already-empty history is still a reset.
- Public `vte::ansi::Handler` allows a wrapper to intercept semantic operations and delegate to `Term` ([trait][handler], [Term implementation][input]). This could maintain daemon IDs/generations, observe state before/after each semantic operation, and separately wrap resize. It must forward the **whole supported handler surface**, track scroll regions/alt transitions and reproduce reflow lineage semantics; no built-in mutation callback does that for it. Byte-at-a-time feeding alone does not provide missing identities or a reflow map, and synchronized updates can defer many operations.
- A daemon-owned logical-history sidecar is a public-API possibility, not a demonstrated drop-in solution. It needs a proven mapping through split/merge, wide spacers, eviction and screen pullback, and must charge its duplication/index/delivery metadata. Matching text or physical row ordinals cannot establish that proof. A custom `Grid<T>` can carry richer cells, but `Term` internally fixes its grid to `Cell` ([fields][term-fields]).

**Narrow extension/patch candidates, not an architecture selection:**

1. Immutable access to both identified grids and any additional restore state actually required by the chosen live renderer. Do not mutate the engine merely to inspect the inactive normal state.
2. Ordered semantic history notifications / durable logical anchors at the mutation sites: history entry, removal reason/count or anchors, clear/reset generation, and old-to-new lineage across resize/reflow, including rows crossing the viewport boundary. Notifications must occur before discarded information is gone; a callback reporting only the final row count is insufficient.
3. Allocation statistics and explicit compaction/retention operations covering both grids, cached rows and string capacities, plus a place to reserve/check history storage before growth if the policy promises no transient budget excess.

The source establishes the missing seams, **not** that this exact extension is already upstream or that an implementation has passed all scrollback cases. A public forwarding/sidecar solution versus a narrow maintained patch is a remaining decision for [Daemon virtual terminal: engine, ownership, and failure boundaries](https://github.com/jackgeek/climon2/issues/43). Existing-client acknowledgements and gap/reset delivery ordering remain with [Protocol 2: atomic restore, live output, and slow clients](https://github.com/jackgeek/climon2/issues/48). A resize cannot simply restart IDs and resend everything to an existing connection without reconciling its delivered position.

## Resource facts and honest history-storage accounting

### Actual cells and allocated rows

Executed probe on **x86_64 macOS, rustc 1.96.0 (`ac68faa20`), default-features=false**:

| `size_of` type | Bytes | Excludes |
|---|---:|---|
| `Cell` | 24 | pointed-to extra/hyperlink allocations |
| `CellExtra` | 56 | its Arc header and pointed-to hyperlink |
| `Hyperlink` | 8 | inner Arc object and ID/URI string allocations |
| `Row<Cell>` | 32 | backing cell vector allocation |

These are measured target/compiler layouts, **not** stable ABI or universal constants. The [cell source][cell] shows nine inline `char` slots in `CellExtra`, optional underline color and optional shared hyperlink; there is no separate per-cell combining-mark vector at this pin. The extra is still a lazy `Arc` allocation, potentially copied on write. Hyperlink inner contains two heap `String`s and is separately shared.
The [pinned Rust Arc layout][arc] has strong/weak `usize` counters: on this measured ABI an extra requires a 72-byte Arc object (56 + 16), excluding allocator overhead; the hyperlink inner's two 24-byte strings imply a 64-byte Arc object (48 + 16), **plus the string capacities**, not just UTF-8 lengths. Those Arc totals are source/layout arithmetic, not measured allocator usable sizes. Shared objects can be conservatively charged once per referencing cell instead of deduplicated; templates, cached rows and sidecars must not disappear from the ledger.

`MAX_CACHE_SIZE = 1_000` is **cached row objects**, not history retention or bytes. [Storage][storage] initializes at least 1,000 additional rows when it grows by a smaller amount; shrink can leave up to 1,000 allocated inactive row objects with their cell payloads. `Storage::truncate` drops inactive rows but calls `Vec::truncate`, **not capacity shrink**. Even clear-history may therefore retain cells in the cache; explicit `Grid::truncate` drops those rows, yet the outer row-vector capacity remains. `Row::shrink` uses `split_off`; a shortened row can keep its old cell capacity. Private row capacity can outlive width reduction ([row][row]). Reflow also temporarily holds multiple row vectors and splits/merges cell vectors before final oldest-first truncation ([reflow][reflow]).

### Row ceiling: important correction

**There is no enforced hard library history-row ceiling of 100,000 at this pin.** `Config::scrolling_history` and `Grid::max_scroll_limit` are `usize`, passed through without that check ([constructor][options], [set options][set-options], [grid][grid-api]). The **Alacritty application's** configuration deserializer rejects above `MAX_SCROLLBACK_LINES = 100_000` ([application config][app-limit]); embedding the library bypasses it. A tiny executed `Grid<Cell>` probe retained **100,001 history rows** at width 2. This was an interface check, not a product configuration recommendation.

This does not make arbitrary `usize` values safe: `Line` is signed `i32`, and dimensions/history arithmetic casts to `i32` and adds/multiplies counts ([dimensions][dimensions], [line type][line-type], [storage indexing][storage-index]). Documented terminal minima are **2 columns / 1 visible line** ([minima][minima]); `Term::new/resize` trust the caller instead of enforcing a complete dimensions policy ([options][options], [term resize][term-access]). Accepted rows, dimensions, their products and transient reflow expansion need checked admission well inside representation/allocator limits. No silent clamp or inferred “unlimited” support.

### What a defensible ledger needs

The agreed policy in [Scrollback: retention, navigation, and attachment experience](https://github.com/jackgeek/climon2/issues/49) is defaults **10,000 rows / 33,554,432 bytes**, maximum **268,435,456 bytes**, excluding current screens but including associated retained-history allocations. These are inherited product decisions, not newly chosen limits.

A conservative **owned/requested storage charge** can overcount shared payloads. It must account for:

```
history-attributable outer row-container capacity and row objects
+ capacities of all history-attributable cell vectors (including cache)
+ extra Arc objects and hyperlink Arc/string capacities
+ history IDs/lineage/delivery indexes and any duplicated retained content
+ explicit conservative padding/allocation allowances required by the policy
```

Screen allocation is separately bounded; shared screen/history container slack needs an explicit attribution rule, not double-exclusion. It is permissible to overcharge all ambiguous cache/container slack to history. Rows are evicted oldest-first until both logical rows and this charge fit; lowering a logical limit without dropping charged capacity is not sufficient. If the associated minimum allocation cannot fit, that needs a defined explicit result, not a fake successful budget check.

**Not sufficient:** `history_rows × columns × 24`, serialized text length, or `24 + 9×4` per cell. They omit caches, capacity, Arc allocations, hyperlink strings and metadata. No finite universal per-cell payload upper bound follows from the combining fix, because URI/ID strings are not bounded at this engine pin and the std OSC parser grows across feeds ([cell][cell], [OSC dispatch][osc-dispatch], [OSC buffer][osc-buffer]).

**Public-only boundary:** logical row eviction is available, but this source exposes neither all allocated row/cell/string capacities nor all cached contents. A public adapter can charge its own explicitly bounded store exactly in its chosen accounting units; that alone does not cover engine history duplication/cache. A source-derived worst-case high-water model is possible only with additional proven admission limits and pinned allocation behavior; this research has **not** proved one. Do not declare the byte contract met from row count or a guessed Vec growth factor.

The narrow stats/compaction seam above can support conservative requested-storage accounting. Even `capacity × size_of` is not exact allocator usage: [Rust Vec guarantees][vec] explicitly permit allocator excess, do not guarantee a growth strategy and do not automatically shrink. Allocator headers, fragmentation, retained freed pages, stacks and unrelated allocations are outside that metric. Exact usable allocation would require allocator-specific instrumentation; **RSS is not derivable from this ledger**. Resize/copy-on-write/event construction may temporarily exceed a steady-state charge unless reservation/preflight or a separately qualified transient policy handles them.

## Parser, admission and synchronous work limits

All statements here concern the pinned parser and engine paths, not a general vulnerability audit.

| Known bound / lack of bound | Consequence |
|---|---|
| VTE stores **32 total parameters/subparameters**, each `u16`, and **2 intermediate bytes**; numeric parsing saturates to `u16` ([params][params], [parser fields][parser], [numeric parse][numeric]). | Bounded CSI metadata, not bounded semantic work. |
| OSC has **16 parameter slice slots**, but `std` uses an unbounded `Vec<u8>`; `MAX_OSC_RAW = 1024` / the const-generic capacity is only used **without std** ([parser][parser], [append/end][osc-buffer]). | This candidate always enables std. A small feed does **not** cap an unterminated OSC accumulated across calls. End/clear leaves Vec capacity. No public `Processor` API exposes/reset-limits just that OSC allocation. |
| DCS payload is passed bytewise to `Perform::put`; this ANSI performer's DCS hooks only log unhandled data. SOS/PM/APC string state ignores payload through `anywhere` ([DCS parser][dcs], [dispatch state][string-state], [termination][anywhere], [ANSI hooks][dcs-handler]). | Unlike OSC, these paths do not accumulate a payload vector here. They still consume arbitrarily long streams until termination; logging is another separately controlled resource. |
| Title/URI/clipboard construction is after parsing; titles and hyperlink strings have no length check here; clipboard decode allocates after OSC dispatch ([dispatch][osc-dispatch], [title][titles], [clipboard][clipboard]). | Handler-side truncation/denial is too late to bound parser accumulation or all temporary copies. A stateful input admission mechanism or parser extension is needed to enforce a control-string budget without silently desynchronizing authoritative state. No particular filter/patch is selected. |
| Title stack and **each** keyboard-mode stack cap at **4,096 entries** ([constants][minima], [keyboard][keyboard], [title stack][titles]). | Title entry byte size remains unbounded; finite entry count is not a byte ceiling. |
| ANSI sync buffer reserves **2,097,152 bytes** at processor construction; it flushes rather than extending when current length + input length reaches `SYNC_BUFFER_SIZE - 1` ([sync constants][sync-constants], [sync implementation][sync]). | Fixed requested capacity is separate from history. A tiny feed can trigger synchronous processing of previously buffered bytes. Actual allocation can exceed request. |
| Sync timeout is **150 ms**, extendable by a new begin-sync sequence ([sync constants][sync-constants], [sync][sync]). `StdSyncHandler::pending_timeout` only tests presence; the caller must schedule the deadline and call `stop_sync` (as [Alacritty's loop does][event-loop]). | This is presentation batching, **not** a parser execution deadline, watchdog or CPU kill. |
| `Processor::advance` takes an arbitrary slice, loops through it, and returns `()`; sync stop calls parser advancement synchronously ([sync][sync]). Lower-level parser has `advance_until_terminated`, but the ANSI processor owns its performer/termination policy ([parser][parser]). | Feed admission bounds new input, not elapsed time or cancellation inside a handler, buffered replay or resize. No recoverable resource-error channel from these signatures. |
| One CSI repeat dispatch can call `Handler::input` up to **65,535 times** ([repeat][repeat]); per input, scroll/insert/wrap can move or reset rows/cells ([input][input], [scroll][scroll]). Resize visits retained grids, builds temporary vectors and reflows before final truncation ([reflow][reflow]). | Small byte chunks are not equal-work chunks. With admitted state sizes some work is finite, but no wall-clock bound/interruptibility is proven; unlimited dimensions, OSC accumulation or overall stream duration remain unbounded. |

Separate admission/charge domains are needed for screen dimensions, history and reflow temporaries, control strings/parser state, input slice length, event payloads/queues and serializer buffers. Apply bounds **before** the allocation/work they claim to constrain. Pixel-area callbacks also multiply `u16` line/column and cell-size fields; admit/check those products rather than assuming a fitting field means a fitting area ([callback][pixel-query]). A callback can bound its own output queue but cannot retroactively prevent engine-created event strings. A rejected/failed authoritative-state transition must be explicit; silently resetting the parser/VT, dropping state-changing bytes or falling back to raw output would not preserve the agreed source of truth. This is a contract implication, not a selected failure mechanism.

Neither a worker thread nor `catch_unwind` establishes containment for abort, allocation failure or a parser/handler that does not return. Those risks are accepted context; no process/RSS/CPU containment or absence of other defects is claimed.

## Executed evidence versus source-only conclusions

Executed only a tiny, isolated Rust probe: local checkout of the exact candidate as path dependency, `default-features=false`, own manifest/lock/cache/target under the research worktree, no terminal control, no product manifests changed. `cargo run --quiet` succeeded after adding the explicit `Processor` type annotation required for inference. Resolved `vte=0.15.0`, `arrayvec=0.7.8`. Its assertions covered:

- Twelve bounded calls to `Cell::push_zerowidth` retain nine marks.
- A 4-column × 2-line VT fed `abcdefghij` has one history row and `WRAPLINE`; resize to 2 × 2 gives three history rows.
- `CSI 6 n` arrives as `Event::PtyWrite("\x1b[2;2R")`.
- Entering alternate has no history; returning normal restores three rows; `CSI 3 J` clears them.
- `Grid::<Cell>::new(1, 2, 100_001); initialize_all()` reports 100,001 history rows.
- Printed layouts: `Cell=24 CellExtra=56 Hyperlink=8 Row<Cell>=32`.

This demonstrates compilation/basic behavior, not all resize/wide-character/reset cases, a byte-budget implementation or native restoration. Inactive-grid/API absence, allocation/capacity limitations, parser bounds and work implications are **source-only** findings. No unbounded-resource probe, exploit or general security audit was run.

<details>
<summary>Exact bounded probe source and reproduction</summary>

Create an isolated edition-2021 Cargo package with `[workspace]` and dependency `alacritty_terminal = { path = "../alacritty/alacritty_terminal", default-features = false }`, with `../alacritty` checked out at the exact candidate. Run `cargo run --quiet`. Dependency resolution is as recorded above, not a promise that a future fresh Cargo resolution is identical. Source SHA-256: `1100c1a0086978c335f49352c12b522ae15e7cf7f23e4931a2d984130f794ced`.

```rust
use std::{cell::RefCell, mem::size_of, rc::Rc};
use alacritty_terminal::{
    event::{Event, EventListener},
    grid::{Dimensions, Grid, Row},
    index::{Column, Line},
    term::{cell::{Cell, CellExtra, Flags, Hyperlink}, Config, Term, TermMode},
    vte::ansi::Processor,
};

struct Size(usize, usize);
impl Dimensions for Size {
    fn total_lines(&self) -> usize { self.1 }
    fn screen_lines(&self) -> usize { self.1 }
    fn columns(&self) -> usize { self.0 }
}
#[derive(Clone)]
struct Listener(Rc<RefCell<Vec<String>>>);
impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        if let Event::PtyWrite(text) = event { self.0.borrow_mut().push(text); }
    }
}
fn main() {
    println!("Cell={} CellExtra={} Hyperlink={} Row<Cell>={}",
        size_of::<Cell>(), size_of::<CellExtra>(), size_of::<Hyperlink>(), size_of::<Row<Cell>>());
    let mut cell = Cell::default();
    for _ in 0..12 { cell.push_zerowidth('\u{301}'); }
    assert_eq!(cell.zerowidth().unwrap().len(), 9);
    let replies = Rc::new(RefCell::new(Vec::new()));
    let mut term = Term::new(Config { scrolling_history: 4, ..Config::default() },
        &Size(4, 2), Listener(replies.clone()));
    let mut parser: Processor = Processor::new();
    parser.advance(&mut term, b"abcdefghij");
    assert_eq!(term.grid().history_size(), 1);
    assert!(term.grid()[Line(-1)][Column(3)].flags.contains(Flags::WRAPLINE));
    term.resize(Size(2, 2));
    assert_eq!(term.grid().history_size(), 3);
    parser.advance(&mut term, b"\x1b[6n");
    assert_eq!(replies.borrow().last().unwrap(), "\x1b[2;2R");
    parser.advance(&mut term, b"\x1b[?1049h");
    assert!(term.mode().contains(TermMode::ALT_SCREEN));
    assert_eq!(term.grid().history_size(), 0);
    parser.advance(&mut term, b"\x1b[?1049l");
    assert_eq!(term.grid().history_size(), 3);
    parser.advance(&mut term, b"\x1b[3J");
    assert_eq!(term.grid().history_size(), 0);
    let mut grid = Grid::<Cell>::new(1, 2, 100_001);
    grid.initialize_all();
    assert_eq!(grid.history_size(), 100_001);
    println!("PASS: nine marks; wrap/reflow 1->3; query callback; alt return; clear; library history >100000");
}
```

Scratch was removed after recording results. No reusable automated product tests or safety qualification are claimed.
</details>

## Actionable handoff, not decisions

For [Daemon virtual terminal: engine, ownership, and failure boundaries](https://github.com/jackgeek/climon2/issues/43):

1. Select/record the **exact fixed development revision and dependency lock**, or a separately verified future release. Do not substitute published `0.26.0`.
2. Resolve inactive-state inspection, stable history/reflow lineage and allocation introspection: narrow upstream extension/maintained patch versus a proven semantic adapter. These are concrete gaps, not supplied by `renderable_content`.
3. Define conservative charged storage and cache/container attribution, associated compaction, and transient qualifications. **100,000 is application config, not an embedding-library cap.**
4. Choose accepted dimensions/rows, control-string and feed admission, event/serializer resource policies and explicit authoritative-state failure handling. Do not claim hard CPU/process containment.
5. Finish live rendering, advertised identity, callback query policy, hidden restore state and module interface. Nothing here resolves those choices or amends ADRs.

For the existing protocol/qualification work: establish logical delivered-suffix anchors through reflow/reset/eviction and ordered acknowledgement; native-host behavior still belongs to [Native scrollback qualification in Terminal.app, iTerm2, and VS Code](https://github.com/jackgeek/climon2/issues/52). No further ticket is necessary for these already-scoped gaps.

Action log: read agent/domain/ADR and issue contracts; claimed the research child first; created isolated worktree; retrieved public pinned engine/parser/registry/Rust sources; inspected relevant interfaces; ran bounded compile/behavior/layout checks; recorded findings and checked citations; committed/pushed only the research branch; published the result and updated the map. Only this Markdown artifact is intended for the commit.

## Pinned primary sources

[manifest]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/Cargo.toml#L1-L29
[fix]: https://github.com/alacritty/alacritty/commit/ede2ac144da4dec4c075bfa803aacf3b3739bce6
[cell]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/cell.rs#L13-L229
[registry]: https://github.com/rust-lang/crates.io-index/blob/95ac3c821589053cb5b45e286a3e79d077dbed09/al/ac/alacritty_terminal
[release-manifest]: https://github.com/alacritty/alacritty/blob/dc946285396bd3e251b6942af7e9961b74f76bee/alacritty_terminal/Cargo.toml#L1-L4
[lock]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/Cargo.lock#L2123-L2135
[grid-api]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L110-L161
[grid-cursor]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L32-L52
[dimensions]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L410-L535
[term-fields]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L268-L386
[term-access]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L635-L735
[render]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L2363-L2411
[wrap]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L942-L980
[linefeed]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1421-L1433
[queries]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1255-L1347
[reports]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L2046-L2151
[events]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/event.rs#L13-L110
[reflow]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/resize.rs#L12-L388
[damage]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L449-L495
[options]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L410-L447
[set-options]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L499-L526
[history-api]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L383-L408
[storage]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/storage.rs#L12-L143
[row]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/row.rs#L14-L151
[scroll]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/mod.rs#L175-L350
[clears]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1750-L1858
[handler]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs#L495-L834
[input]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1059-L1152
[app-limit]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty/src/config/scrolling.rs#L6-L53
[line-type]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/index.rs#L112-L140
[storage-index]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/grid/storage.rs#L218-L244
[minima]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L33-L48
[arc]: https://github.com/rust-lang/rust/blob/ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96/library/alloc/src/sync.rs#L388-L397
[vec]: https://github.com/rust-lang/rust/blob/ac68faa20c58cbccd01ee7208bf3b6e93a7d7f96/library/alloc/src/vec/mod.rs#L367-L398
[params]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/params.rs#L5-L25
[parser]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs#L44-L160
[numeric]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs#L490-L516
[osc-buffer]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs#L519-L559
[osc-dispatch]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs#L1329-L1422
[clipboard]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1702-L1747
[titles]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L2221-L2256
[keyboard]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L1287-L1305
[sync-constants]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs#L35-L48
[sync]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs#L240-L470
[event-loop]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/event_loop.rs#L227-L248
[repeat]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs#L1553-L1569
[dcs]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs#L316-L337
[string-state]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs#L167-L184
[anywhere]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs#L438-L450
[dcs-handler]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs#L1310-L1326
[pixel-query]: https://github.com/alacritty/alacritty/blob/bbb2cbf9470a6707b1205fbddbdb25109b14e2fd/alacritty_terminal/src/term/mod.rs#L2259-L2271
