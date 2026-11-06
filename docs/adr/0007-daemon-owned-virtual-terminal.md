# Daemon-owned virtual terminal

Detach, reattach and takeover need the session's screen, history, modes and title without asking any host terminal. The session daemon therefore owns one virtual terminal per session and is the sole authority for terminal state and terminal-query replies. Clients keep no emulator and no transcript. The virtual terminal lives in the shipped library `climon2-vt` (ADR 0004): pure, platform-neutral, infallible, and exposing only climon2-owned types.

## Engine: unchanged Alacritty 0.26.0

`climon2-vt` is the only crate that depends on `alacritty_terminal = "=0.26.0"` (default features off), the published crates.io release locked by checksum `bda177466b9524d59f1b12f0dd30b68696788e9992a7e959021c4a0ed96fcf59` (upstream commit `94e7c8874e526b1e67b349d9ba30ddf81669119e`, parser vte 0.15.0). It is unpatched and not a git dependency: an immutable, reproducible registry release, and the same engine the test harness already uses. The adapter uses only public API: grid and cursor access, `swap_alt`, `set_options`, and `Processor` with a climon2 `Timeout` and `EventListener`.

Evidence limits: earlier qualification ran against revision `bbb2cbf9470a6707b1205fbddbdb25109b14e2fd` (0.26.1-dev) and does not carry over. In 0.26.0 the combining marks of one cell are stored without a bound. Only the seams this choice touches are validated (front filter, sync timing, the alternate-screen swap, history accounting, serialization and derived rendering), including one cell carrying many zero-width marks. Alacritty 0.26.0 implements only `CSI ? 1049 h/l` for the alternate screen; modes 47 and 1047 are unknown to it.

## Live output policy and query authority

A front filter that mirrors vte 0.15.0's state machine splits PTY output into complete tokens (text never split inside a UTF-8 sequence; partial tokens held across reads up to a 4096-byte body, oversized tokens dropped from engine and output). Every token updates the engine; one policy decides what reaches the controller's host:

- supported queries (DA1/DA2/DECID, DSR 5/6, DECRQM, `CSI 18 t`) are answered by the engine's replies to the PTY and stripped;
- every other host-dependent request, keyboard-protocol negotiation, OSC 52, OSC 8, every other OSC and every DCS/SOS/PM/APC is stripped; the host is never asked anything in-band;
- titles are tracked by value and re-emitted as a safely encoded `OSC 2`;
- `CSI ? 47/1047 h/l` parameters are removed, because the engine does not implement them and the host must not diverge from it;
- everything else is forwarded unchanged.

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
Exact structured schemas remain unresolved; the current filter and query
implementation above are not evidence that these planned signals are supported.

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
