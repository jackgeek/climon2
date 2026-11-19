# Daemon-owned virtual terminal

[Detach](../../GLOSSARY.md#attachment-and-control), reattach and takeover need the [session](../../GLOSSARY.md#sessions)'s screen, history, modes and title without asking any [host](../../GLOSSARY.md#sessions) terminal. The [session daemon](../../GLOSSARY.md#sessions) therefore owns one [virtual terminal](../../GLOSSARY.md#sessions) per session and is the sole authority for terminal state and terminal-query replies. Current [clients](../../GLOSSARY.md#sessions) keep no emulator and no transcript. The virtual terminal lives in the shipped library `climon2-vt` (ADR 0004): pure, platform-neutral, infallible for the daemon, and exposing only climon2-owned types. A monitor's copy is the allocation-bounded `Model` [below](#monitor-models-bounded-by-allocation-on-the-unchanged-engine).

## Engine: unchanged Alacritty 0.26.0

`climon2-vt` is the only crate that depends on `alacritty_terminal = "=0.26.0"` (default features off), the published crates.io release locked by checksum `bda177466b9524d59f1b12f0dd30b68696788e9992a7e959021c4a0ed96fcf59` (upstream commit `94e7c8874e526b1e67b349d9ba30ddf81669119e`, parser vte 0.15.0). It is unpatched and not a git dependency: an immutable, reproducible registry release, and the same engine the test harness already uses. The adapter uses only public API: grid and cursor access, `swap_alt`, `set_options`, `unset_private_mode`, `Processor` with a climon2 `Timeout` and `EventListener`, and, for a monitor model's own `CSI n b`, the engine's `Handler::input`.

**Implemented (#179; owner decision, 2026-11-13):** Alacritty starts, and restarts after RIS (`ESC c`), with alternate scroll (`CSI ? 1007`) on, while xterm starts with it off. The adapter resets the mode when it builds an engine and after every RIS, so the session daemon's virtual terminal and every monitor model report it only while the application has set it. Restorations send the mode from that state: `CSI ? 1007 l` until the application sets it, then `CSI ? 1007 h`. For ordinary attaches this is the one intended change to restoration bytes, which used to send `CSI ? 1007 h` although no application asked for it.

Evidence limits: earlier qualification ran against revision `bbb2cbf9470a6707b1205fbddbdb25109b14e2fd` (0.26.1-dev) and does not carry over. In 0.26.0 the combining marks of one cell are stored without a bound. Only the seams this choice touches are validated (front filter, sync timing, the alternate-screen swap, history accounting, serialization and derived rendering), including one cell carrying many zero-width marks. Alacritty 0.26.0 implements only `CSI ? 1049 h/l` for the alternate screen; modes 47 and 1047 are unknown to it.

## Monitor models: bounded by allocation on the unchanged engine

The [operational limits](../for-humans/behaviour/monitor/operational-limits.md#allocation-aware-terminal-prerequisite)
reopened the unchanged-engine and infallible-interface assumptions for the
monitor role: models must be bounded by allocation, including combining and
auxiliary cell data, resize and restoration staging, rejecting unsafe growth
before it happens, keeping the last safe page and never truncating accepted
characters. Row limits or a size check after allocation do not prove that.

**Outcome (#76): no counterexample on unchanged Alacritty 0.26.0, for the
tested streams.** The engine stays unpatched; only the monitor's interface is
fallible. `climon2-vt` exposes `Model`, one per session, created with
`Limits { bytes, history_rows }` and refusing with `Rejected { needed, limit }`.
The daemon's `Vt` and its behaviour are unchanged, apart from the
classification fix both share
([#152](https://github.com/jackgeek/climon2/issues/152)). The tests below
cover every growth path found and deterministic adversarial streams: they are
evidence that the bound holds for those streams, not a proof for every input.
Every hole found so far was in the adapter's admission, not the engine, so the
stop condition was not reached. The owner accepted the bound with the measured
reserve as #76's risk proof (2026-11-13), once history yielded before
rejection and one token could admit several history chunks. The engine facts
that shaped the design, all for 0.26.0 and vte 0.15.0:

- every `Processor` allocates vte's 2 MiB synchronized-update buffer, and
  remembers the character it printed last, which `CSI n b` repeats;
- one cell's combining marks live in a shared `CellExtra` with an unbounded
  character vector, and `CSI 65535 b` repeats the preceding character, a mark
  included, 65535 times from nine bytes;
- the title stack holds 4096 titles of up to 4096 bytes, and each OSC 8
  hyperlink is a new allocation;
- normal-screen history storage grows 1000 rows at a time, and rows stay
  allocated beyond a cleared history;
- resize reflow splits rows with `split_off`, which keeps the old capacity and
  allocates each remainder: shrinking full 500-column rows to 2 columns
  allocates about 62,000 cells per row (measured), and kept rows can retain
  their old capacity.

The model keeps to its limit this way:

- **Input.** Only what the live output policy forwards to a controller under
  [Direct presentation](../../GLOSSARY.md#restoration-and-presentation)
  reaches the engine, classified by the sequence vte dispatches. The daemon
  already applied that policy, so a stripped token can only be malformed
  input; only the C0 controls vte executes in it reach the engine. Titles, the
  title stack, hyperlinks, clipboard, DCS and queries are therefore stripped
  before a model's engine, whether or not controls sit before their
  introducer; tests feed both forms to a model with no history at its
  mandatory limit.
- **One parser per monitor (#166).** Every model keeps its own engine, but
  one `Parser` (vte's `Processor`) serves all of a monitor's models, so its
  2 MiB buffer (`Parser::BYTES`) is counted once by the monitor, not per
  model. The parser holds nothing from one model when another is fed: each
  model feeds it only complete tokens, a UTF-8 sequence cut off at the end
  of a text token by the escape that follows prints its replacement
  character within that token, and a model expands `CSI n b` itself: the
  controls vte executes in the sequence go through the parser, then the
  model prints its own preceding character `n` times through the engine's
  `Handler::input`, as vte would. A model tracks that character from vte's
  rules for printing text (every character but the C0 and C1 controls, one
  replacement character per invalid sequence except a lone byte up to 0x9f).
- **Fixed charge.** The engine's allocation for its geometry and a
  32 KiB reserve for what the front filter and the engine hold briefly: a
  partial token and the tokens one slice completes (at most about 15 KiB
  measured, for a 4096-byte control string held across one-byte reads; an
  abandoned sequence's held controls are released as one token), the cursor
  templates' cell data, and the engine's new configuration and tab stops
  while it replaces them.
- **Admission.** Output is admitted one token at a time: the text of one
  64-byte slice (with at most three bytes of an unfinished UTF-8 sequence
  held from the slice before), the C0 controls an abandoned sequence held
  (up to 4096 bytes, released as one token), or one control sequence. Before
  the engine sees a token the model charges its worst-case growth, from the
  live output policy's own vte parse of that token: 72 bytes per combining
  mark (new extra data and the vector's first four characters); 56 bytes per
  cell printed while the cursor template carries extra data, such as an
  underline colour, and three cells per wide character; for `CSI n b`, the
  count vte parses, wherever controls sit in the sequence, times the worst
  repeated character. When the running charge leaves no room, the model
  measures what the active screen's cells hold (56 bytes per cell with extra
  data and 16 per mark, which covers the doubling vector and its copy) and
  decides again; the inactive screen's charge stands while the engine cannot
  change it.
- **History.** The engine's history limit is whole 1000-row chunks, so its
  storage never outgrows the limit. The first chunk, which holds the
  protected newest page, is admitted lazily (#166): only before the first
  token that could push a normal-screen row into history, and that token is
  rejected, keeping the page, if the chunk does not fit once history beside a
  staged restoration has yielded. The protected page is therefore an
  obligation at the first scroll, not at creation: a model that never
  scrolled holds no history storage and is charged none. Because admission
  must still precede the allocation, the model judges each token before the
  engine sees it, from the cursor, the scroll region and the token's vte
  parse. The engine keeps the region private, so the model mirrors it from
  DECSTBM (as the engine applies it: ignored unless top is above bottom,
  clamped to the screen), `ESC c` and DECCOLM (`CSI ? 3 h` or `l`, which
  the engine applies as a region reset), the only ways a model's engine
  changes it. Only a region that starts at the top
  row pushes rows into history. A token could scroll if its line feeds
  (including index, next line and those executed inside a sequence) and
  repeated characters could leave that region's bottom row, or it is
  `CSI S` in such a region, `CSI M` on the top row, or `CSI 2 J`, which
  scrolls the normal screen's rows into history whatever the region. While
  the chunk waits, text is fed in pieces that cannot reach the bottom row
  (each character or invalid sequence moves the cursor down at most one row),
  then a character at a time on it, where only a line feed or a character in
  the last column could scroll. The judgement is an upper bound: the chunk
  may be admitted a token early (a character in the last column that does
  not wrap, an erased screen that was already blank), never late. Ahead of any token that could fill the current
  limit, every chunk it could fill is admitted, before it is allocated, while
  the next one fits (a token pushes at most a row per byte and per
  repetition, or a screenful, so one long repeat or released run of line
  feeds can fill several). Beyond the chunks that fit, the engine discards
  its oldest rows instead, and the update is still accepted; the history view
  reports the truncation ([below](#read-only-views-for-painting)). Storage
  left after a token clears history is
  dropped at once, so measuring sees every mark still held. The model shows
  at most `history_rows`.
- **History yields before anything is rejected.** History is discarded
  truthfully as it exceeds the allocation ceiling, never required terminal
  updates ([operational limits](../for-humans/behaviour/monitor/operational-limits.md#memory-history-and-admission)).
  When a token's growth does not fit even after measuring, the oldest history
  chunk is discarded, a chunk at a time and never the first: the engine's
  limit is lowered, the rows it drops and their cell data are released at
  once, and the normal screen is measured again, whichever screen is active.
  Beside a staged restoration the current model's history yields first, to
  the staged model's history chunks as well as its cells, because applying
  the restoration replaces it whole; then the staged model's own. A chunk
  admitted ahead of output but not yet filled yields without losing a row.
  The row vector keeps the capacity its largest limit took, so that capacity
  stays charged. `Model::shorten_history()` discards a chunk the same way for
  the monitor's pressure policy across sessions (#119).
- **Rejection.** The first token that does not fit once history has yielded
  all it can is rejected before the engine sees it, and so is the rest of the
  update. The model keeps the page its accepted tokens produced, with every
  accepted character, and refuses output until a staged restoration is
  applied.
- **Staging and geometry.** A restoration is fed into a separate staged model
  charged beside the current one; applying it replaces the current model and
  history whole, and a rejected or discarded restoration never becomes current.
  Staging is admitted once both models fit, the current model's oldest history
  having yielded first. A model's geometry is fixed: a new effective geometry
  arrives as a restoration staged at that geometry. Models never reflow in
  place. **Decided (owner, 2026-11-13, #76):** the wire is to be amended so
  that a monitor-mode geometry change is always followed by a Restoration
  begin at that geometry before any Display at it, and, as a defence, a
  monitor model marks itself stale if a Display arrives at a geometry it does
  not have. Neither is made by #76: the agreed wire still says only that a
  restoration may follow a resize
  ([Restoration begin](../for-humans/protocol/daemon-client/event-restoration-begin.md)),
  and a [Geometry](../for-humans/protocol/daemon-client/event-geometry.md) event carries
  none. [#89](https://github.com/jackgeek/climon2/issues/89) amends those
  pages and tests the order; [#108](https://github.com/jackgeek/climon2/issues/108)
  adds the stale defence.

Evidence: `crates/climon2-vt/tests/bound.rs` (LIM-03) counts each test thread's
allocations with a global allocator. It shows the following:

- no tested stream takes a model past its limit;
- what an update keeps is no more than it was charged, so the reserve's unused
  room cannot hide an undercharge;
- plain text, invalid bytes included, fills cells the engine already holds
  and is accepted without any charge;
- wide and combining characters, underline colours, `CSI b` with controls
  anywhere in it, staging and resize are accepted exactly at the bound and
  rejected one byte past it, with the page and accepted characters intact;
- history yields instead: live output, the alternate screen, staging and a
  staged restoration keep every history row at the bound and discard the
  oldest chunk one byte below it, never the first chunk (worked cases: 6,000
  marked rows at 200x60 within 64 MiB; staging at 500x200 after 3,100 rows);
- one token admits every history chunk it could fill;
- a model keeping history is admitted with the same bytes as one keeping
  none and holds no history storage until its first scroll; the first chunk
  is admitted on that scroll (23 line feeds from the top of a 24-row screen
  admit nothing, the 24th admits it), at the bound and rejected one byte
  below it with the page kept, following the scroll region and every
  scrolling sequence listed above (#166);
- a parser allocates exactly its 2 MiB buffer, models sharing one hold no
  more than their footprints, and `tests/parser.rs` shows each model
  receiving only its own tokens and repeating only its own preceding
  character, at the repeat count's boundaries (#166);
- deterministic adversarial streams never pass the limit. They slip the
  tokenizer tests' tricky bytes among text, marks, repeats, tints, scrolls,
  screen switches and control strings, read in pieces of any size;
- the reserve covers the largest transients with no history;
- measuring reclaims released marks.

Limits: the bound is tracked allocation capacity, not resident memory or
allocator overhead. Charges are upper bounds, so a model may reject, or stop
growing history, before memory is strictly exhausted; optional history growth
uses the running charge and never measures. Which history yields under
pressure across sessions is monitor policy (#119), through
`Model::shorten_history()`.

**Per-model cost.** Measured on #76, a model's mandatory charge was vte's
2 MiB buffer, the reserve, both screens and, when it kept any history, the
first 1000-row chunk; the default `memory.total_bytes` (512 MiB) held 126,
70 and 28 models with history at 80x24, 200x60 and 500x200, or 241, 198 and
77 without, while PERF-01 asks for 100 sessions at each geometry. The owner
decided (2026-11-13) to share one vte parser per monitor and admit the first
history chunk only when the normal screen first scrolls, then re-measure
([#166](https://github.com/jackgeek/climon2/issues/166)). Re-measured on
#166 (`Model::footprint()`, whose charges the allocator tests above bound;
`crates/climon2-vt/tests/budget.rs` asserts this table and prints it anew
when a charge changes), one parser of 2 MiB per monitor plus, per model:

| Geometry | Before its first scroll | After it (first chunk) | A scrolled restoration staged beside it | 10,000 history rows |
|---|---:|---:|---:|---:|
| 80x24 | 0.12 MiB | 2.05 MiB | 4.09 MiB | 19.35 MiB |
| 200x60 | 0.59 MiB | 5.26 MiB | 10.52 MiB | 47.28 MiB |
| 500x200 | 4.63 MiB | 16.18 MiB | 32.36 MiB | 120.00 MiB |

A restoration staged after a geometry change carries history, so it scrolls
and admits its own first chunk: staging a model that has scrolled costs
twice what it holds after its first scroll, until the restoration is applied
or the current model's history yields.

A model keeping no history costs what the first column says however much it
scrolls. With the parser counted once, the then default 512 MiB held 4,206,
870 and 110 models that had not scrolled (or kept no history), and 249, 96
and 31 that held their first history chunk: once their screens had scrolled,
200x60 fell short of PERF-01's 100 sessions by 4 and 500x200 by 69.

**Decided (owner, 2026-11-15, #166).** The default `memory.total_bytes` is
raised from 512 MiB to 576 MiB, PERF-01's 100 sessions are scoped to
geometries up to 200x60, and at 500x200 PERF-01 records the measured
capacity instead. At 576 MiB, beside one parser, a monitor holds:

| Geometry | Models before their first scroll | Models holding their first chunk |
|---|---:|---:|
| 80x24 | 4,734 | 280 |
| 200x60 | 979 | 109 |
| 500x200 | 124 | 35 |

These counts cover the models alone, before staging, history beyond the
first chunk or any other monitor budget; `tests/budget.rs` asserts them.

### Read-only views for painting

**Implemented (#83):** `Model::view()` lends the monitor climon2-owned, read-only
views of the current model for one frame: each cell's character with its
combining marks and width, its colours and rendition; the cursor; the active
screen, normal or 1049 alternate; the modes the input router and painter
honour; and the normal screen's history with its truncation state. They are
Rust types, not a cross-release cell schema. A test in
`xtask/tests/vt_interface.rs` parses the crate's sources and fails if a
public item or a trait impl of a public type names an engine type, by path,
through a `use` or re-export, or through a crate type alias; items a macro
generates are not checked.

The model records truncation until the history is cleared: rows beyond
`history_rows`, and rows that yielding or `Model::shorten_history()` drops,
are known exactly, and output the admitted chunks had room for loses no row,
even when it fills them exactly. Once `history_rows` caps the history's
growth or the bound refuses it, though, the engine replaces the oldest row of
a full history for each row pushed without reporting it. **Decided (owner,
2026-11-13, #83):** such a history reports truncation as soon as output that
could push a row out leaves it full, even before a row is lost. While the
first chunk awaits the first scroll (#166), no token pushes a row, since one
that could have admitted the chunk, so nothing is truncated then. The flag
keeps that meaning. **Implemented
([#112](https://github.com/jackgeek/climon2/issues/112)):** the anchored
viewport (TRM-05) counts the rows each engine step pushes exactly, with no
counting headroom like the daemon's
([below](#restoration-three-phases-produced-incrementally)): only while a
view is scrolled back, the step starts the engine's own display offset one
row back, and Alacritty moves it back by each row it pushes into history
(for a scroll region at the top row; one below it moves the offset without
pushing, so it is not counted). The count stops one row short of the
engine's history limit, so a single step pushing a whole chunk can leave a
view on the oldest row instead of the notice above it. The offset is never
shown.

## Live output policy and query authority

A front filter that mirrors vte 0.15.0's state machine splits PTY output into complete tokens (text never split inside a UTF-8 sequence; partial tokens held across reads up to a 4096-byte body, oversized tokens dropped from engine and output). Every token updates the engine; one policy decides what reaches the controller's host:

- supported queries (DA1/DA2/DECID, DSR 5/6, DECRQM, `CSI 18 t`) are answered by the engine's replies to the PTY and stripped;
- every other host-dependent request, keyboard-protocol negotiation, OSC 52, OSC 8, every other OSC and every DCS/SOS/PM/APC is stripped; the host is never asked anything in-band;
- titles are tracked by value and re-emitted as a safely encoded `OSC 2`;
- `CSI ? 47/1047 h/l` parameters are removed, because the engine does not implement them and the host must not diverge from it;
- `CSI ? 9001 h/l` (win32-input-mode) parameters are removed on every platform (agreed, not implemented; [#211](https://github.com/jackgeek/climon2/issues/211)). ConPTY requests the mode at startup, and input stays plain VT, so no host may switch to it. ConPTY's DA1 is answered like any other query, and its `CSI ? 1004 h/l` is ordinary output;
- everything else is forwarded unchanged.

A token is classified by the sequence vte dispatches for it. Between `ESC` and the introducer, vte executes C0 controls without leaving its escape state, so they never change a token's kind ([#152](https://github.com/jackgeek/climon2/issues/152)); CAN and SUB, however, cancel the sequence, and ESC restarts it. The policy reads each escape sequence, CSI sequence and control string with vte once. When it strips a token or removes screen modes from it, the C0 controls vte executed in that token still reach the host, in order.

The engine is fed in steps of at most 64 bytes of text and C0 controls; every escape sequence is its own step(s).

### Agreed terminal-status extension

The [session-status revision](../for-humans/behaviour/session-status.md) adds daemon-core observation
of OSC 7501, OSC 133, and OSC 9/99/777 before host-output filtering. The daemon
answers OSC 7501 capability queries locally. Root program reports and
shell lifecycle select explicit or heuristic activity detection; recognized
notifications request attention, while progress alone does not.

Raw sequences remain stripped from host output. Clients/monitors instead
receive structured notification/progress metadata, including while background
or noncontrolling. This extends local interpretation and daemon-owned replies,
not the permission to send host-dependent requests to an outer terminal.
The [agreed daemon wire](../for-humans/protocol/daemon-client/README.md) now specifies
structured schemas and source normalization;
[native declarations](../for-humans/protocol/monitor-client/README.md) are also agreed.
The session daemon implements this extension
([#92](https://github.com/jackgeek/climon2/issues/92),
[#98](https://github.com/jackgeek/climon2/issues/98)); the filter and query
implementation above are not evidence for it.

The [monitor](../../GLOSSARY.md#sessions)'s per-session `climon2-vt` copy for retained disconnected
inspection (#108, #116) is an explicit exception to the current no-client-emulator rule,
not a competing terminal-query authority. Forwarding to other monitors and
xterm.js browsers is a future consumer boundary, not permission for revoked
core screen mirroring or a native library in a browser. The [agreed #63 terminal contract](../for-humans/protocol/terminal-integration.md) now selects
ordered exact daemon terminal bytes, reparsed into monitor-owned `climon2-vt`
models and painted by Ratatui. This deliberately reopens the rejected
client-side-emulator option for the monitor role, not ordinary clients or query
authority. The [read-only views](#read-only-views-for-painting) keep engine
types private; the monitor, the one other consumer allowed by ADR 0004, paints
its selected session from them (#108) and uses the crate for the documented
normal-shell baseline and, at startup, a model's mandatory allocation and
the shared parser's size, to check that its limits hold one session's model
beside the parser (#167, #166).
No native cross-release cell schema or new workspace crate is selected.

Monitor-mode display/restoration uses effective session geometry canonically,
not host/requested dimensions, including passive promotion and background grants,
because host-coordinate restoration cannot safely be reparsed at a smaller
effective size. The session daemon implements this
[canonical presentation](#canonical-monitor-presentation)
([#89](https://github.com/jackgeek/climon2/issues/89)); ordinary direct/derived
attach, native history and release below are unchanged.
The monitor uses its own alternate screen and internal normal history, replaces
whole models/history transactionally, and never writes session bytes directly
to the host or emits duplicate query replies. Selected input also waits for
coherent paint, while applied background models can report Ready without paint.
The later [lifecycle amendment](../for-humans/behaviour/monitor/lifecycle.md) limits stale inspection
to disconnected/final retained history: connected revoked clients show only
the placeholder, with no revoked-cache inspection route. Disconnected history
overlays exactly `Reconnecting...` while passive recovery is scheduled or active;
permanent rejection instead shows a disconnected/error overlay. Recovery removes
the overlay but requires explicit acquisition for a non-controller's live
presentation. No history/retry binding is added. Selector removal discards cache
only after definitive termination and local instance cleanup/join, for this
monitor run only. The monitor implements these behaviors (#108, #112, #116,
#117), which leave daemon query authority and ordinary presentation unchanged.

## Restoration: three phases, produced incrementally

Every grant sends a restoration prefix, then live output. Phase A (prologue reset, preservation of visible host content into native scrollback, coarse truncation notice) is computed from this attachment's presentation. Phase B sends retained history rows oldest first, produced incrementally as the socket drains. Phase C (screen suffix: scroll-off, normal screen and cursor, alternate screen, modes, title) is captured at the snapshot point S. Raw history is never replayed, so historical side effects never repeat; native duplicates on reattach are accepted.

History is a **moving view**: phase C describes S exactly, while phase B sends a snapshot row only if it is still retained when the producer reaches it, marks skipped rows with the notice, and ends early if the application cleared history. Alacritty does not report how many rows a step pushed, and a push at capacity silently replaces the oldest row, so the adapter counts pushes with **counting headroom**: the engine's capacity is `max_rows + 2P`, where `P = max(64, rows, ceil(2 * 65535 / cols) + 1)` bounds the rows one step can push. Headroom rows are never retained history. They cost at most about 262,000 cells at 2 columns and 200,000 cells at 500x200. The eviction flag is content-independent.

The normal screen is read while the application is on the alternate screen by moving the alternate grid aside (a two-cell placeholder takes its place for the swap back to reset), swapping, reading, swapping back and moving the grid home, so no screen is copied. This sets the normal saved cursor to the normal current cursor (accepted exception E1).

## Direct and Derived presentation

Session geometry is the controller's host size clamped to 2..=500 columns and 1..=200 rows. A host of exactly that size gets **Direct** presentation: filtered tokens forwarded unchanged. Absolute positioning in a restoration fixes only the initial image; a larger host would then wrap and scroll unchanged live bytes at its own width and bottom row, not the session's, so it gets **Derived** presentation: live output is rendered from engine state inside the session rectangle, pushed history rows enter native scrollback, and only rows that changed are repainted. Hosts narrower than 2 columns (Alacritty's minimum) are unsupported, and the scroll region is not carried across a switch between Direct and Derived.

## Canonical monitor presentation

A [monitor](../../GLOSSARY.md#sessions) parses a session's bytes into its own model of exactly the effective session geometry and paints that model; its host is never the session's terminal. Every restoration and live byte the daemon sends a monitor-mode attachment, a [passive monitor connection](../../GLOSSARY.md#attachment-and-control) that its Acquire or a [promotion](../../GLOSSARY.md#attachment-and-control) made an attachment, therefore has [canonical presentation](../../GLOSSARY.md#restoration-and-presentation): it is serialized at the session geometry for a fresh terminal of exactly that size, whatever size the monitor reports, and the monitor parses each restoration into a fresh model.

- **Restoration.** Phase A is only the prologue, whose scroll-region reset homes the cursor, and any truncation notice: there is no host content to preserve. Phase B prints the retained history from the top of the fresh screen, and C1 scrolls off exactly the lines printed, a wrapped notice counting as the rows it wraps onto, so the model's history holds exactly the retained rows, with no blank row added. Phase C is the Direct one. `Vt::canonical` takes the snapshot and gives the attachment a fresh Direct presentation record of the session geometry.
- **Live output** is Direct: the filtered tokens apply to a model of the session geometry exactly as to the daemon's engine. Synchronized-update ends and the presentation release work as for any attachment.
- **Geometry.** Every grant of a monitor-mode attachment keeps the session geometry, so acquiring or inheriting control resizes nothing. Only its own Resize while it controls changes the geometry. A model is never reflowed in place: every change of the effective geometry is followed by a restoration at the new geometry before any live byte at it. Once the attachment is ready, Geometry reports the change and a replacement restoration within the period follows at once; before readiness the replacement alone reports it. A host change that keeps the geometry changes nothing.

Considered (developer decision on [#76](https://github.com/jackgeek/climon2/issues/76), 2026-11-13): resizing a model in place was rejected, because reflowing a large history can allocate far beyond the model's bound (about 62,000 cells per row on a 500-to-2-column shrink) and needs a staged copy anyway. A monitor model that receives a Display at a geometry it does not have marks itself stale until the next restoration, as a defence only ([#108](https://github.com/jackgeek/climon2/issues/108)).

## Presentation release

The daemon keeps a per-attachment presentation record of what that attachment's committed bytes put its host into (alternate screen, open synchronized update, normal-screen cursor, Derived cache). Prologues and the release that ends a presentation are computed from it, never from the session's engine state: the release returns the host to a documented normal-shell baseline and leaves the alternate screen only if this attachment entered it.

Release then re-establishes the committed normal-screen cursor by absolute
position, because resetting scroll margins or origin mode can home it. A
restoration captures that cursor at its snapshot point; Direct commits record it
from the corresponding engine output, while Derived updates retain the last
painted normal-screen cursor. Engine advancement alone cannot move a released
host's cursor. Cursor save/restore sequences are not used: restoring saved
attributes or origin mode would undo the normal-shell baseline.

## Zero-buffer synchronized output

The engine never buffers a synchronized update: its `Timeout` reports no pending timeout. The adapter instead latches the first `CSI ? 2026 h` with a 150 ms deadline that repeated begins do not extend; snapshots and derived updates wait for an explicit end or that deadline, and the daemon ends any update it left open on the host with a synthetic `CSI ? 2026 l`.

## Considered options

- **A git or patched Alacritty revision:** rejected; a checksum-locked registry release is reproducible without fetching from GitHub.
- **A second parser, a client-side emulator or a speculative engine trait:** rejected; one engine behind one adapter.
- **Replaying a raw transcript:** rejected; it repeats queries, clipboard writes and clears and cannot bound memory.
- **Raising history capacity as private snapshot storage:** rejected; `set_options` changes the real grid's history and drops rows.
- **The engine's own synchronized-update buffering:** rejected; it re-arms on repeated begins and flushes its whole buffer in one call.
- **Capping combining marks per cell, or only rows, in monitor models:** rejected; a cap truncates accepted characters, and rows bound neither marks nor storage chunks.
- **Reflowing monitor models in place on resize:** rejected; reflow's allocation grows quadratically with the column ratio and keeps old row capacities. A restoration follows every monitor-mode geometry change instead (decided above).
- **A patched or replaced engine for monitor models:** not chosen; admitting each token's worst-case growth before the unchanged engine sees it has found no counterexample (see the outcome above). Reopen this if one is found that admission cannot charge.
