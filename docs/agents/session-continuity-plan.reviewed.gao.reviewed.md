# Session continuity: implementation plan

This is the complete instruction set for implementing session continuity in
climon2: detach, reattach, multi-client control, restoration, deliberate kill
and extended listing. It replaces every earlier continuity plan. You need only
this document and the repository's code to implement it. External links point
only to third-party source (Alacritty, vte) used as API evidence; Appendix B
lists them.

Paths in this plan name code to read or change. Documentation files named in
section 8 are edit targets; you do not need to read them for requirements.

Each statement has one of three statuses:

- **Settled**: confirmed product behaviour (unmarked text).
- **Accepted exception**: a known, approved deviation, labelled
  *(accepted exception)*.
- **Plan choice**: a mechanism this plan proposes, labelled *(plan choice)*.
  You may substitute a different mechanism only if it meets the same stated
  invariants and tests. Record each substitution in the action log.

Where this plan says "choose", the implementer decides. Code references are to
`main` at the time of writing. Re-verify line numbers and symbol names before
editing.

## 0. Ground rules

### 0.1 Working method

- Use red-green TDD for every behaviour change. Write the failing test first,
  see it fail for the reason you expect, then make the smallest change that
  passes. Refactor only with behaviour pinned by tests.
- Get the data shape right before writing logic. Put each decision in one
  place. Keep business logic in pure functions behind boundaries that parse
  outside data once, where it enters.
- Make operations idempotent: running one twice, or after a crash halfway
  through, ends in the same state.
- Give shared mutable state one writer, enforced by structure.
- Finish and verify one small unit before starting the next.
- Tests call code the way its user does and check what that user would see. A
  test that would still pass if every function returned nothing is worthless.
- Keep an action log of every action and its reason. Report it at the end.
- A feature is incomplete until automated tests cover it.

### 0.2 Quality gates

- Every function in shipped source must have **CRAP <= 5**. Crapper counts `?`
  as a branch.
- Mutation testing must leave **zero surviving mutants**.
- There are no exemptions. Never weaken thresholds or guards, add exclusions or
  mark mutants as acceptable. Add every new shipped crate to the gates in the
  same commit that creates it (section 3.1).

### 0.3 Git and authority

- Stage only files that your increment changed. Changes you did not make are
  the user's: never stage, commit, revert or stash them. Run `git status`
  before every commit.
- Authorised: commit **passing** increments and push them directly to `main`
  with your required commit trailers. Make small, ordered commits. Each message
  says why, what changed and how it was proven.
- Authorised: open fresh, isolated Terminal.app and VS Code windows for the
  native checks in section 9. Control only windows that your run created, and
  use isolated data directories. Screenshots, clipboard access and the user's
  existing windows are **not** authorised.
- Not authorised: publishing a release, creating release tags or closing
  GitHub issues.
- The user may be unavailable. Ordinary design choices need no approval. If
  something is genuinely blocked (for example a missing permission or an
  unavailable architecture), record it, keep the guard in place and continue
  with independent work.
- Clean up only resources your run created: temporary homes, sockets, windows,
  processes and worktrees.

### 0.4 Confirmed product decisions

The repository owner confirmed these decisions. They are requirements, not
proposals. If a mechanism in this plan cannot meet one, stop that line of work,
record the conflict in the action log, and continue with independent work. Do
not invent a reconciliation.

1. **Engine.** Use unchanged upstream Alacritty (`alacritty_terminal`): no
   patches and no git dependency. It sits behind a pure, platform-neutral
   `climon2-vt` library that keeps upstream types private. No second parser or
   terminal model, no client emulator and no speculative engine trait.
2. **Engine version.** Ship the published `alacritty_terminal` 0.26.0
   (section 3.1). Earlier qualification evidence used upstream revision
   `bbb2cbf9470a6707b1205fbddbdb25109b14e2fd` (0.26.1-dev) and does not carry
   over. Do not claim that revision's bound on combining-mark storage: 0.26.0
   stores combining marks per cell without it.
3. **Authority.** The daemon is the sole authority for control, session size
   and terminal-query replies. It never asks the host terminal a question
   in-band.
4. **Command-facing identity.** The command sees `TERM=xterm-256color` and
   `COLORTERM=truecolor`. This describes a qualified subset, not complete xterm
   compatibility (section 1.17).
5. **Input.** Legacy keyboard input only: no kitty keyboard or
   modifyOtherKeys translation, no custom key encoder and no special
   application-keypad fidelity. Ordinary keys, digits and cursor/paste
   alignment must still work. Reply-shaped key input typed by the user is
   legitimate input. The daemon finishes an application paste that was already
   started before it admits later input.
6. **Restoration.** Faithful restoration of the normal screen and its history
   is mandatory, including while the application is on the alternate screen.
   Deferred exposure or relying on an application redraw is not a substitute.
   Every grant resends all retained history. Native duplicates are accepted.
   There is no history identity, no incremental catch-up and no history-byte
   accounting. A coarse truncation notice is required before known-truncated
   history (section 1.8.2).
7. **Hyperlinks.** OSC 8 metadata is unsupported, live and restored. The linked
   text appears as plain text. Raw OSC 8 never reaches the host. Printed URLs
   are unaffected.
8. **Titles and bells.** Live titles and bells are supported. Titles are
   safely encoded (section 1.8.1). Historical side effects are never replayed.
9. **Cleanup.** Captured OS terminal settings (termios) are restored exactly.
   Terminal-emulator modes that a client changed may return to a documented
   normal-shell baseline (section 1.7.1). Unrelated native history is never
   cleared.
10. **Failure boundary.** A recoverable attachment or transfer failure isolates
    that attachment or transfer and preserves existing control where safe. A
    detected authoritative failure rejects new attachments and input and
    terminates the session with an explicit diagnostic (exit 125 where
    possible). There is no silent degraded emulation and no raw-output success
    fallback. The risk of in-process OOM, abort or stall is accepted
    (section 1.18).
11. **History configuration.** Default 10,000 rows, allowed 0..=100,000, fixed
    at session startup. Zero disables retained history, not screen
    restoration. Attaching never changes it.
12. **Internal budgets** (not user configurable):
    - control-string body cap 4096 bytes; an oversized token is dropped and
      logged, and the session continues;
    - geometry at most 500 columns x 200 rows; larger host sizes are clamped;
      zero is rejected;
    - the engine is fed at most 64 bytes per step; REP counts are not clamped,
      and the resulting worst-case uninterruptible step (about 86 ms) is
      accepted;
    - synchronized output: the engine buffers 0 bytes; the adapter ends an
      update 150 ms after its first begin; restoration snapshots wait until the
      update ends;
    - live queue 16 MiB per attachment; overflow disconnects that attachment
      as an unexpected departure;
    - restoration: 5 s no-progress timeout; overall deadline
      30 s + restoration size / 4 MiB/s;
    - back-to-back resizes must be merged into one.

    There is no restoration-size cap and no mandated serialization strategy.
13. **Accepted exception E1.** A restoration taken while the application is on
    the alternate screen may set the normal screen's saved cursor to the normal
    current cursor (section 1.8).
14. **Self-attachment.** Reject only identified direct self-attachment. No
    indirect-cycle detection.
15. **Cross-version contracts** are frozen or additively frozen (section 1.15).
16. **Native acceptance** uses four lanes (Terminal.app and VS Code, each on
    native arm64 and native Intel) against the exact final archive, checking
    character presence only (section 9).

Out of scope by decision; their absence is not a defect:

- another engine, engine patches or indirect-cycle detection;
- a client emulator or extended keyboard translation;
- history identity or history-byte accounting;
- lossless hyperlinks;
- exact native de-duplication, pixel sampling and elaborate native observers;
- avoiding accepted duplicate history or changing the reason-sensitive leave
  policy.

### 0.5 Vocabulary

Use these terms in code, tests and docs.

| Term | Meaning |
|---|---|
| Session | One run of a command inside a terminal that climon2 owns, identified by a GUID |
| Session daemon | The detached background process that owns a session's PTY, virtual terminal and command |
| Client | The process that connects the user's terminal to a session daemon |
| Host | The user's outer terminal that a client writes to |
| Attachment | One client's accepted connection for interactive use, controlling or not |
| Controller | The attached client whose input and size drive the session; at most one at a time |
| Control period | One uninterrupted period of control by one client, numbered by a monotonic `u64` |
| Grant | The daemon's act of making an attachment the controller and starting a new period |
| Detach | Ending one attachment without ending the session |
| Leave policy | What the daemon does with its session when an attachment leaves |
| Live session | A session whose daemon answers the hello handshake with the matching session ID |
| Probe | A connection that completes hello without attaching |
| Retained history | The normal-screen history rows the session keeps for restoration, at most `max_rows` (section 1.8.2) |
| Native scrollback | The host terminal's own history |
| Restoration | The bytes that bring a newly granted controller's host to the session's state |
| Snapshot point (S) | The engine state a restoration describes |
| Presentation | The daemon's per-attachment record of what its stream has put the host into |
| Direct / Derived presentation | Live output forwarded as filtered tokens, or rendered from engine state when the host is larger than the session (section 1.6) |
| Placeholder | The screen a non-controlling client shows |
| Release | The bytes that return a host to the normal-shell baseline when an attachment's presentation ends |
| Startup envelope | The schema-1 JSON in `CLIMON2_CLIENT_STARTUP` that tells a client its operation and settings |

## 1. Product specification

### 1.1 Commands, usage and results

New usage string (replace `USAGE` in `crates/climon2-core/src/cli.rs`):

```text
usage: climon2 exec <command> [args...] | attach <session-id> | kill <session-id> | list [--json]
```

| Command | Behaviour |
|---|---|
| `climon2 exec <command> [args...]` | Unchanged start-attached behaviour. Also validates configuration before startup and adds continuity (detach/reattach) and the new client keys. No start-detached option. |
| `climon2 attach <session-id>` | Attach to one live session and take control immediately. Requires stdin **and** stdout to be TTYs. |
| `climon2 kill <session-id>` | Deliberately terminate one session. No TTY needed. Silent on success. |
| `climon2 list [--json]` | Existing listing plus VERSION, STATE and CLIENTS (section 1.11). |

Exit statuses (keep the constants in `crates/climon2-core/src/exit.rs`):

- `0`: the daemon confirmed an explicit detach; `kill` confirmed termination and
  cleanup.
- The command's own status: the session's command finished while this client
  was attached, whether controlling or not. Signal-derived statuses stay
  `128 + signal`.
- `2`: missing or malformed arguments or selector (`UsageError`).
- `125`: a runtime or configuration failure. This covers:
  - an unknown or ambiguous selector;
  - an unsupported older session;
  - a missing matching client binary;
  - a non-TTY attach or a self-attach;
  - a configuration error;
  - an attach refused because the session is terminating or at its connection
    cap;
  - unexpected daemon connection loss;
  - an unconfirmed detach, or a failed or unconfirmed kill;
  - an unconfirmed result after the client's release deadline (section 1.7.1);
  - an authoritative session failure (section 1.18).
- `128 + signal`: the client itself was terminated by a handled signal (existing
  behaviour). The existing `TERMINAL_GONE = 129` on terminal input EOF stays.
- Release the presentation and restore the local terminal (section 1.7.1)
  before writing any diagnostic. Keep the existing one-line
  `climon2: error: <message>` contract and the ` (logs: <path>)` suffix rules
  (`crates/climon2-cli/src/lib.rs`, `crates/climon2-client/src/lib.rs`).
- No success banner on attach. Attach, detach and kill write nothing to stdout.
- After restoring the terminal, an explicit detach writes one short stderr line,
  for example:
  `climon2: detached from session <full-id>; reattach with: climon2 attach <full-id>`.
- Unexpected connection loss is never reported as a detach.

### 1.2 Session selectors (attach and kill)

- Accept a full GUID or a unique prefix of **at least 8** hexadecimal digits.
  Matching is case-insensitive, with or without hyphens. Normalise by removing
  hyphens and lowercasing, then require 8..=32 hex digits. Anything else,
  including an omitted selector, is a usage error (2).
- Match only **live** sessions in the selected climon2 home. A session is live
  when its hello probe succeeds, as `list` already determines. Sessions from
  versions that do not support the operation stay in the candidate set. Never
  drop them to make an ambiguous prefix unique.
- No match: 125. More than one match: 125, and the message lists every matching
  full ID. There are no names, pickers or "latest" selection.
- Never retarget. If a target disappears or changes identity between selection
  and dispatch, fail with 125.

### 1.3 User configuration: `<climon2 home>/config.jsonc`

```jsonc
{
  "client": {
    "escape_prefix": "Ctrl+]",     // null disables escape interception
    "take_control_key": "Ctrl+T"   // null disables keyboard takeover
  },
  "scrollback": {
    "max_rows": 10000
  },
}
```

- The format is JSON with `//` and `/* */` comments and trailing commas.
  A missing file, object or field means the default. `null` disables a binding.
- **Comments are whitespace.** A comment separates tokens exactly as a space
  would; it never joins the text on either side. For example,
  `{"scrollback":{"max_rows":1/* c */0000}}` and `{"x":tr/* c */ue}` are
  malformed, not `10000` and `true`. A trailing comma is one whose next
  non-whitespace, non-comment character is `}` or `]`.
- Key strings (exact spelling):
  - `Ctrl+A`..`Ctrl+Z` (bytes 0x01..0x1A);
  - `Ctrl+[` (0x1B), `Ctrl+\` (0x1C), `Ctrl+]` (0x1D), `Ctrl+^` (0x1E),
    `Ctrl+_` (0x1F);
  - `Ctrl+Space` (0x00).

  Nothing else is accepted: no printable keys, Alt, function keys or
  multi-key sequences.
- `take_control_key` may not be `Ctrl+[` (Escape, 0x1B). `escape_prefix` may be.
- If both bindings are enabled and resolve to the same byte, that is a
  configuration error.
- `scrollback.max_rows` is an integer 0..=100000; the default is 10000.
  0 disables retained history only; the current screen is still restored.
  `scrollback.max_bytes` is **not** a setting and, like any unknown field, is
  ignored.
- Each of these errors exits 125 before raw mode and before any session side
  effect:
  - an unreadable existing file;
  - malformed JSONC, including tokens split by a comment;
  - duplicate keys in a known object;
  - the wrong type for a known field;
  - an invalid key string;
  - a binding conflict;
  - an out-of-range `max_rows`.

  Unknown fields are ignored.
- Scope of validation by operation:
  - `exec`: validate `client` **and** `scrollback` before starting the daemon.
  - `attach`: validate `client` only. A malformed file still fails, but an
    invalid, unused `scrollback` value does not.
  - `list`, `kill`: do not read the file.
- Read the file once per operation. The initial client of `exec` must use exactly
  the snapshot validated before startup, with no re-read. A running daemon's
  retention never changes after startup; attaching never changes it.
- There is no environment-variable override, no config command, no
  auto-creation, no rewriting and no live reload.

### 1.4 Client keys: escape, detach, passthrough, paste, takeover

Implement these as a pure input filter in the client.

- The default escape prefix is Ctrl+] (0x1D):
  - prefix then lowercase `d`: detach this client;
  - prefix then prefix: send **one** literal prefix byte to the session
    immediately;
  - prefix then any other byte: forward both bytes unchanged.

  A lone prefix waits indefinitely for the next byte, with no timeout. Output
  keeps flowing meanwhile.
- Ctrl+C, Ctrl+D and similar keys are forwarded unchanged. Only configured
  bindings are intercepted.
- Bracketed paste: input between `ESC [ 200 ~` and `ESC [ 201 ~` is a paste.
  - No binding is interpreted inside a paste.
  - Markers may be split across reads; buffer partial markers.
  - Unmarked pastes are indistinguishable from typing. Document this.
- The take-control key (default Ctrl+T, 0x14) is intercepted **only while the
  client is non-controlling** and outside a paste. It is consumed as a
  take-control request and never sent to the command. While controlling, it is
  an ordinary byte. (It cannot also be the escape prefix, because validation
  forbids that.)
- A non-controlling client still honours the escape prefix and detach. It
  discards all other application input, including paste content.
- With escape interception disabled (`null`) there is no in-band detach. Closing
  that client is an unexpected departure (section 1.7).
- **Filter state has two independent parts:**
  - *Key state*: a pending escape prefix and a partial non-paste header. This is
    reset on every `Granted` and `Revoked`, and at readiness.
  - *Local paste state*: an in-paste flag and a partial-marker buffer. This is
    **never** reset by role changes. It tracks the local terminal's paste stream
    for the client's lifetime.
  - Payload remaining from a paste that began before a role change is discarded,
    including its closing marker. That payload is never interpreted as bindings,
    and is never forwarded as newly typed input, even after the client becomes
    ready. Interpretation resumes after that paste's closing marker.

### 1.5 Control, readiness and stale input

- The daemon is the sole authority over attachments, control and session size.
  Whenever at least one client is attached, exactly one client is the
  controller.
- An explicit attach, including the initial `exec` client, takes control
  immediately. The previous controller stays attached as non-controlling.
- Attach and take-control requests are serialised in daemon acceptance order.
  Each accepted request takes control, so the last accepted request wins. There
  is no approval prompt, lock or inactivity requirement.
- A take-control request from the current controller is a no-op. It starts no
  new control period, adds no hand-back history entry and does not reset the
  screen.
- Controller departure: control automatically returns to the most recently
  controlling client that is still attached. Each actual grant makes the
  recipient the most recent controller.
- **Control period**: every grant starts a new period. The new controller's
  application input stays blocked until it has received and written the
  complete restoration for that period and confirmed readiness. Input typed or
  queued before readiness is discarded and never replayed.
- When control transfers, any of the old controller's input not yet written to
  the PTY is discarded. Late input from an earlier period is ignored, even if
  the same client later regains control. Bytes already written to the PTY
  cannot be recalled.
- **Application paste cleanup**: the daemon finishes any application paste that
  was already started before it admits later input.
  - The daemon tracks, over the input bytes it has **actually written** to the
    PTY, whether an application paste is open. A paste is open when an opening
    marker was written without its closing marker; a marker may be partially
    written.
  - Admission ends for the writing attachment on takeover, hand-back, detach,
    unexpected loss or shutdown. When it ends while a paste is open, the daemon
    discards that attachment's unwritten input. If the engine's
    bracketed-paste mode (2004) is still set, it also queues **PTY cleanup
    bytes**. These are the remainder of a partially written opening marker plus
    `ESC [ 201 ~`, or the remainder of a partially written closing marker.
  - Cleanup bytes are cleanup, not stale payload. They are fully written to the
    PTY, across partial writes, before any later admitted input.
  - If mode 2004 is off, write nothing; the application no longer expects
    markers.
  - This is marker bookkeeping only, not a key encoder or client emulator.
- A non-controlling client receives **no** session content. On revocation the
  daemon commits that attachment's release (section 1.7.1), then `Revoked`, and
  stops sending output to it. The client then shows a placeholder.
  Committing the revocation after any output already committed on the same
  connection guarantees that stale output cannot overwrite the placeholder.
- The client draws the placeholder:
  - It preserves visible outer content as described in section 1.8, phase A.
  - The screen is blank. Its first line is
    `This session is being viewed elsewhere.`
  - Its second line is `Press <key> to take control`, with the configured key
    string, for example `Ctrl+T`. Omit the second line when takeover is
    disabled.
- A rejected takeover leaves the controller unchanged. Ownership is never
  half-accepted.
- Connection cap: keep the existing limit of 64 simultaneous daemon connections,
  counting both clients and probes. Exhaustion never evicts a client or changes
  control. An attachment that cannot be admitted fails explicitly (125).

### 1.6 Sizing and presentation mode

- Only the controller's size drives the PTY and the daemon's virtual terminal.
  Non-controller resizes are only remembered.
- **Resize coalescing (required).** The daemon keeps one pending size per
  attachment; a new resize frame replaces it. Each loop iteration applies only
  the controller's latest pending size, at most once, before reading the PTY
  and before taking any snapshot. A burst of resize frames received together
  causes exactly one PTY resize and one engine resize. Equal sizes cause none
  (existing behaviour).
- On every grant (attach, takeover or hand-back), apply the new controller's
  latest valid size before taking the snapshot for its restoration.
- **Session geometry** is the controller's host size clamped to
  **2..=500 columns and 1..=200 rows**. Two columns is Alacritty's minimum. An
  invalid (zero) initial size falls back to 80x24 (existing
  `TerminalSize::initial`). Later zero-sized updates are ignored (existing
  `TerminalSize::resize`). A host narrower than 2 columns is unsupported; it
  is not tested beyond not crashing. Document this.
- The session occupies a fixed **session rectangle** at the host's top-left
  origin: host rows `1..=session rows` and host columns `1..=session cols`.
- **Presentation mode** is chosen per attachment from its host size:
  - **Direct** when the host size equals the session geometry. Live output is
    the filtered token stream of section 1.9, forwarded unchanged.
  - **Derived** when the host is larger than the session in either dimension
    (only possible above 500 columns or 200 rows). Unchanged live bytes would
    wrap at the host's width and scroll at the host's bottom, not the
    session's. Live output is therefore rendered from the daemon's engine
    state (section 3.2, "Derived presentation"). Wrapping and bottom-row
    scrolling follow the session geometry; host cells outside the rectangle
    stay blank; rows that the session scrolls into retained history enter the
    host's native scrollback.
- If the controller resizes before it has confirmed readiness, issue a fresh
  restoration at the new size in a new period. Its prologue starts from that
  attachment's actual presentation (section 1.8, phase A).
- After readiness, apply resizes normally:
  - Direct to Direct: the application redraws, as today.
  - Any resize where the old or new mode is Derived: emit a derived full
    repaint (section 3.2) before any further live output.
- After an explicit detach leaves zero clients, keep the last controller's size.

### 1.7 Lifetime without clients and deliberate termination

- One daemon-owned leave policy evaluates every departure:
  - **Another client remains attached** (controlling or not): the session
    continues. If the controller left, control is handed back.
  - **The final client leaves by an accepted explicit detach**: the session keeps
    running indefinitely, with no idle timeout, until the command exits or it is
    killed.
  - **The final client leaves unexpectedly**: start termination immediately, with
    no reconnection grace and no cancellation. Unexpected departures include:
    - terminal loss;
    - a client crash;
    - a socket closed without an accepted detach;
    - live-queue overflow;
    - restoration-deadline expiry (section 1.10);
    - a protocol violation after acceptance.
  - An accepted detach is not sticky. After reattachment, a later unexpected loss
    of the final client starts termination again.
  - Probes, kill-mode connections and rejected or not-yet-accepted connections
    are not attachments. Their departure never triggers this policy.
- Explicit detach boundary: a departure is intentional only after the daemon
  accepts a valid detach request.
  - Loss before acceptance is unexpected.
  - Loss after acceptance stays intentional, even if the confirmation is lost.
  - The client reports success only after receiving the confirmation; otherwise
    it exits 125 ("detach not confirmed").
- While detached, the daemon keeps reading PTY output and updating its virtual
  terminal and retained history. Having no clients never blocks the command's
  output.
- Command exit ends the session:
  1. Keep the existing bounded drain (`PTY_DRAIN_IDLE` 100 ms, `PTY_DRAIN_CAP`
     1 s).
  2. Release presentation (section 1.7.1).
  3. Deliver `Exit{code}` to every attached client, controlling or not.
  4. Remove metadata and the socket, and release state.

  There is no completed-session state and no later attachment.
- **Initial attachment obligation**: if no client attaches within 5 seconds of
  daemon readiness, terminate the session. This is a failed startup, not a
  detach. Probes and rejected connections neither satisfy nor extend this
  deadline. A command that exits first follows normal completion.
- The daemon serialises command completion, attach, detach and kill. Once
  shutdown begins, refuse new attachments (125 at the client). Attach and detach
  cannot cancel shutdown.
- `climon2 kill <selector>` takes a single target. It has no `--all`, force,
  signal choice or prompt.
  - It works without a TTY, whether or not clients are attached.
  - The target daemon owns shutdown. Never signal a PID read from metadata.
  - Kill and unexpected final loss both use the existing shutdown sequence:
    1. Hang up the PTY.
    2. Wait the existing 5-second grace (`HANGUP_GRACE`).
    3. Forcibly terminate any remaining managed jobs (`Pty::leave` /
       `Pty::finish` in `crates/climon2-daemon/src/pty.rs`).
  - Preserve the existing process-identity and ownership safeguards.
- Repeated kills join the existing shutdown and do not restart the grace. Kill
  terminates an attach accepted before it; an attach after shutdown begins
  fails.
- Kill result:
  - Silent `0` only after the daemon confirms both that the command terminated
    **and** that cleanup (metadata and socket removal) finished.
  - One monotonic **10-second overall deadline** in the CLI covers selection,
    probing, helper startup, the request and waiting for completion.
  - On expiry: 125 with "termination of session <id> was not confirmed". Do not
    claim the session is still running. An accepted shutdown continues.
  - The CLI may stop and reap only the helper process that it spawned.
- Clients attached during a kill receive the command's actual (signal-derived)
  status if it is delivered. Connection loss before a terminal result is 125.
- Killing the enclosing session is allowed without a prompt.

#### 1.7.1 Presentation release on exit

Every **daemon-confirmed end of an attachment's presentation** gets a release,
in this order. Those ends are revocation, accepted detach, command exit and
shutdown with the attachment present.

1. **Daemon**: after any output already committed for that attachment, commit
   its release bytes as `Output`. Compute them from that attachment's
   **presentation** (section 3.3), not from the session's current engine
   state. Then commit `Revoked`, `Detached` or `Exit`.
   - A non-controlling attachment was already released at revocation, so it
     needs no further daemon release.
   - Never send `CSI ? 1049 l` to an attachment whose host was never put on the
     alternate screen by this attachment's stream. A placeholder client is one
     example.
2. **Client**: write every `Output` byte received before the terminal message
   to stdout, including short or blocked writes. Then end any placeholder
   presentation with its own known bytes: cursor to the last row, then CR LF.
   Then restore termios exactly. Only after that, write any stderr line or
   diagnostic and exit.
3. **Unexpected loss** (no daemon response): the client writes a conservative
   reset and then restores termios. Never send `CSI ? 1049 l` blindly. The
   reset is:
   - `CSI 0 m`;
   - `CSI ? 25 h`;
   - mouse, focus and bracketed-paste modes off;
   - application cursor keys and keypad off;
   - end any synchronized update.

   Document that alternate-screen and other unknown state cannot be restored
   without a daemon response.

**The client's release deadline must be reachable.** Today the bridge
(`crates/climon2-client/src/bridge.rs`) reads server frames only when its stdout
buffer is empty, and disables the socket's poll descriptor (fd `-1`) when it has
nothing to request. While stdout is blocked it therefore cannot see a terminal
message or a hang-up, and a timeout placed in the terminal-message handler is
never reached. Required behaviour:

- The client always observes connection loss (hang-up, error or EOF) and its own
  deadlines, even while stdout is blocked. Keep the socket descriptor in the
  poll set at all times; `POLLHUP` and `POLLERR` are reported even when no
  events are requested.
- While stdout is blocked, the client reads no further frames. It holds at most
  the one `Output` frame it is writing, so its memory stays bounded. It
  processes a terminal message only after every byte received before it has
  been written.
- One **5-second local cleanup bound** starts at the earliest of:
  - a terminal message (`Detached`, `Exit`, `Terminated` or `Error`) is parsed;
  - connection loss is observed;
  - the client sends `Detach`.
- When the bound expires before a terminal message has been processed, the
  client:
  1. stops writing, discarding pending output and unread socket data;
  2. makes one non-blocking attempt to write the conservative reset;
  3. restores termios exactly;
  4. writes one diagnostic, for example
     `climon2: error: terminal output did not drain within 5 s; result of session <id> was not confirmed`,
     or "detach not confirmed" for a pending detach;
  5. exits 125.
- A processed terminal message determines the status as in section 1.1.
- Termination signals keep their existing behaviour (`128 + signal`).

**Documented normal-shell baseline** after release:

- synchronized update ended;
- main screen, only if this attachment entered the alternate screen;
- margins reset; origin, insert and line-feed/new-line modes off; autowrap on;
- mouse, focus, bracketed paste, application cursor keys and application keypad
  off;
- cursor visible with the default cursor style; `CSI 0 m`.

The host title is not restored; document this. Captured OS terminal settings
(termios) are restored exactly.

### 1.8 Restoration

The daemon owns one virtual terminal per session: Alacritty, wrapped by
`climon2-vt` (section 3.2). Clients keep no emulator and no hidden transcript.

On **every grant**, the daemon sends the new controller a restoration and then
live output. Grants are the initial exec attach, an explicit attach, takeover,
hand-back and a re-grant after a pre-readiness resize.

**Snapshot point S.** The restoration describes the engine's state after the
**last completed token**. It does not wait for the front filter to have no
partial token.

- A partial token held by the front filter has reached neither the engine nor
  any host. It stays pending. When it completes, it is fed to the engine and,
  if section 1.9 forwards it, sent to the new controller as live output after
  the restoration.
- The only wait before the snapshot is the synchronized-update latch, which is
  bounded (section 1.10).

**Incremental production.** The restoration is a **prefix** of the
attachment's stream. It is produced incrementally as the attachment's socket
drains, and it is not counted against the live queue.

- There is no cap on total prefix size. Only produced-but-unwritten prefix
  bytes are bounded, by a refill window *(plan choice: 1 MiB)*.
- Live output after S waits behind the prefix, counted against the live queue
  (16 MiB). A prefix larger than 16 MiB therefore reaches a healthy reader
  completely and in order.
- Valid history is never truncated to fit a queue.

**Three phases, in this order.** Every byte of an earlier phase precedes every
byte of a later phase.

**Phase A: lead.** Computed and committed at the grant, from this attachment's
presentation (section 3.3) and its host size.

- **A1. Prologue.** The host may still hold modes from an earlier application,
  or from an earlier restoration on this same attachment. Emit, in this order:
  - `CSI ? 1049 l`, only if this attachment's presentation is on the alternate
    screen. A replacement restoration before readiness is the usual cause. This
    attachment created that alternate presentation, so leaving it returns the
    host to its genuine normal screen.
  - `CSI ? 2026 l` to end any open synchronized update.
  - `CSI r` to reset margins.
  - `CSI ? 6 l` (origin off), `CSI 4 l` (insert off) and `CSI 20 l`
    (line-feed/new-line off).
  - `CSI ? 7 h` (autowrap on) and `CSI 0 m`.

  Never blindly reset a host screen whose original state is unknown.
- **A2. Preserve visible outer content.** Move the host cursor to the bottom row
  (`CSI <host rows> ; 1 H`, using the controller's reported, unclamped rows).
  Then emit `host rows` line feeds, so the visible host screen scrolls into
  native scrollback. Never send `CSI 2 J`, `CSI 3 J`, `ESC c` or anything else
  that clears native scrollback.
- **A3. Truncation notice.** If the eviction flag (section 1.8.2) was set at S
  and the snapshot's retained history is non-empty, emit
  `[climon2: older session scrollback was discarded]` in the default rendition,
  then CR LF.

**Phase B: history.** The snapshot's retained-history rows (section 1.8.2),
oldest first, produced incrementally from the live engine.

- Render each row from parsed cell state: text, wide characters, combining
  marks and SGR rendition (bold, dim, italic, underline styles, inverse,
  hidden, strikeout, and foreground, background and underline colours).
- In Direct presentation, end hard-ended rows with CR LF and write
  soft-wrapped rows full-width without CR LF, so the host wraps them itself. In
  Derived presentation, end every row with CR LF.
- Rows printed from the host's bottom row scroll into native scrollback as
  printing continues.
- If a snapshot row is no longer retained when the producer reaches it, skip to
  the oldest row that is, and emit the notice line at that point, unless the
  previous emitted line was already the notice. If no snapshot row remains,
  emit the notice (with the same exception) and end phase B.
- If the application cleared history after S, end phase B at that point
  without a notice. The host receives the clearing token as live output after
  the restoration, so the end state agrees.

**Phase C: screen suffix.** Captured at S, except C1.

- **C1. Scroll off.** Only if phase B emitted at least one row: move to the
  host's bottom row and emit `host rows` line feeds, so restored history sits
  in native scrollback and the visible host screen is blank.
- **C2. Normal screen.** Render each normal-screen row at its absolute
  position inside the session rectangle, using `CSI <row> ; 1 H` per row.
- **C3. Normal cursor.**
  - First establish the normal screen's saved cursor (position and pen, then
    `ESC 7`). Then establish the current cursor position and pen.
  - In Direct presentation, if the cursor has a pending wrap
    (`Cursor::input_needs_wrap`), position at the last column and re-print
    that cell's character with its rendition. The host then also enters the
    pending-wrap state; CUP alone clears it. Derived presentation positions
    the cursor only, because no raw text follows.
- **C4. Alternate screen.** If the application is on the alternate screen:
  - emit `CSI ? 1049 h`;
  - render the alternate screen's rows in the session rectangle (it has no
    history);
  - establish its saved cursor and its current cursor and pen, as in C3.
- **C5. Modes.** Emit DECSET/DECRST so that the host matches the virtual
  terminal's modes:
  - cursor visibility (25);
  - application cursor keys (1);
  - application keypad (`ESC =` / `ESC >`);
  - bracketed paste (2004);
  - mouse reporting modes (1000/1002/1003) and encodings (1006/1005);
  - focus reporting (1004);
  - alternate scroll (1007);
  - cursor style (DECSCUSR from `Term::cursor_style`);
  - in Direct presentation only: autowrap (7), origin (6), insert
    (`CSI 4 h/l`) and line-feed/new-line (`CSI 20 h/l`). Derived presentation
    keeps the host at the prologue's values for these four, because its
    output relies on them.
- **C6. Title.** Emit `OSC 2 ; <encoded title> ESC \` with the application's
  current title, or the default title when none is set (section 1.8.1).

Then mark the end of restoration (`RestoreEnd`, section 3.4). Live output
follows in order.

Why this order matters:

- The prologue leaves an alternate presentation **before** any history, so
  history reaches the host's normal screen and native scrollback, not the
  alternate buffer.
- Margins, origin and insert modes are reset **before** history, which relies
  on them.
- Visible outer content is preserved **before** anything overwrites it.
- When the application is on the alternate screen, phase B and C2–C3 still
  restore the underlying normal screen and history before C4. This is
  required. When the application later leaves the alternate screen, the host
  shows the correct normal content.

Never replay raw historical output, clears, bells, queries, clipboard
operations or other side effects. Hyperlinks become plain text.

Not restored; document these as limitations:

- scroll region, tab stops and character sets;
- palette changes (OSC 4/10/11), which are also not forwarded live
  (section 1.9);
- keyboard-protocol modes;
- hyperlinks;
- the host title on exit.

*(accepted exception E1)*: when a restoration is taken while the application is
on the alternate screen, the normal screen's **saved cursor** becomes the normal
current cursor at restoration time (section 3.2). The normal content, history
and current cursor are unchanged. Usually this matches the previous saved
cursor exactly. It can differ after a hidden resize. Document this.

Repeated attachment repeats history that the host may already hold. Native
duplicates are accepted. There is no incremental catch-up and no per-client
delivery tracking: every grant resends all retained history.

Live output that clears or resets the host keeps its ordinary meaning. An
application running `clear` is an example.

#### 1.8.1 Titles

- Default title: `climon2 exec <command> <args...>`, joined by single spaces
  from the daemon's application argv. Non-UTF-8 arguments are converted with
  lossy UTF-8. Examples: `climon2 exec bash`, `climon2 exec vim notes.txt`.
- Application titles (OSC 0/2, title stack push/pop) replace the default title.
  A title reset returns to the default.
- Every title emission, restored, live and derived, preserves ordinary Unicode.
  It percent-encodes each UTF-8 byte of these characters as uppercase `%XX`:
  - every C0 control (U+0000..U+001F);
  - DEL (U+007F);
  - every C1 control (U+0080..U+009F, that is `%C2%80`..`%C2%9F`).

  Nothing else is encoded.

#### 1.8.2 Retained history and the truncation notice

- **Retained history** is the newest `min(H_normal, max_rows)` normal-screen
  history rows, where `H_normal` is the number of history rows the engine
  currently holds for the normal screen. The engine may hold more rows than
  `max_rows` as counting headroom (section 3.2). Those extra rows are **never**
  retained history: no snapshot includes them, no restoration or derived
  update sends them, and nothing an attachment does turns them into retained
  history. With `max_rows` 0, retained history is always empty.
- At S, the snapshot's history is the `h0` retained rows at S, numbered
  `0` (oldest) to `h0 − 1`.
- **Moving view.** Phase C describes S exactly. Phase B sends each snapshot row
  only if it is **still retained** when the producer reaches it. Rows that later
  output pushed past `max_rows` before they were sent are skipped, and the gap
  is marked with the notice. If the application cleared history after S,
  phase B ends early. The prefix therefore never claims to be the complete
  history at S; it is the history at S minus rows that left retention or were
  cleared before they were sent.
- An unfinished restoration never extends retention, and a later snapshot is
  never influenced by an earlier, unfinished one.
- **Eviction flag.** A coarse, content-independent flag:
  - set when any row leaves retained history because later output pushed it
    past `max_rows`, including when the evicted and replacement rows have
    identical content (for example `yes` output);
  - set when the adapter cannot account for history exactly (section 3.2);
  - set when a resize may have dropped history rows;
  - cleared when the application clears history (`CSI 3 J` or `ESC c`).

  It does not measure what was lost.
- **Notice text**: `[climon2: older session scrollback was discarded]`, in the
  default rendition, followed by CR LF. It is presentation text, not part of
  retained history. It appears in phase A when the flag was set at S and the
  snapshot history is non-empty, and at gaps in phase B and in derived updates.

### 1.9 Live output policy at the VT seam

One policy, applied by the `climon2-vt` front filter, decides what reaches the
controller's host.

- The daemon reads the PTY continuously, and every byte updates the virtual
  terminal.
- Output goes to the controller only, when one exists and its restoration has
  started (section 1.8).
- In **Direct** presentation, forwarded tokens are sent unchanged. In
  **Derived** presentation, the classification below still runs (replies,
  stripping and title tracking are the same), but no class 8 token is sent
  raw; the controller instead receives derived updates (section 3.2).
- Nothing here inspects or filters **input**. Reply-shaped key input, such as
  CPR-shaped key encodings, is legitimate and passes unchanged.

Each output token is classified as one of the following:

1. **Answered by the daemon, stripped from live output.**
   - The daemon writes the engine's replies (`Event::PtyWrite`) to the PTY,
     whether or not clients are attached. The host never sees these requests:
     - `CSI c`, `CSI 0 c` (DA1); `CSI > c`, `CSI > 0 c` (DA2); `ESC Z` (DECID);
     - `CSI 5 n`, `CSI 6 n` (DSR);
     - `CSI Ps $ p` and `CSI ? Ps $ p` (DECRQM);
     - `CSI 18 t` (text area size in characters).
   - A supported query split across PTY reads is reassembled by the front filter
     and answered exactly once.
2. **Host-dependent requests, stripped and unanswered.** The application sees
   what it would see from a terminal without that feature. The host is never
   recruited to answer, attached or not. This class contains:
   - every other `CSI … c`, `CSI … n` (including `CSI ? 6 n`), `CSI … x`
     (DECREQTPARM) and `CSI > … q` (XTVERSION);
   - every `CSI … t` window operation;
   - `CSI ? … S` (XTSMGRAPHICS);
   - every OSC whose request parameter is `?` (OSC 4/10/11/12/… colour queries);
   - every DCS, SOS, PM and APC string, including DECRQSS `DCS $ q` and
     XTGETTCAP `DCS + q`.

   Ignore the corresponding engine events (`ColorRequest`,
   `TextAreaSizeRequest`, `ClipboardLoad`). The engine still processes these
   tokens for its own state.
3. **Keyboard-protocol negotiation, stripped.** Input is legacy-only. Strip
   `CSI ? u`, `CSI > … u`, `CSI < … u` and `CSI = … u` (kitty keyboard), and
   `CSI > … m` (xterm modifyOtherKeys). Configure the engine with
   `kitty_keyboard: false`.
4. **Clipboard, stripped.** Strip OSC 52 entirely, both store and read. Also
   configure the engine with `Osc52::Disabled` and ignore `ClipboardStore`.
5. **Titles, re-encoded.** Strip raw OSC 0/1/2. When the tracked title value
   changes after feeding the token, emit `OSC 2 ; <encoded title> ESC \`
   (section 1.8.1) in its place. Compare values; never react to the engine's
   `Title` events alone (section 3.2 explains why).
6. **Hyperlinks, plain text** *(plan choice; an approved degradation)*. Strip
   OSC 8 open/close sequences. The linked text, which is outside the control
   string, is forwarded normally. This matches the restoration. Mechanically
   percent-encoding the link controls is the permitted alternative. Raw OSC 8
   is never forwarded.
7. **Every other OSC is stripped.** Examples are palette set/reset, OSC 7 and
   shell-integration marks. They are host side effects that restoration cannot
   reproduce. Document this as a limitation.
8. **Everything else is forwarded unchanged** (Direct presentation). This
   covers text, C0 controls, ESC sequences and the remaining CSI sequences:
   SGR, cursor movement, modes, synchronized-output begin/end and so on.

Other rules:

- Control strings (OSC, DCS, SOS, PM, APC) are held until their terminator.
  If a body exceeds **4096 bytes**, the token is dropped from both the engine
  and live output and logged without content; the session continues. The same
  rule applies to CSI sequences longer than 4096 bytes.
- The engine's `Bell` events have no daemon side effect. In Direct
  presentation a bell reaches the host as a forwarded C0 byte; in Derived
  presentation, as a derived BEL (section 3.2).
- Hyperlinks never fail or terminate a session.

### 1.10 Internal budgets (not user configurable)

| Budget | Value |
|---|---|
| Control-string / CSI cap | 4096-byte body; oversized tokens are dropped and logged |
| Geometry | 2..=500 columns, 1..=200 rows; larger hosts clamped and use Derived presentation; zero rejected (80x24 initial fallback); session rectangle at the host origin |
| Engine step | At most 64 bytes per step. Printable text and C0 controls may share a step; every ESC, CSI and control-string token is fed as its own step or steps (section 3.2). No engine-side synchronized-update buffering, so no exception to the step bound |
| History counting headroom | Engine history capacity is `max_rows + 2P` rows, where `P` is the per-step push bound for the current geometry (section 3.2). Never retained history. Internal; not user visible |
| Resize | Merged: at most one PTY and engine resize per loop iteration, using the controller's latest size |
| Synchronized output (`CSI ? 2026 h/l`) | **Zero engine buffering**; adapter deadline **150 ms from the first begin** (a first-begin latch: repeated or fragmented begins do not extend it). The snapshot for a pending grant, and every derived update, waits for an explicit end or that deadline; `tick` services the deadline without another PTY read. Host presentation is separate: the daemon tracks whether this attachment's host has an open update and emits a synthetic `CSI ? 2026 l` on deadline, revocation, release or restoration. This is not a hard wall-clock CPU bound |
| Live queue (per attachment) | 16 MiB of live output: Direct tokens waiting behind a restoration plus committed live and derived bytes. The restoration prefix is not counted. Overflow disconnects that attachment as an unexpected departure |
| Restoration prefix | Produced incrementally; at most 1 MiB produced-but-unwritten (plan choice). No total-size cap |
| Restoration deadline | One overall deadline per period, from grant to accepted `Ready{period}`, covering preparation (synchronization latch), delivery and the client's write-and-`Ready`. It fails on either condition below, as an unexpected departure: (a) **no socket write progress for 5 s** while prefix or queued bytes are outstanding; (b) **not `Ready` within `30 s + produced_prefix_bytes / 4 MiB/s`**, where the allowance grows as prefix bytes are produced |
| Initial attachment | 5 s after readiness |
| Kill operation | 10 s overall (CLI) |
| Client cleanup bound | 5 s from the earliest of: terminal message parsed, connection loss observed, `Detach` sent (section 1.7.1) |
| Connections | 64 (existing `MAX_CONNECTIONS`) |
| Startup envelope | 4096 bytes |

### 1.11 Discovery, hello extensions and listing

Hello stays frozen (section 1.15). The new daemon adds **optional** reply
fields:

```json
{"session_id":"…","climon2_version":"…","protocol":1,
 "capabilities":["attach","kill","state-v1","client-startup-v1"],
 "state":{"lifecycle":"attached","attachments":2,"controlled":true}}
```

- `capabilities` is an array of strings, and unknown tokens are ignored.
  - Never infer support from versions or protocol numbers.
  - A malformed advertisement grants nothing and is logged. It does not
    invalidate the mandatory hello or mark the session stale.
  - Do **not** advertise `nesting-v1`.
- Required capabilities:
  - public attach requires `attach` and `client-startup-v1`;
  - public kill requires `kill` and `client-startup-v1`;
  - showing state requires `state-v1`.
- `state` is one consistent point-in-time snapshot. `lifecycle` takes one of
  these values:
  - `starting`: awaiting the initial attach; 0 attachments; `controlled:false`.
  - `attached`: attachments > 0; `controlled:true`.
  - `detached`: after an accepted explicit detach; 0 attachments;
    `controlled:false`.
  - `terminating`: shutdown under way; attachments >= 0; `controlled` only if
    attachments > 0.

  `attachments` counts accepted attachments only.
- `list --json` keeps the existing array of metadata objects. Each object gains
  a `"state"` key holding that state object, or `null`. It is `null` when there
  is no `state-v1`, or when state is missing, malformed or inconsistent. Log
  malformed data. Do not change metadata.
- Human `list` keeps SESSION, CREATED, COMMAND and ARGS, safe quoting and
  truncation, and newest-first order. It adds three columns:
  - VERSION: the metadata `climon2_version`.
  - STATE: the lifecycle, or `unknown`.
  - CLIENTS: the attachment count, including `0`, or `-`.

  Suggested order: SESSION, CREATED, VERSION, STATE, CLIENTS, COMMAND, ARGS.
- Listing never requires the matching client binary.

### 1.12 Version-matched dispatch and the client startup envelope

A session daemon is only ever spoken to after hello by the client of its own
version. The CLI never talks to a daemon after hello. For attach and kill it:

1. Selects the target (section 1.2) and checks the required capabilities.
2. Checks that the matching client exists at
   `<executable home>/<target climon2_version>/climon2-client`
   (`ExecutableLayout::client_invocation` in `crates/climon2-core/src/layout.rs`).
   Otherwise it fails with 125, naming the version. Never use a different
   version's client, and never install anything.
3. For attach only, requires stdin/stdout TTYs, the self-attach check (section
   1.13) and a validated client-settings snapshot.
4. Invokes the frozen argv `climon2-client attach --metadata <absolute path>`.
   It sets the environment variable `CLIMON2_CLIENT_STARTUP` to a schema-1 JSON
   envelope, always overwriting any inherited value:

```json
{"schema":1,"operation":"attach","settings":{"escape_prefix":29,"take_control_key":20}}
{"schema":1,"operation":"kill"}
```

The client applies the envelope rules before raw mode and before any daemon
request. Every failure exits 125.

- The envelope is a UTF-8 JSON object of at most 4096 bytes. `schema` must be
  `1`. `operation` must be exactly `attach` or `kill`.
- `attach` requires `settings` with both fields present. Each field is an
  integer control byte (0..=31) or `null`. Revalidate the key vocabulary, the
  exclusion of `27` for take-control and the conflict rule. Never fill missing
  fields from defaults.
- `kill` must not carry `settings`.
- These are errors: duplicate known fields, missing or wrong-typed known fields,
  malformed JSON, invalid UTF-8 and envelopes over 4096 bytes. Unknown optional
  fields are ignored.
- A present, valid envelope is used exactly, with no config read. A present,
  invalid envelope never falls back.
- An **absent** envelope means attach. The client then loads and validates its
  own client settings once, so generic callers of the frozen argv keep working.
- The CLI removes `CLIMON2_CLIENT_STARTUP` from the daemon's and the
  application's environment. It is internal startup input, not a user
  configuration override.
- `exec` passes the same envelope (operation `attach`, the pre-startup
  snapshot) to the client that it `exec`s into.
- On its own hello, the client rechecks the session identity, its own version
  and protocol, and the currently advertised capabilities before sending an
  operation request.
- Kill mode never attaches, takes control, enters raw mode, requires a TTY or
  sends application input. Its connection is not an attachment.

Retention handoff: the CLI passes the validated value to the daemon through the
existing private daemon argv as a **required** option,
`--scrollback-max-rows <0..=100000>`, placed before `--`. The daemon revalidates
the range. On invalid or missing input it fails before starting the
application.

### 1.13 Self-attachment

- If the invoking environment's `CLIMON2_SESSION_ID` (the session enclosing the
  caller) equals the selected target, refuse before dispatch. Exit 125 with an
  explicit diagnostic. Control does not change and nothing is disrupted.
- Make this check in the **CLI** attach path, as a preflight before dispatch, so
  that the decision lives in one place.
  - Today only the daemon and the application receive the projected session
    environment (`env_clear()` plus projected values in
    `crates/climon2-cli/src/exec.rs`).
  - The client that `exec` replaces itself with inherits the CLI's own
    environment. Its `CLIMON2_SESSION_ID` is therefore the *enclosing* session's
    ID, if there is one.
  - Keep that contract and test it.
- If the enclosing session cannot be identified, do not refuse for that reason.
- There is no ancestry graph, no indirect-cycle detection, no lock and no
  nesting advertisement. Other cycles are the user's responsibility; document
  this.

### 1.14 Out of scope

- the monitor;
- global or indirect cycle detection;
- extended keyboard protocols and application-keypad fidelity (ordinary keys,
  digits and cursor/paste alignment must still work);
- crash and reboot recovery;
- completed-session persistence;
- pixel-exact native rendering;
- exhaustive re-testing of Alacritty behaviour;
- iTerm2;
- start-detached;
- session names.

### 1.15 Frozen contracts (must not change)

- **Launcher**: reads one line from `<home>/current` and runs
  `<home>/<version>/climon2-cli` with unchanged argv. The launcher stays
  dependency-free.
- **Client argv**: exactly `climon2-client attach --metadata <absolute path>`.
  Every other input reaches the client through the environment.
- **Framing**: a big-endian `u32` length (covering the type byte and body), then
  one type byte, then the body. The length is 1..=1,048,576. Data frames
  (input/output) are at most 65,536 bytes.
- **Hello**:
  - type `0x01`; request `{"protocol":1}`;
  - the reply has mandatory `session_id`, `climon2_version` and `protocol`;
  - 2-second absolute deadline.

  Optional reply fields may be added; readers ignore unknown fields.
- **Metadata**: `<home>/sessions/<guid>.pty`, written once, with eleven core
  fields: `format_version`, `session_id`, `socket_path`, `daemon_pid`,
  `daemon_start_time`, `command`, `args`, `cwd`, `depth`, `created_at` and
  `climon2_version`.
  - Readers accept `format_version >= 1` and ignore unknown fields.
  - Never write dynamic state into metadata.
  - Existing goldens under `crates/climon2-core/tests/goldens` are append-only.
- **Additively frozen.** Never change the meaning of these. New behaviour needs
  a new token or schema:
  - the schema-1 `CLIMON2_CLIENT_STARTUP` envelope, with its fields, operations
    and validation rules (section 1.12);
  - the capability tokens `attach`, `kill`, `state-v1` and
    `client-startup-v1`;
  - the `state` object's fields and its lifecycle names `starting`, `attached`,
    `detached` and `terminating`.
- Messages after hello are version-private, exchanged only between a client and
  daemon of the same version. You may add new type codes and change post-hello
  bodies freely.
- Older releases are not retrofitted. Unsupported operations against them fail
  with 125 without disturbing the session.

### 1.16 Client diagnostics per invocation

Every client invocation writes its **own** diagnostic log in the existing
session-log directory, `<home>/logs/<session-id>/`. This covers the initial
`exec` client, every `attach` and kill mode.

- *(plan choice)* Name each file `client-<UTC timestamp>-<pid>.log`. Keep
  `create_new`, `0600` and `O_NOFOLLOW`, content-free records, retention and the
  ` (logs: <path>)` diagnostic suffix rules.
- Never truncate, delete, share or reopen an earlier client's log.
- A collision on this unique name is an ordinary I/O failure. It must not occur
  in normal use, and reattach and concurrent attach must never collide.
- Today `LogFile::session(..., Role::Client, ...)`
  (`crates/climon2-platform/src/logging.rs`) opens the fixed name `client.log`
  with `create_new(true)` (`session_log` in `crates/climon2-client/src/lib.rs`).
  The second attach therefore fails before hello. Fix this before any
  continuity path is exercised (increment 3).
- Update the assertions that depend on the fixed name, in
  `crates/climon2-client/tests/bridge.rs` and
  `crates/climon2-e2e/tests/packaged.rs` (search for `client.log`).
- Logs never contain terminal content, titles, arguments or environment values.

### 1.17 Command-facing environment

- The projected environment given to the daemon and the application always
  sets `TERM=xterm-256color` and `COLORTERM=truecolor`, **replacing** any
  inherited value, including an empty one.
- Everything else is unchanged: other inherited variables pass through as exact
  OS strings, `CLIMON2_SESSION_ID` and `CLIMON2_SESSION_DEPTH` are set as today,
  and `CLIMON2_CLIENT_STARTUP` is removed (section 1.12).
- The client's own environment (the CLI's environment) is unchanged. Its
  `CLIMON2_SESSION_ID` stays the enclosing session's, so the self-attach check
  (section 1.13) is unaffected.
- This intentionally changes today's behaviour, where
  `SessionEnvironment::for_session` (`crates/climon2-core/src/session.rs`)
  keeps an inherited `TERM` and only defaults it when absent, and sets no
  `COLORTERM`. Update the tests listed in section 3.6.

### 1.18 Failure destinations

Every real failure has exactly one destination. Do not add failure variants
for operations that cannot fail: `Vt::feed`, `tick`, snapshotting, production
and derived rendering are infallible.

| Failure | Destination |
|---|---|
| Connection cap reached; shutdown under way; invalid or unsupported request before acceptance | **Rejected admission**: send `Error`, close. No control change. The client exits 125 |
| Socket I/O error, live-queue overflow, restoration deadline expiry or protocol violation on an accepted attachment | **Attachment loss**: an unexpected departure (`Lost`); the leave policy hands control back or, for the final attachment, starts termination |
| A takeover or attach that cannot complete after acceptance (the attachment is lost meanwhile) | Attachment loss of the requester; the previous controller is re-granted under the hand-back rule |
| PTY read, write or resize error; failure to spawn or reap the command; cleanup failure | **Authoritative failure**: log an explicit content-free diagnostic, refuse new attachments and input, run the bounded shutdown (section 3.3) and send each attachment its release followed by `Error{message}` (client exits 125). A kill requester gets `Error`, never `Terminated` |
| History accounting cannot be exact (counting headroom exhausted) | Not a failure: set the eviction flag, end in-progress restorations' history phase with the notice and log (section 3.2) |
| Oversized control string or CSI | Not a failure: drop and log (section 1.9) |
| Client terminal input EOF; client stdout or terminal I/O error | Existing client behaviour (129 for terminal input EOF, 125 otherwise), after the release rules of section 1.7.1 |
| In-process OOM, abort or stall | Not contained; accepted risk. No `catch_unwind` containment or recovery emulation |

## 2. Current code map

The shipped crates are all synchronous, with no async runtime. The package
layout follows one package per shipped binary; the CLI, daemon and client never
link each other and reach each other only by path at runtime. Shared code lives
in `climon2-core` (platform-neutral, no `cfg(target_os)`) and
`climon2-platform` (platform seams).

- **`climon2-core`**:
  - `cli.rs`: `CliArgs::{Exec, List, Help, Version}` and `USAGE`.
  - `exit.rs`.
  - `identity.rs`: `SessionId`, `VersionName` and related types.
  - `json.rs`: `from_object`.
  - `layout.rs`: `HomeLayout`, `ExecutableLayout::client_invocation` and
    `RuntimeLayout`.
  - `metadata.rs`: `SessionMetadata`.
  - `protocol.rs`: constants; `Frame`; `read_frame` and `write_frame`;
    frame-type validation at the `0x01 | 0x10..=0x12 | 0x20..=0x22 | 0x7f`
    match; `TerminalSize`, `ConnectionState`, `HelloReply`, `ClientMessage` and
    `ServerMessage`.
  - `session.rs`: `SessionEnvironment::for_session` injects
    `CLIMON2_SESSION_ID` and `CLIMON2_SESSION_DEPTH`, and defaults `TERM` only
    when absent (changes in section 1.17).
  - `diagnostics.rs`.
- **`climon2-cli`**:
  - `exec.rs`: the TTY preflight, then the daemon spawn with private argv
    `--session-id --home --socket --version --cli-pid --rows --cols --cwd [--log]
    --readiness-fd -- <command> <args>`, then the readiness wait, then
    `CommandExt::exec` into the client.
  - `list.rs`: `discover` scans metadata and probes hello in scoped threads.
    It also does stale cleanup via process identity, `table` and JSON output.
- **`climon2-daemon`**:
  - `args.rs`: the private argv parser.
  - `startup.rs`: prepare, publish and cleanup.
  - `pty.rs`: `Pty::spawn/resize/write/read_output/try_wait/leave/finish`.
  - `connection.rs`: per-connection framed I/O and queue.
  - `session.rs`: the single-threaded loop, with `TICK` 2 ms, `OUTPUT_CHUNK`
    32 KiB and `MAX_CONNECTIONS` 64.

  Current behaviour:
  - There is only one controller; a second `Attach` is rejected.
  - The PTY is read only when the controller can accept output
    (`pump_output` backpressure). Output produced before the first attach
    therefore waits in the PTY and reaches the client raw.
  - Output goes only to the controller.
  - Controller departure hangs up the session.
  - PTY errors propagate out of `serve` and end the daemon.
- **`climon2-client`**:
  - `lib.rs`: argv, metadata, logs (`session_log`, fixed `client.log`), hello
    validation, attach and raw mode.
  - `transport.rs`: framed transport, hello and attach.
  - `bridge.rs`: a `poll` loop over socket, stdin and stdout. SIGWINCH becomes
    `Resize`, signals become `128+signal` and stdin EOF becomes 129. It reads
    server frames only when its stdout buffer is empty
    (`Bridge::receive_ready`), and `descriptor` sets the socket fd to `-1`
    when `socket_events` is empty, so a hang-up is invisible while stdout is
    blocked (section 1.7.1).
- **`climon2-platform`**: PTY allocation, processes, signals, termios
  (`RawModeGuard`), sockets, peer credentials, presentation and logging.
  `LogFile::open` uses `create_new(true)`.
- **`climon2-launcher`** and **`climon2-install`** are unaffected, except for
  docs and tests if needed.

Test-only crates:

- **`climon2-test-term`**: a real PTY plus published `alacritty_terminal` 0.26.0.
  - API: `Terminal::spawn/send/screen/wait_for_text/wait_for_bytes/resize/wait_exit`.
  - It writes `Event::PtyWrite` replies back automatically.
  - `shell.rs` holds the bash profiles.
  - Its normal dependencies turn on the `test-hooks` feature of the CLI,
    daemon, client and installer, so workspace tests build hooked binaries.
    Release builds use `-p <package>` per shipped package, and a release gate
    checks that no hook marker is present.
- **`climon2-fixtures`**: `climon2-fixture`, `climon2-tui-fixture`,
  `climon2-cli-fixture` and `climon2-daemon-fixture`.
- **`climon2-e2e`**: `tests/packaged.rs` and `tests/support/mod.rs` (with
  `Package`).

Existing `test-hooks` fault injection: `CLIMON2_CLIENT_FAULT=panic` (client) and
`CLIMON2_DAEMON_FAULT=panic|never-ready` (daemon). New hooks follow the same
pattern and must stay `test-hooks`-only.

Guards and gates:

- **`xtask/src/quality.rs`**:
  - the `SHIPPED` source list;
  - `CRAP_THRESHOLD = "5"`;
  - the crapper and mutator pins;
  - changed-source selection via `CLIMON2_MUTATION_BASE`.
- **`xtask/tests/architecture.rs`**:
  - the exact package set (`PACKAGES`, checked by
    `workspace_has_the_agreed_packages`);
  - allowed internal dependency edges (`allowed_internal_dependencies`,
    checked by `shipped_packages_follow_dependency_direction`).
- **Other xtask modules**: `xtask/src/release.rs` (`PACKAGES`),
  `xtask/src/dist.rs` and `xtask/src/fixtures.rs`.
- **CI, `.github/workflows/check.yml`**:
  - The `macos` job is a matrix of runner x bash profile.
  - The `quality` job runs on `macos-15` and **pins `CLIMON2_TEST_BASH:
    /bin/bash`**.
  - The `dist` job builds **one** universal artifact, on `macos-15` (arm64)
    only.
  - The `archive` job tests that archive on both `macos-15` and
    `macos-15-intel`.
  - CI does **not** run Terminal.app or VS Code.
- **CI, `release.yml`**.

## 3. Design

### 3.1 Crates, dependency direction and engine pin

- Add the shipped crate **`crates/climon2-vt`**. It is the only crate that
  depends on `alacritty_terminal = { version = "=0.26.0", default-features = false }`
  from crates.io.
  - It exposes climon2-owned types only; no Alacritty types appear in its
    public API.
  - It is platform-neutral: no I/O, sockets or PTYs, and no `cfg(target_os)`.
  - It has no internal dependencies.
- **Engine pin. Decision: ship published `alacritty_terminal` 0.26.0.**
  - Identity: the `alacritty_terminal` entry in `Cargo.lock`, checksum
    `bda177466b9524d59f1b12f0dd30b68696788e9992a7e959021c4a0ed96fcf59`. Its
    registry archive identifies upstream commit
    `94e7c8874e526b1e67b349d9ba30ddf81669119e`. Its parser is vte 0.15.0.
  - The source is unchanged upstream, with no patches and no git dependency.
  - Justification:
    - It is an immutable, checksum-locked registry release, reproducible in
      release builds without fetching from GitHub.
    - It is the exact engine already locked for the test harness, so product
      and harness share one engine.
    - The adapter uses only public API, verified against this source: grid and
      cursor access, `swap_alt`, `set_options`, `Processor` with a custom
      `Timeout` and `EventListener`.
  - Evidence limits (section 0.4, item 2): record them in the new ADR and the
    README limitations.
  - Validate only the seams this choice affects: the adapter, the front filter,
    sync timing, alternate-screen swap, history accounting, serialization and
    derived rendering. Use the focused tests of increment 6, including one cell
    carrying many zero-width marks that still restores and serializes
    correctly. Do not restart engine selection or exhaustive upstream testing.
- `climon2-daemon` depends on `climon2-vt`; no other shipped crate does.
- Pure, shared contracts go in `climon2-core`:
  - config parsing (`config.rs`) and key vocabulary;
  - selector parsing and matching (`selector.rs`);
  - the startup envelope (`startup_envelope.rs` or similar);
  - hello extensions, capabilities and state;
  - the title-encoding helper, if shared;
  - new protocol messages.
- Shipped crates must not depend on test-only crates, except as
  dev-dependencies.
- **In the same commit that creates the crate**:
  - Add `crates/climon2-vt/src` to `SHIPPED` in `xtask/src/quality.rs`.
    Confirm that coverage, crapper and mutator see it. Re-run
    `cargo xtask tooling-probe` if selection logic is touched.
  - In `xtask/tests/architecture.rs`:
    - add `climon2-vt` to `PACKAGES`;
    - give `climon2-vt` no allowed internal dependencies;
    - add `climon2-vt` to `climon2-daemon`'s allowed internal dependencies.
  - Amend the crate-layout ADR (`docs/adr/0004-crate-layout-and-dependency-direction.md`)
    with a `climon2-vt` row (shipped lib, crates.io only) and the new daemon
    edge.
  - Never weaken either guard.
- Daemon binary packaging is unchanged: `climon2-vt` is a library and needs no
  packaged binary.

### 3.2 The virtual terminal adapter (`climon2-vt`)

Suggested public surface (adapt as needed; keep it small):

```rust
pub struct Geometry { pub cols: u16, pub rows: u16 }          // already clamped
pub struct VtConfig { pub max_history_rows: u32, pub default_title: String }
pub enum Mode { Direct, Derived }
pub struct Presentation { /* host size, mode, alternate, sync_open, derived cache */ }
pub struct Restoration { /* period, history view, captured suffix, stage */ }
pub struct Vt { /* Term, Processor, front filter, events, title, history accounting, sync latch */ }

impl Vt {
    pub fn new(geometry: Geometry, config: VtConfig) -> Self;
    /// Feed PTY output: section 1.9 policy applied. Fed { live, replies, dropped }.
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Fed;
    pub fn resize(&mut self, geometry: Geometry);
    pub fn sync_pending(&self, now: Instant) -> bool;   // first-begin latch open and not expired
    pub fn tick(&mut self, now: Instant) -> Fed;         // expires the latch
    /// Snapshot at S: returns phase A bytes and the restoration (phases B and C).
    pub fn grant(&mut self, period: u64, presentation: &mut Presentation) -> (Vec<u8>, Restoration);
    /// Next prefix bytes, whole rows only, up to about `budget` bytes.
    pub fn produce(&mut self, restoration: &mut Restoration, budget: usize) -> Chunk;
    /// Derived live update for a Derived-mode controller whose restoration has ended.
    pub fn derive(&mut self, presentation: &mut Presentation) -> Vec<u8>;
    /// Section 1.7.1 release bytes for this presentation.
    pub fn release(presentation: &Presentation) -> Vec<u8>;
}
```

`Presentation` lives in `climon2-vt` because the prologue, release and derived
renderer read it, but the daemon owns one per attachment and updates it as
bytes are **committed** (section 3.3). Every function here is infallible.

#### Engine setup and events

- Create the engine with `Term::new(config, &dimensions, listener)` and
  `term::Config { scrolling_history: max_rows + 2 * P, kitty_keyboard: false,
  osc52: Osc52::Disabled, ..Default::default() }` (see "History accounting"
  for `P`). Implement `Dimensions` with a tiny struct.
- Events: `EventListener::send_event(&self, Event)` takes `&self`. Use a
  listener holding `Rc<RefCell<Vec<Event>>>` shared with the adapter; the daemon
  is single-threaded.
  - Drain the events after each step.
  - `PtyWrite(text)` becomes replies.
  - `Title(t)` and `ResetTitle` update the tracked title value. A live title is
    emitted only when the value changes (section 1.9, class 5).
  - `Bell` increments a bell counter used by derived presentation.
  - Ignore all other events.
- **`Term::set_options` always emits `Title` or `ResetTitle`** for the current
  title, and marks full damage. Drain and discard the events produced by your
  own `set_options` calls. Comparing title values also makes these events
  harmless.
- Grid access (Alacritty 0.26 public API):
  - `term.grid()` returns a `Grid<Cell>` with `Dimensions::{history_size,
    screen_lines, columns, topmost_line, bottommost_line}`, indexed by `Line`
    and `Column`. History rows are `Line(-1)` (newest) down to
    `Line(-history_size)` (oldest).
  - `grid.cursor` and `grid.saved_cursor` expose public `point`, `template` and
    `input_needs_wrap`.
  - Each cell has `c`, `fg`, `bg`, `flags`, `zerowidth()` and
    `underline_color()`. Relevant flags are `WRAPLINE`, `WIDE_CHAR`,
    `WIDE_CHAR_SPACER`, `LEADING_WIDE_CHAR_SPACER`, `BOLD`, `DIM`, `ITALIC`,
    the underline variants, `INVERSE`, `HIDDEN` and `STRIKEOUT`.
  - Modes come from `term.mode()` (`TermMode`, including `ALT_SCREEN`). Cursor
    style comes from `term.cursor_style()`.

#### Front filter, tokens and engine steps

The front filter is pure, inside `climon2-vt`. It splits PTY output into
complete tokens: printable text runs (never split inside a UTF-8 sequence), C0
controls, ESC sequences, CSI sequences and control strings.

- Mirror the state machine in vte 0.15.0's `src/lib.rs` exactly:
  - OSC ends on BEL or ST (`ESC \`);
  - DCS, SOS, PM and APC end on ST only, so BEL is payload;
  - an ESC not followed by `\` ends the string and starts a new escape;
  - CAN and SUB abort;
  - C0 controls inside CSI are executed.
- Differential-test token boundaries against vte on tricky inputs.
- Partial tokens are buffered across reads, bounded by the 4096-byte cap.
- For each complete token: drop it entirely if it exceeds the cap; otherwise
  feed it to the engine and apply the section 1.9 classification to decide what
  is appended to `live`.
- **Step rule.** Printable text and C0 controls may be coalesced into one
  engine step of at most 64 bytes. Every ESC, CSI and control-string token is
  fed as its own step; a token longer than 64 bytes is split into 64-byte
  steps, and its effect happens at its final byte. This bounds the rows one
  step can push into history (below) and isolates history-clearing and
  screen-switching tokens.
- Snapshots are taken after a completed token. A pending partial token is later
  fed and forwarded after the restoration, so host and engine agree at the
  cutover point.

#### Synchronized output

- Parameterise `Processor` with a climon2 `Timeout` implementation whose
  `pending_timeout()` always returns `false`. The engine therefore never enters
  its synchronized buffering path (vte's default handler re-arms on repeated
  begins and flushes its whole buffer in one call; see Appendix B). This is
  **zero engine buffering**. `stop_sync` is never needed.
- The front filter recognises DECSET/DECRST of private mode 2026 and drives a
  **first-begin latch** using the `now` passed to `feed` and `tick`:
  - the first begin opens the update with a deadline of 150 ms;
  - repeated or fragmented begins while it is open do not extend that deadline;
  - an explicit end or `tick` past the deadline closes it.
- `sync_pending(now)` gates the snapshot of a pending grant and every derived
  update. `tick` closes an expired latch without another PTY read.
- In Direct presentation, begin and end tokens are forwarded like other CSI
  sequences, and `Presentation.sync_open` tracks them. The daemon emits a
  synthetic `CSI ? 2026 l` on expiry, revocation, release or restoration.
  Derived presentation never sends 2026 to the host.
- Tests use an injected clock. Cover a first begin, a repeated or fragmented
  begin, an explicit end, a timeout and re-arming after close. Include a
  pending grant served through the daemon loop after the deadline with no
  further PTY input. Do not promise a hard wall-clock CPU bound.

#### History accounting *(plan choice; invariants binding)*

Alacritty does not expose how many rows a step pushed into history, and once
history is at capacity a push replaces the oldest row without changing
`history_size()`. Comparing row contents cannot detect that either: identical
rows replace each other silently. `Term::set_options` changes the real grid's
history capacity (`Grid::update_history` drops rows), so it cannot serve as
private snapshot storage. The adapter therefore counts pushes with headroom
that is never treated as retained history.

**Invariants:**

- **H1 Retention.** No snapshot includes, and no restoration or derived update
  sends, a row outside the newest `max_rows` normal-history rows at the moment
  it is captured or sent. With `max_rows` 0, no history is ever sent.
- **H2 Exact location.** A snapshot row is read from the engine line that holds
  it, or the history phase ends. A different row is never sent in its place.
- **H3 Content independence.** The eviction flag never depends on row content.
- **H4 Bounded memory.** The engine holds at most `2P` rows beyond `max_rows`,
  where `P` depends only on geometry.
- **H5 Cutover.** Live tokens completed after S follow the prefix in order,
  with nothing lost or duplicated.

**Mechanism:**

1. **Push bound.** `P` is the most rows one engine step can push into normal
   history:
   `P = max(64, rows, ceil(2 * 65535 / cols) + 1)`.
   - 64: a 64-byte text/C0 step (64 line feeds, or wraps).
   - `rows`: one scrolling CSI (`CSI n S`, `CSI 2 J`) moves at most a screen.
   - `ceil(2 * 65535 / cols) + 1`: one REP (`CSI 65535 b`) of a wide
     character.

   Derive this bound in code and prove it with a test that feeds the worst
   token of each class at the minimum and maximum geometry and asserts the
   observed pushes never exceed `P`. Recompute `P` on resize.
2. **Capacity.** The engine's history capacity is `max_rows + 2P`.
3. **Counters** in the adapter: `pushed: u64` (rows pushed into normal history,
   monotonic), `epoch: u64` (incremented whenever normal history is cleared or
   becomes unaccountable), `evicted: bool`.
4. **Before each step** with the normal screen active: read
   `s0 = history_size()`. If `capacity − s0 < P`, trim: `set_options` with
   `scrolling_history = max_rows` (dropping the oldest rows beyond `max_rows`,
   which are not retained history), then `set_options` back to
   `max_rows + 2P`. Discard the events these calls emit.
5. **After each step:**
   - If the step was a history-clearing token (`ESC c`, or a CSI with final
     `J` whose parameters include 3): `epoch += 1`; `evicted = false`.
   - Else, if the normal screen was active before and after the step: read
     `s1 = history_size()`.
     - If `s1 < s0` or `s1 == capacity`: `epoch += 1`, `evicted = true`, and
       log "history accounting lost" (content-free). This should not happen
       when `P` is correct; it keeps H2 true if it does.
     - Otherwise `pushed += s1 − s0`, and if `s1 > max_rows`,
       `evicted = true`.
   - Steps that switch screens (`CSI ? … h/l` with 47, 1047 or 1049) push no
     normal-history rows. While the alternate screen is active, normal history
     changes only by resize or `ESC c` (which returns to the normal screen and
     clears it).
6. **Resize.** Before `Term::resize`, trim to `max_rows` if the normal screen is
   active. After it, `epoch += 1` (any in-progress restoration is retired by the
   grant or repaint that follows every resize). If the normal screen is active,
   set `evicted` when `history_size()` exceeds `max_rows` or equals capacity,
   then trim. If the alternate screen is active, set `evicted` when columns or
   rows decreased, because reflow may have dropped rows that cannot be read
   without a swap. Then set the capacity for the new `P`.
7. **Snapshot at S.** `h0 = min(normal history_size(), max_rows)`. Record
   `epoch_at_s`, `pushed_at_s`, `h0` and whether `evicted` was set (for A3).
   Reading the normal grid while the alternate screen is active uses the swap
   sequence below.
8. **Locating snapshot row `k`** (0 = oldest). With
   `p = pushed − pushed_at_s`, its current age is
   `a = (h0 − 1 − k) + p` (age 0 = newest history row). It is retained iff
   `epoch == epoch_at_s` and `a < max_rows`. Its engine line is
   `Line(−1 − a)`.
9. **Production of phase B.** For the next row `k`:
   - if `epoch != epoch_at_s`, end phase B;
   - if row `k` is not retained, jump to `k' = h0 + p − max_rows`, emitting the
     notice (unless the previous emitted line was the notice); if `k' >= h0`,
     end phase B;
   - otherwise render row `k` and advance.
10. **Memory cost.** `2P` rows of `cols` cells beyond `max_rows`: at most about
    262,000 cells at 2 columns and 200,000 cells at 500x200. Record this in the
    new ADR.

#### Restoration production

`grant` takes the snapshot at S, never while `sync_pending`:

- builds phase A from the presentation and host size (section 1.8);
- records the history view (step 7 above);
- captures phases C2–C6 into the restoration's suffix buffer. This buffer is
  bounded by geometry (at most two 500x200 screens plus modes and title);
- initialises the derived cache from the captured screens when the
  presentation is Derived, so the first `derive` after `RestoreEnd` sends
  exactly what changed since S.

`produce` emits phase B rows until about `budget` bytes, then C1 if any row was
emitted, then the suffix, then reports that `RestoreEnd` may follow. It commits
**whole rows only**; a row larger than one frame spans several frames, but a
restoration is never retired in the middle of a row.

#### Reading the normal screen while the alternate screen is active

The inactive grid is private. Inside `grant` and `produce`, single-threaded and
never while `sync_pending`, use this sequence:

1. `let alt = term.grid().clone();` This is bounded, because the alternate
   screen has no history.
2. `term.swap_alt();` While in alt mode this only swaps; it does not reset.
   The normal grid, with history, is now `term.grid()`. Read its screen,
   cursor and history rows.
3. `term.swap_alt();` This switches back. It resets the alternate grid,
   overwrites its cursor **and sets the normal grid's
   `saved_cursor = cursor`** (Appendix B).
4. `*term.grid_mut() = alt;` This puts the alternate grid back exactly.

Batch history rows per `produce` call. No pushes reach normal history while the
alternate screen is active, so ages computed in step 8 are stable across the
swap. Repeated swaps re-apply E1 with the same, unchanged normal cursor, so
they are idempotent.

Tests must state exactly what is preserved:

- The alternate screen's content, cursor and saved cursor are unchanged.
- After the application later leaves the alternate screen, the normal content,
  history, current cursor and keyboard modes are unchanged.
- The normal **saved cursor** equals the normal current cursor at restoration
  time (E1). Assert this exact value; do **not** assert that the saved cursor
  is unchanged.
- Include a case with a hidden resize where E1 differs from the previous saved
  cursor, so that the test distinguishes E1 from unintended corruption.

#### Derived presentation *(plan choice; invariants binding)*

Used only when the host is larger than the session (section 1.6).

**Invariants:**

- After each derived update, the host's session rectangle shows the engine's
  active screen and cursor, host cells outside the rectangle are blank, and
  input-affecting modes match the engine.
- Rows the engine pushes into retained history reach the host's native
  scrollback oldest first, subject to H1; a skipped run is marked with the
  notice.
- Bells and title changes reach the host.
- Output per update is bounded by the rows changed plus the rows pushed.

**State** in `Presentation` for Derived mode: the last emitted rectangle rows
(cells with rendition and wrap flag), `pushed_seen`, `epoch_seen`, the last
emitted input/display modes, the last emitted title and `bells_seen`.

**`derive`**, called once per loop iteration for a Derived controller whose
`RestoreEnd` is committed, and never while `sync_pending`:

1. If `epoch != epoch_seen`: mark a full repaint and set `pushed_seen = pushed`.
2. If `Presentation.alternate` is set but the engine's active screen is normal:
   emit `CSI ? 1049 l`, clear it and mark a full repaint.
3. If `n = pushed − pushed_seen > 0`: send the pushed rows that are still
   retained, oldest first (ages `min(n, max_rows) − 1` down to 0), while the
   host is on its normal screen. Read them through the swap sequence if the
   engine's alternate screen is now active. If `n > max_rows` and
   `max_rows > 0`, emit the notice first. Send them in chunks of at most
   `host rows`: paint the chunk at host rows `1..=len` (`CSI <r> ; 1 H`,
   `CSI 0 m`, row, `CSI 0 m`, `CSI K`), then `CSI <host rows> ; 1 H` and `len`
   line feeds. Then mark a full repaint and set `pushed_seen = pushed`.
4. If the engine's active screen is alternate and `Presentation.alternate` is
   not set: emit `CSI ? 1049 h`, set it and mark a full repaint.
5. Repaint each session row that differs from the cache (or every row on a full
   repaint): `CSI <r> ; 1 H`, `CSI 0 m`, the row with trailing default blanks
   trimmed, `CSI 0 m`, `CSI K`. On a full repaint, also blank the host below
   the rectangle: `CSI <session rows + 1> ; 1 H` then `CSI J`, when the host is
   taller. Update the cache.
6. Position the cursor with `CSI <row> ; <col> H` (no pending-wrap re-print).
7. Emit DECSET/DECRST for input/display modes that changed: 25, 1, keypad, 2004,
   1000/1002/1003, 1006/1005, 1004, 1007 and DECSCUSR. Never change 6, 7, 4 or
   20 in Derived mode.
8. If the title value changed, emit the encoded title.
9. If `bells > bells_seen`, emit one BEL.

**Mode transitions on resize after readiness:**

- Direct to Derived: emit the prologue's baseline (without `CSI ? 1049 l`; a
  synthetic `CSI ? 2026 l` if `sync_open`), reset the cache and run a full
  repaint.
- Derived to Derived at a new size: reset the cache and run a full repaint.
- Derived to Direct: run a full repaint, then emit modes 6, 7, 4 and 20 to
  match the engine; Direct forwarding resumes after it. The application's
  scroll region is not restored (limitation).

#### Serializer

The serializer is a set of pure functions over a read-only grid view, shared by
phases B and C and by derived updates.

- Build SGR from cell attributes:
  - named colours map to 30–37, 90–97, 40–47, 100–107, 39 and 49;
  - indexed colours map to `38;5;n`;
  - RGB colours map to `38;2;r;g;b`;
  - underline colour maps to `58:…`.
- Emit `ESC [ 0 m` at the start of each row.
- Skip wide-character spacers and append zero-width characters.
- Trim trailing default blank cells on hard-ended rows. Print soft-wrapped rows
  at full width (Direct phase B only).
- Position screen rows absolutely in the session rectangle.
- Encode titles as in section 1.8.1.
- Keep functions tiny. Table-drive SGR and mode emission so that CRAP stays at
  5 or below.

### 3.3 Daemon structure

**One writer.** The existing single-threaded `Session` loop owns the PTY, the
`Vt`, all connections, a pure **control state machine** and the per-attachment
`Presentation` and delivery records. No other thread touches them.

**Control state machine** (a new pure module, for example
`crates/climon2-daemon/src/control.rs`). Inputs are events and outputs are
effect lists. It owns:

- lifecycle and attachments;
- the controller and the hand-back recency order;
- the control period and readiness;
- each client's latest pending size and the applied session size;
- the application-paste cleanup obligation;
- shutdown state.

```rust
enum Input { Attach{conn, size}, TakeControl{conn}, Ready{conn, period},
             Resize{conn, size}, Detach{conn}, Lost{conn}, Kill{conn},
             InitialDeadline, RestoreDeadline{conn, period}, CommandExited,
             AuthoritativeFailure }
enum Effect { Grant{conn, period}, Release{conn}, Revoke{conn},
              ResizeSession(size), AcceptDetach{conn}, Refuse{conn, reason},
              EnableInput{conn}, DiscardPendingInput{conn}, ClosePaste,
              BeginShutdown, DeliverExit, DeliverError, ... }
```

- Every loss of input admission emits `DiscardPendingInput` and, when
  applicable, `ClosePaste` (section 1.5).
- Every daemon-confirmed end of presentation emits `Release` before
  `Revoke`, `AcceptDetach`, `DeliverExit` or `DeliverError` (section 1.7.1).
- `Resize` only records the attachment's pending size. The loop applies the
  controller's pending size once per iteration (section 1.6).

The rules of sections 1.5–1.7 and 1.18 live here and nowhere else. Expose
`snapshot() -> State` for hello. Unit-test the module exhaustively; it is the
easiest place to kill mutants.

**`Presentation` (per attachment).** The record of what this attachment's
stream has put its host into: host size, mode (Direct/Derived), `alternate`,
`sync_open` and the derived cache.

- It is updated as bytes are **committed** to the attachment's outgoing queue
  (below), never when they are merely produced or held back:
  - phase A clears `alternate` and `sync_open`; C4 sets `alternate`;
  - committed live `CSI ? 1049/1047/47 h/l` set and clear `alternate`;
  - committed live `CSI ? 2026 h/l` and the synthetic end set and clear
    `sync_open`;
  - derived updates maintain `alternate` and the cache.
- It is used for the replacement-restoration prologue and for
  `Vt::release(presentation)`.
- Release is the section 1.7.1 baseline, plus `CSI ? 1049 l` only when
  `alternate` is set.
- This is not a client emulator.

**Delivery per attachment and period retirement.**

- `outgoing`: committed frames, strictly FIFO to the socket (the existing
  connection queue). Once committed, a frame is never removed, and a partially
  written frame always completes.
- `restoration`: the in-progress `Restoration` producer, if any.
- `backlog`: Direct-mode live tokens completed after S, waiting until
  `RestoreEnd` is committed. Not yet committed, so not yet reflected in
  `Presentation`.
- The 16 MiB live queue counts `backlog` plus committed live and derived bytes.

A period is **retired** by a replacement grant to the same attachment (a
pre-readiness resize), by revocation, or by a release:

1. Committed bytes stay, in order. `Presentation` already reflects them.
2. The old producer stops; its uncommitted history and suffix are dropped.
   Because `produce` commits whole rows, the host is never left mid-sequence.
3. The old `backlog` is dropped. The new snapshot already includes its effects;
   for a release, the session's content no longer matters to this host.
4. The new phase A prologue (or the release) is computed from `Presentation`
   after all committed bytes, and committed after them.

The client may see `Granted{n}`, part of a prefix, then `Granted{n+1}`. It writes
everything in order and sends `Ready` only for the current period.

**Paste cleanup.** A pure tracker runs over input bytes **as actually written**
to the PTY, so it sees partial writes. It records `open` and any partially
written marker.

- `ClosePaste` computes the cleanup bytes (section 1.5) when engine mode 2004 is
  set.
- Those bytes become a PTY-write prefix that must be fully written before any
  later admitted input.

**Loop changes** (`session.rs`). Each iteration:

1. Accept connections and read all available client frames into the state
   machine. Resizes only update pending sizes.
2. Apply the controller's latest pending size, once: `pty.resize` and
   `vt.resize`. For a Ready controller, a resize involving Derived mode
   enqueues the derived repaint of section 3.2.
3. Read the PTY, regardless of clients, bounded per iteration (for example one
   32 KiB read). Feed the `Vt`; write replies to the PTY.
4. Call `vt.tick(now)`.
5. Route output for the controller:
   - Direct: if a restoration is in progress, append `live` to `backlog`;
     otherwise commit it.
   - Derived: once `RestoreEnd` is committed, commit `vt.derive(presentation)`
     (empty while `sync_pending`). No backlog is needed: the cache was
     initialised at S.
6. For each **grant** effect, when `!vt.sync_pending(now)` (bounded by the
   latch): retire the previous period if any, call `vt.grant`, then commit
   `Granted{period}` and phase A.
7. Refill restoration `Output` frames (each at most 65,536 bytes) up to the
   1 MiB window. When production ends, commit `RestoreEnd{period}`, then move
   `backlog` to `outgoing`.
8. For each **release** effect: retire the period, commit
   `Vt::release(presentation)` as `Output`, then the terminal message. Stop
   output to that attachment.
9. Write input to the PTY: pending paste-cleanup bytes first, then input from
   the Ready controller of the current period only. `DiscardPendingInput` drops
   that attachment's unwritten input.
10. Flush sockets. Enforce the 16 MiB live queue and the restoration deadline
    (section 1.10) in `connection.rs`, tracking socket write progress
    separately from receipt of `Ready`. Violations become `Lost{conn}` or
    `RestoreDeadline`.

- **Initial deadline**: 5 s from readiness while `starting` raises
  `InitialDeadline`.
- **Authoritative failures** (section 1.18) raise `AuthoritativeFailure`
  instead of propagating out of `serve`.
- Hello replies include `capabilities` and `state`.

**Shutdown needs restructuring, not just reuse.**

- Today `serve` in `crates/climon2-daemon/src/lib.rs` returns from
  `session::serve`, dropping every connection, and only then calls
  `Resources::cleanup`. `Pty::leave` also discards the command's exit status
  and blocks the loop with `thread::sleep(HANGUP_GRACE)`.
- Change it as follows:
  1. Make the hangup grace a loop deadline. The loop keeps answering hello
     with `terminating` and refusing attaches.
  2. Capture the actual exit status from hangup or kill.
  3. Have the session perform `Resources::cleanup` itself **while established
     connections stay open**. Sending on an accepted Unix stream after
     unlinking the socket path is valid.
  4. Only then commit each attachment's release (section 1.7.1), then
     `Exit{code}` (or `Error` after an authoritative failure) to attachments
     and `Terminated{code}` to kill requesters. Never send `Terminated` if
     cleanup failed.
  5. Flush all connections non-blockingly under one shared short deadline,
     prioritising kill requesters, then exit.
- Keep the existing normal-exit drain.

`args.rs`: make `--scrollback-max-rows` required, and build the default title
from the argv after `--`.

### 3.4 Protocol additions (version-private, `climon2-core/src/protocol.rs`)

Keep the existing codes and framing. Extend the frame-type validation match and
the data-length rule.

| Code | Direction | Body | Meaning |
|---|---|---|---|
| `0x10` Attach | C->D | `{"cols","rows"}` | attach and take control (existing code, new semantics) |
| `0x11` Input | C->D | raw | application input (applied only if ready controller) |
| `0x12` Resize | C->D | `{"cols","rows"}` | pending size; applied only for controller |
| `0x13` TakeControl | C->D | `{}` | takeover request |
| `0x14` Ready | C->D | `{"period":u64}` | restoration written; enable input |
| `0x15` Detach | C->D | `{}` | explicit detach request |
| `0x16` Kill | C->D | `{}` | kill-mode request (instead of Attach) |
| `0x20` Attached | D->C | `{}` | attachment accepted (non-controlling until Granted) |
| `0x21` Output | D->C | raw | restoration, release, live or derived bytes |
| `0x22` Exit | D->C | `{"code":u32}` | command finished (after release) |
| `0x23` Granted | D->C | `{"period":u64}` | this client controls; restoration follows |
| `0x24` RestoreEnd | D->C | `{"period":u64}` | restoration complete |
| `0x25` Revoked | D->C | `{}` | lost control (after release); show placeholder |
| `0x26` Detached | D->C | `{}` | detach accepted (after release) |
| `0x27` Terminated | D->C | `{"code":u32}` | kill confirmed after cleanup |
| `0x7f` Error | both | `{"message"}` | refusal, protocol error or authoritative failure |

Update `ConnectionState`:

- After hello, a connection may send `Attach` or `Kill`, but not both.
- `TakeControl`, `Ready`, `Detach`, `Input` and `Resize` require an accepted
  attachment.

Readiness:

- The client sends `Ready{period}` only after writing every byte up to
  `RestoreEnd{period}` to stdout. A stale `Ready` for an old period is ignored.
- Per-connection FIFO ordering plus `Ready` rejects stale socket input without
  tagging every input frame.
- Keystrokes typed during restoration can still sit in the local terminal's
  input queue. When it is about to send `Ready`, the client therefore:
  1. drains all **immediately readable** stdin through its input filter in
     discard mode;
  2. resets the key state (section 1.4);
  3. sends `Ready` and starts forwarding.
- The drain replaces `tcflush`. A kernel flush could discard a paste's closing
  marker and leave the local paste state wrong. Reading through the filter
  keeps the marker tracking accurate.
- If the filter is still inside a paste, its remainder is discarded until the
  closing marker (section 1.4).

### 3.5 Client structure

- **Startup**:
  1. Parse the envelope, or load config if the envelope is absent, before raw
     mode.
  2. Open this invocation's own log (section 1.16).
  3. Run hello and validate identity, version, protocol **and** capabilities.
  4. Send `Attach` or `Kill`.
- **Input filter** (pure, `keys.rs`):
  - State is `KeyState` {escape pending, partial header} plus `PasteState`
    {in paste, partial marker buffer} (section 1.4).
  - `feed(bytes, role) -> Vec<Action>`, where the role is Controlling-ready,
    Pending or NonControlling.
  - Actions are `Forward(bytes)`, `Detach` and `TakeControl`.
  - Pending and NonControlling discard application bytes.
  - Only `KeyState` is reset on role changes.
- **Client state** (pure: role, current period and closing deadline), driven by
  server messages:
  - `Granted` → Pending for that period.
  - After stdout has drained past `RestoreEnd` of the current period: run the
    readiness sequence (section 3.4), send `Ready` and become Controlling.
  - `Revoked`: after writing the preceding release bytes, draw the placeholder
    and become NonControlling.
- **Bridge observation** (section 1.7.1):
  - Keep the socket descriptor in the poll set at all times; request `POLLIN`
    only when stdout has nothing pending and `POLLOUT` only when there is
    outgoing data. Treat `POLLHUP`/`POLLERR` as connection loss even while
    stdout is blocked.
  - Start the 5-second closing deadline at the earliest trigger, and service it
    from the poll timeout, independently of stdout progress.
  - Never read further frames while stdout has pending bytes.
- **Placeholder drawing**:
  1. Move to the bottom row and emit `rows` line feeds, preserving visible
     content.
  2. Emit `CSI H`, then the two placeholder lines.
  3. Redraw on SIGWINCH while non-controlling.
- **Exit ordering** (section 1.7.1): on `Detached`, `Exit`, `Error` after
  output, or `Terminated`:
  1. All preceding `Output`, including release bytes, has been written (frames
     are processed in order).
  2. End the placeholder if one is shown.
  3. Restore termios exactly.
  4. Print the stderr line or diagnostic, then exit with the status from
     section 1.1.

  On unexpected loss, write the conservative reset of section 1.7.1 instead.
  On closing-deadline expiry, follow section 1.7.1.
- **Detach**: send `Detach`, which starts the closing deadline, and wait for
  `Detached`. Then run the exit ordering and exit 0. Without confirmation,
  exit 125.
- **Kill mode**: no raw mode. Send `Kill` and wait.
  - `Terminated{code}` → exit 0.
  - `Error` → exit 125.
  - Connection loss → exit 125.

### 3.6 CLI and environment

- `cli.rs`: add `Attach { selector }` and `Kill { selector }`. Parse only the
  selector's shape; a malformed or missing selector is a usage error (2).
- New `attach.rs` and `kill.rs` share a `dispatch.rs`:
  - Discover by reusing `list::discover`, extended to return parsed hello
    extensions.
  - Select the target, then check capabilities and the client path.
  - Attach: TTY check, self-attach check, `config` client snapshot, then `exec`
    the client with the envelope.
  - Kill: spawn the client with the kill envelope (stdin, stdout and stderr
    suitable for a non-TTY). Wait up to the 10-second deadline; on expiry, kill
    and reap only that child.
- `exec.rs`:
  - load config (client and scrollback) first;
  - pass `--scrollback-max-rows`;
  - strip `CLIMON2_CLIENT_STARTUP` from the daemon and application environment
    (`SessionEnvironment` projection);
  - set the attach envelope when exec'ing the client.
- `climon2-core/src/session.rs`: in `SessionEnvironment::for_session`, drop
  inherited `TERM` and `COLORTERM` entries and push `TERM=xterm-256color` and
  `COLORTERM=truecolor` (section 1.17). Update the doc comment. Tests that
  encode the old behaviour and must change:
  - `crates/climon2-core/tests/inputs.rs` (an inherited empty `TERM` is kept
    today);
  - `crates/climon2-cli/tests/exec.rs` (the "absent TERM" whole-environment
    test and the nested-bash `TERM` inheritance test).

  Search for `TERM` in tests to find any others.
- `list.rs`: add the VERSION, STATE and CLIENTS columns and the JSON `state`.

## 4. Increments

Each increment follows the same cycle: red test, green, refactor, gates,
commit, push.

- Run `cargo xtask test` at least once per increment, and run the quality gate
  on changed shipped source (section 7) before committing.
- Every commit passes every existing test. Existing `exec` behaviour must never
  regress at any commit. This is why the library (increment 6) is complete
  before the daemon switches to the new pump (increment 8): once the daemon
  reads the PTY continuously, an application's output before the first attach
  can only reach the host through restoration, so restoration must already
  cover rendition, the alternate screen, cursor, modes and title.
- An increment may take several commits; each must pass. Do not add product
  flags or scaffolding solely for staging. The final acceptance (section 6) and
  the gates are unchanged.

1. **Baseline.**
   - Confirm the architecture (`uname -m`).
   - Select bash 5.x via `CLIMON2_TEST_BASH`, for example
     `/usr/local/bin/bash` or Homebrew bash.
   - Run `cargo xtask test`. Record pre-existing failures and environment gaps
     in the action log.
   - Commit nothing unless something is fixed.
2. **Bash 5-only interactive qualification.**
   - In `climon2-test-term/src/shell.rs`, accept only GNU bash 5.x. Probe
     `CLIMON2_TEST_BASH`, then `/opt/homebrew/bin/bash`, then
     `/usr/local/bin/bash`, but not `/bin/bash`.
   - Update `climon2-fixtures/tests/bash.rs` and the packaged tests that
     iterate `bash_profiles()`.
   - Update the CI matrices: drop interactive `3.2` from both the `macos` and
     the `archive` jobs. **Also select a controlled bash 5 for the `quality`
     job**, replacing `CLIMON2_TEST_BASH: /bin/bash` in
     `.github/workflows/check.yml`. Use the same controlled-shell policy as the
     ordinary test jobs.
   - Update `README.md` and `crates/climon2-e2e/README.md`.
   - Keep `install.sh` and any installer or non-interactive `/bin/bash` uses
     unchanged, and keep them tested. Do not weaken any guard to make this
     migration green.
3. **Per-invocation client logs.** Implement section 1.16 and update the
   dependent assertions.
   - Red: start a client, then start a second client on the same session log
     directory. Today this fails on `create_new`.
   - Green: every invocation gets its own file, with the existing permissions,
     privacy and suffix rules.
4. **Core contracts (pure) and the command-facing environment.**
   - `config.rs`: a lexical normaliser that respects strings and escapes. It
     (a) replaces each comment with whitespace, preserving token separation,
     and (b) removes a comma whose next non-whitespace, non-comment character
     is `}` or `]`.
   - Then deserialize with `serde_json` into **operation-specific** structs:
     `ExecConfig { client, scrollback }` and `AttachConfig { client }`.
     - The attach struct does not name `scrollback`, so serde ignores it as
       unknown. An invalid, unused value therefore cannot fail attach.
     - serde derive rejects duplicate known fields. Use a three-state field type
       to distinguish missing, `null` and a value.
   - Tests: the exact sample in section 1.3 (comments and trailing commas);
     comments between otherwise valid tokens; `//` and `/*` inside strings
     left intact; rejection of `{"scrollback":{"max_rows":1/* c */0000}}` and
     of `tr/* c */ue` in a value position, both as malformed JSONC; an invalid
     `scrollback` during attach.
   - Then add the key vocabulary, selector parsing and matching, and the startup
     envelope.
   - Add hello extensions with lenient optional parsing: parse the mandatory
     `HelloReply` as today, and parse the optional fields separately from a
     `serde_json::Value`.
   - Add the state consistency rules and title encoding.
   - Write exhaustive unit tests, including the exact 4096-byte envelope
     boundary.
   - Command-facing environment (section 1.17): red test through public
     `exec` with conflicting inherited `TERM` and `COLORTERM`; the fixture must
     observe `xterm-256color` and `truecolor`, while unrelated inherited
     variables and the enclosing-session behaviour stay intact. Update the
     tests listed in section 3.6.
5. **Protocol messages.** Add the new codes and `ConnectionState` rules, with
   framing tests.
6. **`climon2-vt`, complete.**
   - First commit: create the crate and register it with the gates:
     - add it to `SHIPPED`;
     - update the architecture `PACKAGES` and dependency-edge guards;
     - amend the crate-layout ADR (0004);
     - add the new ADR (section 8).
   - Then build, test-first, in passing commits:
     - the tokenizer: split-across-reads cases, the cap, CAN/SUB, UTF-8
       boundaries and differential boundaries against vte;
     - the step rule;
     - the section 1.9 classification, including a supported query split across
       reads, unsupported requests, OSC 52, control-bearing titles and links,
       and a CPR-shaped sequence in output;
     - engine feed and replies; title tracking that ignores `set_options`
       events;
     - the first-begin sync latch with an injected clock;
     - history accounting: the push-bound test, the eviction flag with
       identical rows, clear handling, resize handling, trimming;
     - the serializer: SGR and colours, wide characters and combining marks
       (including many marks on one cell), pending wrap;
     - three-phase restoration: phase order, phase B moving view with gap
       notices and early end, the alternate screen with exact E1 assertions,
       modes, title;
     - derived presentation, including mode transitions;
     - release bytes.
   - The test oracle is described in section 5.2.
7. **Daemon state machine (pure).** Exhaustively test sections 1.5–1.7 and
   1.18, including:
   - `DiscardPendingInput` and `ClosePaste` on every loss of admission;
   - `Release` before every `Revoke`, `AcceptDetach`, `DeliverExit` and
     `DeliverError`;
   - `RestoreDeadline`;
   - replacement grants before readiness;
   - resizes recorded as pending, never applied directly.
8. **Integration: the new pump for `exec`, attach and detach.**
   - **Daemon**: continuous PTY reading, VT feeding and replies, resize
     coalescing, grants with incremental restoration and period retirement,
     Direct and Derived delivery, `Ready` gating, detach with release and
     `Detached`, exit with release and `Exit`, unexpected final loss, the
     initial deadline, the restoration deadline, authoritative-failure
     handling, the required `--scrollback-max-rows` and hello extensions.
   - **Client**: envelope or config startup, the detach key, roles, the
     readiness drain, bridge observation with the closing deadline, and the
     exit ordering.
   - **CLI**: `attach` with selectors, capability and client-path checks,
     self-attach and the envelope.
   - **Existing-exec preservation test, in this increment's first integration
     commit**: hold the initial attachment until the TUI fixture has entered
     the alternate screen, drawn its display, enabled its input modes and is
     waiting for input. Initial `exec` must then show that display and apply
     those modes without any further application redraw (section 6.2).
   - Test through the real daemon binary with raw-socket fake clients, in the
     style of `crates/climon2-daemon/tests/lifecycle.rs`, and through
     `crates/climon2-client/tests/bridge.rs`.
   - Add a packaged detach/reattach smoke test.
9. **Multi-client control.**
   - Takeover, hand-back and the placeholder.
   - Release at revocation, with no `1049 l` for placeholder clients.
   - Sizing across controllers.
   - Stale-period input.
   - The paste cleanup tracker under partial PTY writes.
   - Client paste-state persistence across role changes.
10. **Kill, shutdown, listing and the remaining CLI.**
    - Kill mode and the 10 s CLI deadline.
    - Shutdown restructuring.
    - `list` columns and JSON.
    - Config scope per operation.
    - Envelope overwrite, and its exclusion from the daemon and application
      environment.
    - Simulate an older session with no capabilities, using a fake hello server
      or a controlled-version install.
    - A deterministic config-edit race: edit the file after validation but
      before the client starts. The client must use the old snapshot, and the
      daemon must keep its retention.
11. **Packaged product tests** (section 6).
12. **Docs, native checks, full gates, push** (sections 8–10).

## 5. Test harness and test design

### 5.1 Harness changes

- **`climon2-test-term`**:
  - Add an option to **disable automatic `PtyWrite` replies**, for example
    `Terminal::spawn_with(…, Replies::Off)`, so tests prove that the daemon
    answers queries and that the host is not recruited.
  - Add access to the emulated window title by tracking `Event::Title` and
    `ResetTitle`.
  - Add access to scrollback text (history rows).
  - Add an **occupied-host** helper that fills the outer terminal with known
    content before attaching.
  - Add access to the outer terminal's mode state (alternate screen, cursor
    visibility, mouse, bracketed paste, application cursor keys) for restoration
    and release assertions.
  - Allow host sizes above 500 columns and 200 rows, for Derived presentation.
  - Record whether the host was asked any query or clipboard operation.
- **Client test hook** *(plan choice)*: `CLIMON2_CLIENT_FAULT=hold-attach`
  with `CLIMON2_CLIENT_HOLD=<path>`, `test-hooks` only. Before sending `Attach`,
  the client waits (at most 4 s, inside the daemon's 5 s initial deadline) for
  that file to exist.
- **Daemon test hook** *(plan choice)*: `CLIMON2_DAEMON_FAULT=pty-write`,
  `test-hooks` only, makes the next PTY write fail so the authoritative-failure
  path can be tested.
- **Fixtures**: extend `climon2-fixture` and `climon2-tui-fixture` only as
  needed. Possible modes:
  - send DA1/DSR and print the reply received, with a timeout;
  - wait for an acknowledgement file or byte before printing, to produce output
    while detached deterministically;
  - write a large volume (tens of MiB) and report completion;
  - write numbered or identical lines on demand;
  - set a title, including control bytes;
  - emit an unfinished `ESC [` or OSC and then wait for input;
  - emit numbered output continuously across a restoration;
  - enable mouse, hidden cursor, alternate screen, application cursor keys and
    bracketed paste, then exit without undoing them;
  - draw an alternate-screen display once, write an acknowledgement file, then
    wait for input and report how many times it drew;
  - report `TERM`, `COLORTERM` and the PTY size.
- Use acknowledgements and observable completion for ordering. Sleeps are never
  proof of readiness.

### 5.2 Test-design rules

- A roundtrip check (feed the source into `Vt`, feed the prefix into a
  **fresh** Alacritty `Term`, compare visible text, rendition, cursor, history
  rows and wrap flags) is a compatibility check, not the oracle. A fresh `Term`
  as a test dependency of `climon2-vt` is fine.
- Add **literal expected** assertions for climon2's own renderer and transport:
  - exact expected screen text and positions;
  - expected input admission and ordering;
  - exact expected prefix bytes for small cases, including a normal-only case
    whose literal bytes show phase A, phase B rows, C1, normal-screen rows and
    the suffix in order, so a missing phase cannot hide behind a roundtrip.

  They must fail if restoration is omitted, a phase is missing or misordered,
  the wrong normal/alternate screen is used, a live suffix is lost or stale
  input is enabled. Do not build a second complete emulator or an exhaustive
  rendition suite.
- Exercise an **occupied host** and **two restorations on one attachment**: a
  pre-readiness resize, and a takeover followed by a hand-back.
- Use **numbered fixture phases with fresh acknowledgements**. Each phase prints
  a marker unique to that phase, and each wait looks for that phase's marker
  only after the phase begins. Accepted duplicate history means a marker that
  is already present in native scrollback must never satisfy a new phase.
- History tests use **literal numbered rows**, not only final screen text.

## 6. Product acceptance

### 6.1 Packaged scenarios (`crates/climon2-e2e/tests/packaged.rs`)

The full chain runs: installer, launcher, CLI, daemon, client. Use real,
controlled bash 5 outer terminals and isolated homes. Each test asserts actual
exit statuses and ends with `assert_clean()`: no owned processes, sockets or
metadata are left behind.

1. **Detach and reattach.**
   - Produce shell output and history before detaching.
   - Ctrl+] d exits 0, with the stderr line and `climon2 attach <id>`.
   - Produce output while detached.
   - `list --json` shows `detached` with 0 clients.
   - Attach by an 8-hex prefix. History, the detached output and the prompt are
     restored, and typing works.
   - `exit 7` returns 7 to the attached client.
   - Run it on an occupied host. Then detach and reattach a second time, and
     verify that each invocation has its own client log.
2. **Alternate screen.**
   - Run the TUI fixture (or `vim`/`less`) on the alternate screen, detach, and
     reattach. The alternate content is shown.
   - Quitting the TUI shows the restored normal content.
   - Resize before `Ready` so that a replacement restoration runs; after the
     TUI exits, the current normal content and cursor-addressed output are
     correct, and restored history is in the host's normal scrollback, not
     lost to the alternate buffer.
3. **Multi-client.**
   - A execs and B attaches. A shows the placeholder with
     `Press Ctrl+T to take control`.
   - A takes control with Ctrl+T. B's input is discarded while it is
     non-controlling.
   - B detaches and A continues.
   - The controller's size drives the session; the fixture reports the PTY
     size.
   - Command exit returns the status to every attached client.
   - Logs: start A, attach B, detach B, reattach C, then kill. Each invocation
     reaches its operation and has an independently usable log.
4. **Unexpected final loss.** Kill the only client process. The session
   terminates and cleans up. Adapt the existing
   `killed_installed_client_uses_daemon_leave_policy…` test.
5. **Kill.**
   - A detached session is killed silently with 0.
   - An attached session is killed while a client is attached; the client
     receives the signal-derived status.
   - Unknown and ambiguous selectors exit 125; a malformed selector exits 2.
6. **Self-attach.** Inside a session, `climon2 attach $CLIMON2_SESSION_ID`
   exits 125. The session and the controller are undisturbed.
7. **Configuration.**
   - An invalid config makes `exec` exit 125 before any session exists.
   - A custom escape prefix works.
   - `take_control_key: null` hides the instruction.
   - A small `max_rows` shows the truncation notice on reattach.
   - `max_rows: 0` restores the screen without history.
8. **Query authority and output policy.** Disable harness replies.
   - A fixture's DA1/DSR receives exactly one reply, both attached and while
     detached, including when split across reads.
   - An unsupported host-dependent request and OSC 52 store/read cause no host
     query or clipboard effect.
   - A legitimate CPR-shaped key sequence typed by the user reaches the
     application.
9. **Title and links.**
   - The default `climon2 exec <cmd> <args>` title is visible in the harness
     after attach.
   - An application title containing a control byte arrives percent-encoded,
     both live and after restoration.
   - A control-bearing OSC 8 link arrives as plain text, live and restored.
10. **Detached throughput.** A large write completes while detached, and the
    session stays live.
11. **Release.** Use the TUI fixture with mouse reporting, a hidden cursor,
    the alternate screen and bracketed paste enabled:
    - detach;
    - natural exit, where the fixture exits without undoing its modes;
    - kill;
    - a non-controlling departure.

    After each client returns, verify ordinary shell input, the documented
    baseline (section 1.7.1) and termios restoration.
12. **Interrupted paste.**
    - Interrupt a fragmented bracketed paste by takeover and by client socket
      loss. Place both client-action bindings (Ctrl+] `d` and Ctrl+T) in the
      remaining payload.
    - Assert no accidental action and no stale payload.
    - Assert exactly one ordered application-paste closure when one is needed,
      and normal interpretation of B's subsequent input.
    - Also change readiness while a local paste is still streaming.
13. **Large restoration.**
    - A plain-text history producing a prefix larger than 16 MiB reaches a
      healthy reader completely and in order.
    - Numbered output produced during restoration shows no lost suffix and no
      duplicated live span.
14. **Oversized hosts (Derived presentation).** Use a 501x24 host and an 80x201
    host. Reach readiness, then:
    - print 500 narrow characters then one more: the extra character appears at
      host row 2, column 1 (the session's wrap), not column 501;
    - with the cursor on session row 200 of the tall host, send LF plus text:
      the text appears on host row 200, host row 201 stays blank, and the row
      that scrolled off is in the host's native scrollback;
    - subsequent application cursor positioning lands in the session
      rectangle;
    - assert literal host rows and columns, not only marker presence.
15. **Terminal identity.** Start `exec` with conflicting inherited `TERM` and
    `COLORTERM`. The fixture reports `xterm-256color` and `truecolor`;
    unrelated inherited variables are unchanged.
16. **Existing tests keep passing**: vim, less, TUI queries/mouse/paste/resize,
    raw-socket abuse, version routing, log privacy and stale cleanup.
17. **Previous-release compatibility.** The existing
    `actual_previous_release_remains_live_across_current_archive_install` gate
    stays. No release exists yet, so it is a no-op locally. Never fabricate a
    previous archive. Synthetic controlled versions cover the
    unsupported-operation paths.

### 6.2 Daemon integration tests with raw-socket fake clients

These are required, and deterministic because they do not depend on host
timing. Assert literal bytes or literal numbered rows.

- **Existing-exec preservation**: the fixture enters the alternate screen,
  draws once, enables mouse, bracketed paste and application cursor keys,
  writes its acknowledgement and waits. Only then does a client attach (raw
  socket in the daemon test; the `hold-attach` hook through public `exec` in a
  CLI-level test). The restoration contains the drawn display and the modes;
  the host shows it; a key reaches the fixture; the fixture reports one draw.
- **Phase order**: prime an occupied host, complete an initial restoration that
  enters the alternate screen (C4 committed), then force a replacement
  restoration. The replacement's `CSI ? 1049 l` and the rest of phase A precede
  every restored history row; outer sentinels survive in the host's normal
  scrollback; normal rows and alternate rows appear in their phases. Include a
  small normal-only case with exact expected prefix bytes.
- **Retention with an unfinished prefix**: with `max_rows` 0 and then 2, start
  a restoration and leave it unfinished; feed more scrolling; take a second
  snapshot (takeover by another fake client). The second restoration contains
  no history for 0, and at most 2 rows (the newest) for 2.
- **Moving view**: between production slices, (a) push enough numbered rows
  that unsent snapshot rows leave retention, and (b) separately clear history.
  For (a), the remaining sent rows are the expected numbered rows, preceded by
  exactly one notice at the gap; for (b), phase B ends and the suffix follows.
- **Identical-row eviction**: with `max_rows` 2, emit many identical
  hard-ended lines, then attach: the notice is present. Clear history, emit one
  line, reattach: no stale notice.
- **Period retirement**: a partially sent frame, a committed alternate entry
  and a replacement grant. The partial frame completes intact; the new
  prologue starts with exactly one `CSI ? 1049 l`, after all committed bytes;
  no uncommitted bytes of the old period appear.
- **Resize burst**: a burst of resize frames in one write produces exactly one
  PTY resize (the fixture reports one SIGWINCH and the last size).
- **Unfinished token**: the command writes `ESC [` (or an unterminated OSC)
  and then waits. A fake client attaches and receives a restoration and
  `RestoreEnd` without any further command output.
- **Withheld `Ready`**: after a small prefix has been fully accepted by the
  socket, `Ready` is never sent. The restoration deadline fires as an
  unexpected departure, and the leave policy applies.
- **Stalled reader**: a client that stops reading during a large prefix hits
  the 5 s no-progress deadline. A fast, quiet client succeeds.
- **Sync latch**: with repeated begins, the grant is served at most 150 ms
  after the **first** begin, through `tick` alone.
- **Replacement restoration**: the first prefix enters the alternate screen,
  then a resize starts period+1. The new prologue begins with `CSI ? 1049 l`
  exactly once.
- **Paste cleanup under partial PTY writes**: use a slow PTY reader to force
  partial writes. The cleanup bytes complete before any later admitted input.
- **Release ordering**: byte-exact order of release bytes, then
  `Detached`/`Exit`/`Revoked`. There is no `1049 l` for an attachment that never
  entered the alternate screen.
- **Authoritative failure**: the `pty-write` daemon hook (section 5.1) produces
  release then `Error` for each attachment, cleanup, and exit 125 at the client.

### 6.3 Client bridge tests (`crates/climon2-client/tests/bridge.rs`)

- **Blocked stdout**: make the client's stdout block with output pending (a PTY
  whose master is not read), queue further `Output` and `Exit` in the fake
  daemon, then close the fake daemon connection. The client returns within the
  5 s bound plus a small margin, termios is restored, and it exits 125 with the
  unconfirmed-result diagnostic. A test that first delivers `Exit` to the
  handler does not exercise this trigger.
- **Pending detach with blocked stdout**: send the detach key while stdout is
  blocked; without `Detached`, the client exits 125 ("detach not confirmed")
  within the bound.
- **Ordinary exit**: `Output`, release bytes and `Exit` are written in order,
  then termios is restored, then the status is returned.

## 7. Quality gates and commands

- `cargo xtask test` runs fmt, strict clippy (`-D warnings`), ShellCheck and
  all tests.
- Mutation testing:
  - run `cargo xtask setup-quality` once;
  - during increments, run `CLIMON2_MUTATION_BASE=<base> cargo xtask mutate`
    for changed shipped source;
  - run `cargo xtask check` before pushing;
  - run `cargo xtask check --mutate-all` before declaring the work complete.
    It is long-running; run it in the background.
- Run `cargo xtask fixtures-check` (append-only goldens) and
  `cargo xtask release-check`.
- `cargo xtask dist` requires an arm64 macOS host. In CI, the `dist` job builds
  the universal archive on `macos-15` (arm64) only, and the `archive` job tests
  that archive on both `macos-15` and `macos-15-intel`. If the local host is
  Intel, rely on CI for `dist` and report it.
- Every gate runs under the controlled bash 5 policy, including the `quality`
  job.
- For the new shipped crate:
  - add `crates/climon2-vt/src` to `SHIPPED` and update the architecture guards
    (section 3.1);
  - confirm that crapper and mutator report its functions.

  Never weaken thresholds or guards, add exclusions or mark mutants as
  acceptable.

## 8. Documentation updates

These files are edit targets.

- **`README.md`**:
  - usage: attach, kill, keys, detach and takeover, the placeholder;
  - replace "Leaving the client ends the session; detach, reattach and screen
    restoration are not supported" with the new leave policy;
  - the configuration file (including comments-as-whitespace) and the list
    columns;
  - the command-facing `TERM=xterm-256color` and `COLORTERM=truecolor`;
  - the shell matrix: bash 5.x interactive only, with `/bin/bash` installer
    compatibility;
  - limitations:
    - unmarked paste;
    - binding conflicts with applications;
    - native duplicates on reattach;
    - unrestored state (section 1.8), including the host title on exit;
    - the E1 saved-cursor exception;
    - the live output policy: stripped host queries, OSC 52, palette and other
      OSC, and plain-text hyperlinks;
    - hosts larger than 500x200 use derived output; the scroll region is not
      carried across a Derived/Direct switch; hosts narrower than 2 columns are
      unsupported;
    - self-attach detection only, with other cycles the user's responsibility;
    - legacy-only keyboard input;
    - what unexpected loss cannot restore;
    - 0.26.0's unbounded per-cell combining-mark storage, and that earlier
      qualification evidence for another revision does not apply.
- **`GLOSSARY.md`**: add or update attachment, controller, control period,
  grant, detach, take control, hand-back, restoration, restoration prefix,
  snapshot point, retained history, presentation, direct and derived
  presentation, placeholder, leave policy, release, capabilities and startup
  envelope, consistent with section 0.5.
- **`docs/adr/`**:
  - Add an ADR recording the daemon-owned virtual terminal:
    - Alacritty 0.26.0 in `climon2-vt`, with the pin justification and the
      evidence limits;
    - three-phase incremental restoration, the moving-view history semantics,
      and the counting headroom with its memory cost;
    - Direct and Derived presentation and why the clamp needs derived output;
    - the live output policy and query authority;
    - presentation release;
    - zero-buffer synchronized output.
  - Amend the crate-layout ADR (0004) for `climon2-vt` **in increment 6**, with
    the crate itself.
- Crate READMEs: `crates/climon2-client/README.md` (per-invocation logs, the
  `hold-attach` hook), `crates/climon2-daemon/README.md` (any new fault hook)
  and `crates/climon2-e2e/README.md`.
- Mark `docs/agents/session-continuity-handoff.md` as superseded by this plan.

## 9. Native sanity checks (character presence only)

The checks run in four lanes: **Terminal.app** and **VS Code**, each on
**native arm64** and on **native Intel** macOS.

- **Artifact identity**:
  - Every lane uses the **exact final universal archive** whose packaged gates
    passed. Record its SHA-256 and version.
  - Install it into an isolated `HOME`/`CLIMON2_HOME` and invoke the
    **explicit executable path** from that installation. Never use a bare
    `climon2` from `PATH`.
- **Shell isolation** (both hosts): start bash with `--noprofile --norc` **and**
  `HISTFILE=/dev/null` and `BASH_ENV=` (empty). `--noprofile --norc` alone
  does not stop history loading or writing. Never read the user's history
  files.
- **Script**: run `exec bash` and print markers unique to each phase. Detach,
  reattach, and confirm that the phase markers and the restored history are
  present.
- **Terminal.app**:
  - Open a fresh window via `osascript`, running the script.
  - Read the window text through AppleScript, using the `contents` and
    `history` of the run's own tab.
  - Close only that window.
- **VS Code** (setup safeguards that earlier runs found necessary):
  - Use a short temporary root (for example under `/tmp`), so VS Code's IPC
    socket paths stay within Unix socket length limits.
  - Remove `VSCODE_IPC_HOOK_CLI` from the launch environment, so the command
    cannot attach to the user's running VS Code.
  - Launch with every storage location isolated under that root:
    `code --new-window --disable-extensions --user-data-dir <root>/profile
    --extensions-dir <root>/extensions --shared-data-dir <root>/shared
    --agents-user-data-dir <root>/agents-user
    --agents-extensions-dir <root>/agents-extensions
    --agent-plugins-dir <root>/agent-plugins <workspace>`.
    An isolated user-data directory alone still reached the default shared
    storage (`~/.vscode-shared`) in an earlier run. Check `code --help` for the
    installed version; if any storage location cannot be isolated, record the
    lane as blocked rather than launch against default storage.
  - Configure the integrated terminal profile with the shell isolation above.
  - Proceed only if text can be observed without screenshots or the clipboard.
    Otherwise record the lane as blocked.
  - Close only the window and processes your run started.
- **Record per lane**: the archive SHA-256 and version, the host app and its
  version, the architecture (`uname -m`, plus native vs. Rosetta) and the
  outcome.
- **Unavailable lanes**: a lane whose architecture or host is not available
  stays **open** and is reported as outstanding. CI is never a substitute.
- If macOS permission prompts (Automation or Accessibility) block automation,
  record the lane as blocked. Do not change system settings.
- Make no pixel, palette, glyph-shape, selection or elaborate observer
  assertions.

## 10. Finish and report

- Commit and push every increment to `main`.
- CI must be green: the `macos` matrix and `archive` jobs on both `macos-15`
  (arm64) and `macos-15-intel`, plus `dist` and `quality`. Watch with
  `gh run watch`.
- CI green is **not** native acceptance (section 9). Never substitute one
  architecture for the other, and report an unavailable runner as an open gate.
- The final report covers:
  - the exact behaviour completed and the tests added;
  - the gates run and their results;
  - outstanding environment and architecture gates;
  - the four native-lane records (artifact identity, host, architecture and
    outcome, or open/blocked);
  - the action log, listing every action and why;
  - every *(plan choice)* that was substituted.

## 11. Pitfalls

- **Self-attach and environment.** Self-attach detection belongs in the CLI
  preflight. The exec'd client inherits the CLI's environment, not the
  projected session environment. Keep the startup envelope out of the daemon
  and application environment. `TERM` and `COLORTERM` are overwritten only in
  the projected environment.
- **Fixed client log name.** `client.log` with `create_new` blocks the second
  client; use per-invocation logs (section 1.16).
- **PTY reading.** Today the daemon stops reading the PTY when the controller
  is slow. Continuity requires continuous reading, with the 16 MiB live queue
  and a separate, incrementally produced restoration prefix. Switching to
  continuous reading before restoration is complete regresses existing `exec`.
- **Frame-type validator.** It rejects unknown codes before reading the body.
  Add the new codes there, or their frames will be rejected.
- **`HelloReply` parsing.** It uses serde defaults, so unknown fields are
  ignored. Keep mandatory field parsing strict and optional extensions lenient.
  Bad optional data must never make a live session look stale, because stale
  cleanup deletes metadata.
- **Snapshot point.** Snapshot after a completed token, never mid-token, and
  outside the synchronized-update latch. Never wait for the front filter to
  have no partial token.
- **Restoration order.** Phase A (including leaving this attachment's
  alternate screen and resetting margins) precedes all history. Do not emit
  history before the prologue.
- **History capacity is real.** `Term::set_options` and
  `Grid::update_history` change the engine's actual history and drop rows.
  Raising capacity never creates private snapshot storage, and rows beyond
  `max_rows` are never retained history.
- **Eviction is not a content change.** Identical rows replace each other
  silently once history is full. Count pushes; never compare row contents.
- **`set_options` emits a title event.** Compare title values; discard events
  from your own `set_options` calls.
- **Synchronized output.** Do not use the engine's default synchronized-update
  buffering: it re-arms on repeated begins and flushes its buffer in one call.
  Use the zero-buffer `Timeout` and the first-begin latch.
- **Alternate-screen swap.** `Term::swap_alt` resets the alternate grid when
  entering alt mode, so always restore the cloned alternate grid afterwards. It
  also rewrites the normal saved cursor (E1), so assert E1 rather than an
  unchanged saved cursor.
- **Clamped geometry.** Absolute positioning fixes only the initial image.
  Unchanged live bytes wrap and scroll at the host's size; use Derived
  presentation whenever the host is larger than the session.
- **Output passthrough.** Never forward raw OSC 52, raw titles, raw OSC 8 or
  host-dependent requests (section 1.9). Never filter input that looks like a
  reply.
- **Client backpressure.** A poll descriptor of `-1` hides hang-ups. Keep the
  socket in the poll set and service the closing deadline from the poll
  timeout.
- **Resets.** Resetting the input filter must not forget an in-progress local
  paste. Discarding input on transfer must not leave an application paste open
  (section 1.5).
- **Release and retirement.** Release and prologues are computed per attachment
  from committed bytes, not from the session's engine state. Never remove a
  committed frame.
- **JSONC comments.** Replace comments with whitespace; deleting them can join
  tokens into valid but wrong values.
- **Logs.** They must not contain terminal content, titles, arguments or
  environment values.
- **Function size.** Keep each function tiny. Crapper counts `?` as a branch,
  and a test must kill every mutant of every delegation and constant.

## Appendix A. Review findings applied in this revision

The previous revision of this plan was reviewed adversarially. Each finding
and where this document resolves it:

| Finding | Gap in the previous revision | Resolution | Sections |
|---|---|---|---|
| F1 (P1) | The production recipe emitted history before the prologue and preservation, and gave no phase to the normal screen or the post-history scroll | Three explicit phases with every step assigned; literal phase-order tests | 1.8, 3.2 (restoration production), 5.2, 6.2, 11 |
| F2 (P1) | The eviction hold raised real engine capacity, so later snapshots could include rows beyond `max_rows`; the prefix claimed to be both the complete history at S and a moving, truncated view | Retained history is the newest `max_rows` rows only; headroom is never retained; moving-view semantics chosen and stated; tests at limits 0 and 2 | 1.8.2, 3.2 (history accounting), 6.2, 11 |
| F3 (P1) | Clamping the session while forwarding unchanged bytes misaligns wrapping and bottom-row scrolling on oversized hosts | Derived presentation renders live output from engine state when the host is larger; Direct otherwise | 1.6, 1.9, 3.2 (derived presentation), 3.3, 6.1 (14), 8, 11 |
| F4 (P2) | The client's release deadline was unreachable behind blocked stdout | Socket always polled for hang-up; a 5 s bound from the earliest trigger; bounded buffering; honest result | 1.7.1, 1.10, 2, 3.5, 6.3, 11 |
| F5 (P2) | The command-facing `TERM`/`COLORTERM` identity had no implementation step | Projected environment overwrites both; tests listed | 0.4, 1.17, 3.6, 4 (inc. 4), 6.1 (15), 8 |
| F6 (P2) | Eviction was detected by comparing row contents | Push counting with headroom; content-independent flag; identical-row test | 1.8.2, 3.2, 6.2, 11 |
| F7 (P2) | Deleting comments could join tokens into valid settings | Comments become whitespace; split-token rejection tests | 1.3, 4 (inc. 4), 11 |
| F8 (P2) | The first integration slice could regress existing `exec` before restoration breadth existed | Library complete before the pump switch; preservation test in the first integration commit | 4 (inc. 6, 8), 5.1, 6.2 |
| O1 | Period retirement did not say what happens to committed frames and backlog | Committed bytes stay; uncommitted production and backlog drop; prologue from committed state | 3.3, 6.2 |
| O2 | Failure boundaries were linked but not carried into the design | A destination for every real failure; `AuthoritativeFailure` input | 0.4, 1.18, 3.3, 6.2 |
| O3 | Native VS Code setup omitted safeguards found earlier | Shared and agent storage isolation, IPC hook removal, short paths, bash history isolation | 9 |
| O4 | Resize coalescing was optional | Required: one pending size, applied once per iteration; burst test | 0.4, 1.6, 1.10, 3.3, 6.2 |

## Appendix B. Third-party API evidence

All Alacritty references are to `alacritty_terminal` 0.26.0, upstream commit
`94e7c8874e526b1e67b349d9ba30ddf81669119e`.

- `Term::set_options` (emits `Title`/`ResetTitle`, applies history capacity to
  the normal grid even while the alternate screen is active, marks full
  damage):
  <https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/term/mod.rs#L498-L526>
- `Term::swap_alt` (resets the alternate grid on entry and sets the normal
  `saved_cursor`):
  <https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/term/mod.rs#L715-L725>
- `Grid::update_history` (drops rows when capacity shrinks) and
  `increase_scroll_limit` / `scroll_up` (a push at capacity reuses the oldest
  row):
  <https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/grid/mod.rs#L153-L188>
  and
  <https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/grid/mod.rs#L252-L307>
- Rendered source for the same release:
  <https://docs.rs/alacritty_terminal/0.26.0/src/alacritty_terminal/term/mod.rs.html>
  and
  <https://docs.rs/alacritty_terminal/0.26.0/src/alacritty_terminal/grid/mod.rs.html>
- vte 0.15.0 synchronized-update handling (default handler re-arms on repeated
  begins and flushes its buffer in one call):
  <https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs>
- vte 0.15.0 parser state machine (token boundaries for the front filter):
  <https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/lib.rs>
