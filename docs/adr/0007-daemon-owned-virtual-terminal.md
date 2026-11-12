# Daemon-owned virtual terminal

[Detach](../../GLOSSARY.md#attachment-and-control), reattach and takeover need the [session](../../GLOSSARY.md#sessions)'s screen, history, modes and title without asking any [host](../../GLOSSARY.md#sessions) terminal. The [session daemon](../../GLOSSARY.md#sessions) therefore owns one [virtual terminal](../../GLOSSARY.md#sessions) per session and is the sole authority for terminal state and terminal-query replies. Current [clients](../../GLOSSARY.md#sessions) keep no emulator and no transcript. The virtual terminal lives in the shipped library `climon2-vt` (ADR 0004): pure, platform-neutral, infallible for the daemon, and exposing only climon2-owned types. A monitor's copy is the allocation-bounded `Model` [below](#monitor-models-bounded-by-allocation-on-the-unchanged-engine).

## Engine: unchanged Alacritty 0.26.0

`climon2-vt` is the only crate that depends on `alacritty_terminal = "=0.26.0"` (default features off), the published crates.io release locked by checksum `bda177466b9524d59f1b12f0dd30b68696788e9992a7e959021c4a0ed96fcf59` (upstream commit `94e7c8874e526b1e67b349d9ba30ddf81669119e`, parser vte 0.15.0). It is unpatched and not a git dependency: an immutable, reproducible registry release, and the same engine the test harness already uses. The adapter uses only public API: grid and cursor access, `swap_alt`, `set_options`, and `Processor` with a climon2 `Timeout` and `EventListener`.

Evidence limits: earlier qualification ran against revision `bbb2cbf9470a6707b1205fbddbdb25109b14e2fd` (0.26.1-dev) and does not carry over. In 0.26.0 the combining marks of one cell are stored without a bound. Only the seams this choice touches are validated (front filter, sync timing, the alternate-screen swap, history accounting, serialization and derived rendering), including one cell carrying many zero-width marks. Alacritty 0.26.0 implements only `CSI ? 1049 h/l` for the alternate screen; modes 47 and 1047 are unknown to it.

## Monitor models: bounded by allocation on the unchanged engine

The [operational limits](../monitor-operational-limits.md#allocation-aware-terminal-prerequisite)
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
stop condition was not reached. The engine facts that shaped the design, all
for 0.26.0 and vte 0.15.0:

- every `Processor` allocates vte's 2 MiB synchronized-update buffer;
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

- **Input.** Only what the live output policy forwards to a Direct host reaches
  the engine, classified by the sequence vte dispatches. The daemon already
  applied that policy, so a stripped token can only be malformed input; only
  the C0 controls vte executes in it reach the engine. Titles, the title
  stack, hyperlinks, clipboard, DCS and queries are therefore stripped before
  a model's engine, whether or not controls sit before their introducer;
  tests feed both forms to a model with no history at its mandatory limit.
- **Fixed charge.** The engine's allocation for its geometry, vte's buffer and a
  32 KiB reserve for what the front filter and the engine hold briefly: a
  partial token and the tokens one slice completes (at most about 15 KiB
  measured, for a 4096-byte control string held across one-byte reads; an
  abandoned sequence's held controls are released as one token), the cursor
  templates' cell data, and the engine's new configuration and tab stops
  while it replaces them.
- **Admission.** Output is admitted one token at a time, a token being at most
  one 64-byte slice of text or one control sequence. Before the engine sees a
  token the model charges its worst-case growth, from the live output policy's
  own vte parse of that token: 72 bytes per combining mark (new extra data and
  the vector's first four characters); 56 bytes per cell printed while the
  cursor template carries extra data, such as an underline colour, and three
  cells per wide character; for `CSI n b`, the count vte parses, wherever
  controls sit in the sequence, times the worst repeated character. When the
  running charge leaves no room, the model measures what the active screen's
  cells hold (56 bytes per cell with extra data and 16 per mark, which covers
  the doubling vector and its copy) and decides again; the inactive screen's
  charge stands while the engine cannot change it.
- **History.** The engine's history limit is whole 1000-row chunks, so its
  storage never outgrows the limit; the first chunk, which holds the protected
  newest page, is mandatory. The next chunk is admitted, before it is
  allocated, ahead of any token that could fill the current one (a token
  pushes at most a row per byte and per repetition, or a screenful). When the
  chunk does not fit, the engine discards its oldest rows instead, and the
  update is still accepted; no truncation state reports this yet. Storage
  left after a cleared history is dropped at once, so measuring sees every
  mark still held. The model shows at most `history_rows`.
- **Rejection.** The first token that does not fit is rejected before the
  engine sees it, and so is the rest of the update. The model keeps the page its
  accepted tokens produced, with every accepted character, and refuses output
  until a staged restoration is applied.
- **Staging and geometry.** A restoration is fed into a separate staged model
  charged beside the current one; applying it replaces the current model and
  history whole, and a rejected or discarded restoration never becomes current.
  A model's geometry is fixed: a new effective geometry arrives as a restoration
  staged at that geometry, admitted only if both models fit. Models never
  reflow in place. **Open:** that needs a restoration after every effective
  geometry change, but the agreed wire says only that one may follow a resize
  ([Restoration begin](../protocol/daemon-client/event-restoration-begin.md)),
  and a [Geometry](../protocol/daemon-client/event-geometry.md) event carries
  none. How a model handles a Geometry event without a restoration awaits the
  developer's decision.

Evidence: `crates/climon2-vt/tests/bound.rs` (LIM-03) counts each test thread's
allocations with a global allocator. It shows the following:

- no tested stream takes a model past its limit;
- what an update keeps is no more than it was charged, so the reserve's unused
  room cannot hide an undercharge;
- plain text, wide and combining characters, underline colours, `CSI b` with
  controls anywhere in it, staging and resize are accepted exactly at the
  bound and rejected one byte past it, with the page and accepted characters
  intact;
- deterministic adversarial streams never pass the limit. They slip the
  tokenizer tests' tricky bytes among text, marks, repeats, tints, scrolls,
  screen switches and control strings, read in pieces of any size;
- the reserve covers the largest transients with no history;
- measuring reclaims released marks.

Limits: the bound is tracked allocation capacity, not resident memory or
allocator overhead. Charges are upper bounds, so a model may reject, or stop
growing history, before memory is strictly exhausted; optional history growth
uses the running charge and never measures. Which history yields under
pressure across sessions is monitor policy (#119).

**Open: per-model cost (measured on #76).** A model's mandatory charge is
vte's 2 MiB buffer, the reserve, both screens and, when it keeps any history,
the first 1000-row chunk:

| Geometry | No history | With history | Staged beside the current model | 10,000 history rows |
|---|---:|---:|---:|---:|
| 80x24 | 2.12 MiB | 4.05 MiB | 8.09 MiB | 21.35 MiB |
| 200x60 | 2.59 MiB | 7.26 MiB | 14.52 MiB | 49.28 MiB |
| 500x200 | 6.63 MiB | 18.18 MiB | 36.36 MiB | 122.00 MiB |

The default `memory.total_bytes` (512 MiB) therefore holds 126, 70 and 28
models with history, or 241, 198 and 77 without, while PERF-01 asks for 100
sessions at each geometry. Whether to change the design or the budgets awaits
the developer's decision.

## Live output policy and query authority

A front filter that mirrors vte 0.15.0's state machine splits PTY output into complete tokens (text never split inside a UTF-8 sequence; partial tokens held across reads up to a 4096-byte body, oversized tokens dropped from engine and output). Every token updates the engine; one policy decides what reaches the controller's host:

- supported queries (DA1/DA2/DECID, DSR 5/6, DECRQM, `CSI 18 t`) are answered by the engine's replies to the PTY and stripped;
- every other host-dependent request, keyboard-protocol negotiation, OSC 52, OSC 8, every other OSC and every DCS/SOS/PM/APC is stripped; the host is never asked anything in-band;
- titles are tracked by value and re-emitted as a safely encoded `OSC 2`;
- `CSI ? 47/1047 h/l` parameters are removed, because the engine does not implement them and the host must not diverge from it;
- everything else is forwarded unchanged.

A token is classified by the sequence vte dispatches for it. vte executes a C0 control between `ESC` and the introducer without leaving its escape state, so such controls never change a token's kind ([#152](https://github.com/jackgeek/climon2/issues/152)). The policy reads each escape sequence, CSI sequence and control string with vte once. When it strips a token or removes screen modes from it, the C0 controls vte executed in that token still reach the host, in order.

The engine is fed in steps of at most 64 bytes of text and C0 controls; every escape sequence is its own step(s).

### Agreed terminal-status extension, not implemented

The [session-status revision](../session-status.md) adds daemon-core observation
of OSC 7501, OSC 133, and OSC 9/99/777 before host-output filtering. The daemon
will answer OSC 7501 capability queries locally. Root program reports and
shell lifecycle select explicit or heuristic activity detection; recognized
notifications request attention, while progress alone does not.

Raw sequences remain stripped from host output. Clients/monitors instead
receive structured notification/progress metadata, including while background
or noncontrolling. This extends local interpretation and daemon-owned replies,
not the permission to send host-dependent requests to an outer terminal.
The [agreed daemon wire](../protocol/daemon-client/core-v1.md) now specifies
structured schemas and source normalization;
[native declarations](../protocol/monitor-client/core-v1.md) are also agreed.
The current filter and query
implementation above are not evidence that these planned signals are supported.

The [monitor](../../GLOSSARY.md#sessions)'s planned per-session `climon2-vt` copy for retained disconnected
inspection is an explicit exception to the current no-client-emulator rule,
not a competing terminal-query authority. Forwarding to other monitors and
xterm.js browsers is a future consumer boundary, not permission for revoked
core screen mirroring or a native library in a browser. The [agreed #63 terminal contract](../protocol/terminal-integration.md) now selects
ordered exact daemon terminal bytes, reparsed into monitor-owned `climon2-vt`
models and painted by Ratatui. This deliberately reopens the rejected
client-side-emulator option for the monitor role, not ordinary clients or query
authority. Planned read-only Rust cell/mode/cursor/history views keep engine
types private; current `climon2-vt` has no public cells and remains daemon-only.
No native cross-release cell schema or new workspace crate is selected.

Monitor-mode display/restoration will use effective session geometry canonically,
not host/requested dimensions, including passive promotion and background grants.
Current host-coordinate restoration cannot safely be reparsed at a smaller
effective size. This is an agreed unimplemented presentation-policy amendment;
ordinary direct/derived attach, native history and release below are unchanged.
The monitor uses its own alternate screen and internal normal history, replaces
whole models/history transactionally, and never writes session bytes directly
to the host or emits duplicate query replies. Selected input also waits for
coherent paint, while applied background models can report Ready without paint.
The later [lifecycle amendment](../monitor-lifecycle.md) limits stale inspection
to disconnected/final retained history: connected revoked clients show only
the placeholder, with no revoked-cache inspection route. Disconnected history
overlays exactly `Reconnecting...` while passive recovery is scheduled or active;
permanent rejection instead shows a disconnected/error overlay. Recovery removes
the overlay but requires explicit acquisition for a non-controller's live
presentation. No history/retry binding is added. Selector removal discards cache
only after definitive termination and local instance cleanup/join, for this
monitor run only. All are unimplemented and leave daemon query authority and
ordinary presentation unchanged.

## Restoration: three phases, produced incrementally

Every grant sends a restoration prefix, then live output. Phase A (prologue reset, preservation of visible host content into native scrollback, coarse truncation notice) is computed from this attachment's presentation. Phase B sends retained history rows oldest first, produced incrementally as the socket drains. Phase C (screen suffix: scroll-off, normal screen and cursor, alternate screen, modes, title) is captured at the snapshot point S. Raw history is never replayed, so historical side effects never repeat; native duplicates on reattach are accepted.

History is a **moving view**: phase C describes S exactly, while phase B sends a snapshot row only if it is still retained when the producer reaches it, marks skipped rows with the notice, and ends early if the application cleared history. Alacritty does not report how many rows a step pushed, and a push at capacity silently replaces the oldest row, so the adapter counts pushes with **counting headroom**: the engine's capacity is `max_rows + 2P`, where `P = max(64, rows, ceil(2 * 65535 / cols) + 1)` bounds the rows one step can push. Headroom rows are never retained history. They cost at most about 262,000 cells at 2 columns and 200,000 cells at 500x200. The eviction flag is content-independent.

The normal screen is read while the application is on the alternate screen by cloning the alternate grid, swapping, reading, swapping back and restoring the clone. This sets the normal saved cursor to the normal current cursor (accepted exception E1).

## Direct and Derived presentation

Session geometry is the controller's host size clamped to 2..=500 columns and 1..=200 rows. A host of exactly that size gets **Direct** presentation: filtered tokens forwarded unchanged. Absolute positioning in a restoration fixes only the initial image; a larger host would then wrap and scroll unchanged live bytes at its own width and bottom row, not the session's, so it gets **Derived** presentation: live output is rendered from engine state inside the session rectangle, pushed history rows enter native scrollback, and only rows that changed are repainted. Hosts narrower than 2 columns (Alacritty's minimum) are unsupported, and the scroll region is not carried across a switch between Direct and Derived.

## Presentation release

The daemon keeps a per-attachment presentation record of what that attachment's committed bytes put its host into (alternate screen, open synchronized update, Derived cache). Prologues and the release that ends a presentation are computed from it, never from the session's engine state: the release returns the host to a documented normal-shell baseline and leaves the alternate screen only if this attachment entered it.

## Zero-buffer synchronized output

The engine never buffers a synchronized update: its `Timeout` reports no pending timeout. The adapter instead latches the first `CSI ? 2026 h` with a 150 ms deadline that repeated begins do not extend; snapshots and derived updates wait for an explicit end or that deadline, and the daemon ends any update it left open on the host with a synthetic `CSI ? 2026 l`.

## Considered options

- **A git or patched Alacritty revision:** rejected; a checksum-locked registry release is reproducible without fetching from GitHub.
- **A second parser, a client-side emulator or a speculative engine trait:** rejected; one engine behind one adapter.
- **Replaying a raw transcript:** rejected; it repeats queries, clipboard writes and clears and cannot bound memory.
- **Raising history capacity as private snapshot storage:** rejected; `set_options` changes the real grid's history and drops rows.
- **The engine's own synchronized-update buffering:** rejected; it re-arms on repeated begins and flushes its whole buffer in one call.
- **Capping combining marks per cell, or only rows, in monitor models:** rejected; a cap truncates accepted characters, and rows bound neither marks nor storage chunks.
- **Reflowing monitor models in place on resize:** rejected; reflow's allocation grows quadratically with the column ratio and keeps old row capacities.
- **A patched or replaced engine for monitor models:** not chosen; admitting each token's worst-case growth before the unchanged engine sees it has found no counterexample (see the outcome above). Reopen this if one is found that admission cannot charge.
