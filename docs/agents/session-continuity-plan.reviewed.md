# Session continuity: implementation plan (reviewed)

This revises [session-continuity-plan.md](session-continuity-plan.md) by applying
the adversarial critique in
[session-continuity-plan.gao.md](session-continuity-plan.gao.md). The original
plan is left unchanged. Appendix A maps every critique finding to the sections
changed here.

This plan is intended to be executable on its own. Every requirement needed to
implement it is stated here. It is not the authority for those requirements.
Section 0.1 links the confirmed decisions that are. If this plan and a linked
decision disagree, the decision wins: record the conflict and do not invent a
reconciliation.

Each statement has one of three statuses:

- **Settled**: confirmed product behaviour (unmarked text).
- **Accepted exception**: a known, approved deviation. These are labelled
  *(accepted exception)* with their source.
- **Plan choice**: a mechanism proposed by this plan. These are labelled
  *(plan choice)*. A different mechanism may be used only if it meets the same
  stated invariants and tests. Record the substitution in the action log.

Where this plan says "choose", the implementer decides. Code references are to
this repository at the time of writing (`main` after `e0d9b5d`). Re-verify line
numbers before editing.

## 0. Ground rules

- Read `AGENTS.md` and follow it. Invoke the `ub-mode` skill before planning or
  writing code, and follow it:
  - use red-green TDD for every behaviour change;
  - make the smallest edit;
  - keep pure business logic behind checked boundaries;
  - keep an action log and report it at the end.
- A feature is incomplete until automated tests cover it.
- Every function in shipped source must have **CRAP <= 5**. Crapper counts `?`
  as a branch. Mutation testing must leave **zero surviving mutants**. There are
  no exemptions. Add every new shipped crate to the gates (section 7).
- The user's uncommitted `CLAUDE.md` change is unrelated. Never stage, commit,
  revert or stash it. Check `git status` before every commit.
- Authorised: commit **passing** increments and push them directly to `main`
  with your own required commit trailers. Make small, ordered commits. Each
  message says why the change was made, what changed and how it was proven.
- Authorised: open fresh, isolated Terminal.app and VS Code windows for automated
  native sanity checks (section 9). Control only windows that your run created,
  and use isolated data directories. Screenshots, clipboard access and the
  user's existing windows are **not** authorised.
- Not authorised: publishing a release, creating release tags or closing GitHub
  issues.
- The user may be unavailable. Ordinary design choices do not need approval.
  If something is genuinely blocked, record it, keep the guard in place and
  continue with independent work. Examples are a missing permission or an
  unavailable architecture.
- Clean up only resources that your run created: temporary homes, sockets,
  windows, processes and worktrees.

### 0.1 Sources and supersession

The latest confirmed amendment overrides older issue bodies and prototype
requirements.

| Topic | Authoritative decision | Effect on this plan |
|---|---|---|
| Implementation may start; unchanged Alacritty | [#40 latest][map-latest], [#43 final engine/testing][engine-latest] | Unchanged upstream engine; ordinary product checks; safe title encoding; hyperlinks degrade to plain text |
| Budgets and restoration qualification | [#48 B5 budgets][budgets], [#48 follow-up][budget-followup], [#48 latest][protocol-latest] | Previously approved ordinary budgets remain. There is no new restore-size cap and no serialization strategy approved by issue. The restoration prefix is separate from the live queue. Synchronized output uses zero engine buffering, with a deadline from the first begin |
| Legacy input and paste | [#43 legacy-input refinements][paste-obligation] | No in-band host queries. The daemon finishes an application paste that was already started. Reply-shaped key input is legitimate |
| Output, failure and cleanup baseline | [#43 cleanup baseline][cleanup-baseline] | A documented normal-shell mode baseline is allowed on exit. OS terminal settings are restored exactly |
| Restoration exceptions | [#43 restore decisions][restore-decisions] (E1) | The normal saved cursor may be perturbed by alternate-screen restoration |
| Unsafe OSC 8 / encoding seam | [#43 OSC 8 experiment][unsafe-osc], [#43 encoding follow-up][safe-osc] | Live and restored link metadata must not reach the host unencoded |
| Acceptance lanes | [#45 approved lanes][acceptance], [#45 latest][acceptance-latest], [#52 latest][native-latest] | Four native lanes on the exact final archive. Character presence only, without pixel or exhaustive observers |
| Self-attachment | [#50 replacement][self-only] | Reject only identified direct self-attachment |
| Keypad scope | [#56 keypad amendment][keypad-reduction] | No special application-keypad fidelity. Ordinary digits and cursor/paste alignment remain required |
| History retention | [#49 retention][history-amendment], [#49 notice][notice] | No history-byte accounting and no incremental catch-up. The coarse truncation notice is required |
| Cross-version contracts | [#44][cross-version], ADR 0006 | The schema-1 startup envelope and named capability/state tokens are additively frozen (section 1.15) |
| Engine provenance | [#46][engine-research], [#53][embedding-research], [#56 provenance][provenance] | That evidence applies to `bbb2cbf…`, not to the shipped 0.26.0. See section 3.1 |

Out of scope by decision; their absence is not a defect:

- another engine, engine patches or indirect-cycle detection;
- a client emulator or extended keyboard translation;
- history identity or history-byte accounting;
- lossless hyperlinks;
- exact native de-duplication, pixel sampling and a renewed research programme;
- avoiding accepted duplicate history or changing the reason-sensitive leave
  policy.

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
  - an attach refused because the session is terminating;
  - unexpected daemon connection loss;
  - an unconfirmed detach, or a failed or unconfirmed kill.
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
- Never retarget if the selected session disappears. If a target disappears or
  changes identity between selection and dispatch, fail with 125.

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
  - malformed JSONC;
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
- **Filter state has two independent parts** (R2):
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
- **Control period**: every grant starts a new period, numbered by a monotonic
  `u64`. The new controller's application input stays blocked until it has
  received and written the complete restoration for that period and confirmed
  readiness. Input typed or queued before readiness is discarded and never
  replayed.
- When control transfers, any of the old controller's input not yet written to
  the PTY is discarded. Late input from an earlier period is ignored, even if
  the same client later regains control. Bytes already written to the PTY
  cannot be recalled.
- **Application paste cleanup** (R2; [#43][paste-obligation]): the daemon
  finishes any application paste that was already started before it admits
  later input.
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
  daemon queues that attachment's presentation release (section 1.7.1), then
  `Revoked`, and stops forwarding output to it. The client then shows a
  placeholder.
  - Sending the revocation after any queued output on the same connection
    guarantees that stale output cannot overwrite the placeholder.
- The client draws the placeholder:
  - It preserves visible outer content as described in section 1.8.
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

### 1.6 Sizing

- Only the controller's size drives the PTY and the daemon's virtual terminal.
  Non-controller resizes are only remembered.
- On every grant (attach, takeover or hand-back), apply the new controller's
  latest valid size before producing its restoration.
- The maximum session geometry is **500 columns x 200 rows**. Larger host sizes
  are clamped. The session occupies a fixed rectangle at the host's top-left
  origin; see section 1.8, step 3.
- An invalid (zero) initial size falls back to 80x24 (existing
  `TerminalSize::initial`). Later zero-sized updates are ignored (existing
  `TerminalSize::resize`).
- Intermediate resizes may be coalesced. If the controller resizes before it has
  confirmed readiness, issue a fresh restoration at the new size in a new
  period. Its prologue starts from that attachment's actual host presentation
  (section 1.8, step 0). After readiness, resizes are applied normally and the
  application redraws.
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
    - restoration-deadline expiry (section 1.10).
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
  terminal and bounded history. Having no clients never blocks the command's
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

#### 1.7.1 Presentation release on exit (R3)

Every **daemon-confirmed end of an attachment's presentation** gets a release,
in this order. Those ends are revocation, accepted detach, command exit and
shutdown with the attachment present:

1. **Daemon**: after any output already queued for that attachment, queue its
   release bytes as `Output`. Compute them from that attachment's
   **tracked host presentation**, not from the session's current engine state
   (section 3.3). Then queue `Revoked`, `Detached` or `Exit`.
   - A non-controlling attachment was already released at revocation, so it
     needs no further daemon release.
   - Never send `CSI ? 1049 l` to an attachment whose host was never put on the
     alternate screen by this attachment's stream. A placeholder client is one
     example.
2. **Client**: write every `Output` byte received before the terminal message
   to stdout, including short or blocked writes, with a bounded deadline of
   5 s. Then end any placeholder presentation with its own known bytes: cursor
   to the last row, then CR LF. Then restore termios exactly. Only after that,
   write any stderr line or diagnostic and exit.
   - If the stdout deadline expires, still restore termios and report honestly.
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

**Documented normal-shell baseline** after release ([#43][cleanup-baseline]):

- synchronized update ended;
- main screen, only if this attachment entered the alternate screen;
- margins reset; origin, insert and line-feed/new-line modes off; autowrap on;
- mouse, focus, bracketed paste, application cursor keys and application keypad
  off;
- cursor visible with the default cursor style; `CSI 0 m`.

The host title is not restored; document this. Captured OS terminal settings
(termios) are restored exactly.

### 1.8 Restoration, history, title and hyperlinks

The daemon owns one virtual terminal per session: Alacritty, wrapped by
`climon2-vt` (section 3.2). Clients keep no emulator and no hidden transcript.

On **every grant**, the daemon sends the new controller a restoration and then
live output. Grants are the initial exec attach, an explicit attach, takeover,
hand-back and a re-grant after a pre-readiness resize.

**Snapshot point (D2).** The restoration describes the engine's state after
the **last completed token**. It does not wait for the front filter to have no
partial token.

- A partial token held by the front filter has reached neither the engine nor
  any host. It stays pending.
- When the token completes, it is fed to the engine. If it is forwarded under
  section 1.9, it is forwarded to the new controller as live output after
  cutover.
- The only remaining wait before the snapshot is the synchronized-update latch,
  which is bounded (section 1.10).

**Incremental production (D1).** The restoration is a **prefix**. It is
produced incrementally as the attachment's socket drains, and it is not
counted against the live queue.

- The total prefix size is unbounded by any new cap. Only the outstanding
  produced-but-unwritten prefix bytes are bounded, by a small refill window
  (plan choice: 1 MiB).
- Live output generated after the snapshot point is queued behind the prefix in
  the attachment's live queue (16 MiB). A prefix larger than 16 MiB therefore
  reaches a healthy reader completely and in order.
- Valid history is never truncated to fit a queue.

*(plan choice)* Production uses the mechanism in section 3.2. Any substitute
must satisfy the invariants and tests in section 3.2.

The restoration contains these steps:

0. **Prologue.** The host may still hold modes from an earlier application, or
   from an earlier restoration on this same attachment. Emit, in this order:
   - If this attachment's tracked host presentation is on the alternate screen
     (D4), `CSI ? 1049 l` to leave it. A replacement restoration before
     readiness is the usual cause. This attachment created that alternate
     presentation, so leaving it returns the host to its genuine normal
     screen.
   - `CSI ? 2026 l` to end any open synchronized update.
   - `CSI r` to reset margins.
   - `CSI ? 6 l` (origin off), `CSI 4 l` (insert off) and `CSI 20 l`
     (line-feed/new-line off).
   - `CSI ? 7 h` (autowrap on) and `CSI 0 m`.

   The following steps rely on this state. Never blindly reset a host screen
   whose original state is unknown.
1. **Preserve visible outer content.** Move the host cursor to the bottom row
   (`CSI <host rows> ; 1 H`, using the controller's reported, unclamped rows).
   Then emit `host rows` line feeds so that the visible host screen scrolls into
   native history. Never send `CSI 2 J`, `CSI 3 J`, `ESC c` or anything else
   that clears native history.
2. **Truncation notice.** If the daemon knows retained history was evicted
   (section 3.2) and history is non-empty, emit the line
   `[climon2: older session scrollback was discarded]` in the default rendition,
   followed by CR LF.
   - This is presentation text, not part of retained history. It is emitted on
     every such restoration.
   - If rows are evicted while the prefix is still being produced, emit the same
     notice at the gap (section 3.2).
3. **Normal history, then screen, in the session rectangle (D4).**
   - Render every retained history row, oldest first, from parsed cell state.
     This includes text, wide characters, combining marks and SGR rendition:
     bold, dim, italic, underline styles, inverse, hidden, strikeout, and
     foreground, background and underline colours.
   - End hard-ended rows with CR LF. Write soft-wrapped rows full-width without
     CR LF so that the host wraps them itself. If the host is wider than the
     session because of the 500-column clamp, use CR LF for every row.
   - After the history rows, move to the host's bottom row and emit `host rows`
     line feeds. Restored history then sits in native history and the visible
     host screen is blank.
   - Render each normal-screen row at its absolute position inside the
     **session rectangle**: host rows `1..=session rows` and host columns
     `1..=session cols`, using `CSI <row> ; 1 H` per row.
   - The rectangle stays at the host's top-left origin. Subsequent
     cursor-addressed application output therefore lands where the application
     intends, including when the host is taller or wider than the clamp.
     Host cells outside the rectangle stay blank.
   - Never replay raw historical output, clears, bells, queries, clipboard
     operations or other side effects. Hyperlinks become plain text.
4. **Normal cursor.**
   - First establish the normal screen's saved cursor (position and pen, then
     `ESC 7`). Then establish the current cursor position and pen.
   - If the cursor has a pending wrap (`Cursor::input_needs_wrap`), position at
     the last column and re-print that cell's character with its rendition.
     The host then also enters the pending-wrap state; CUP alone clears it.
5. **Alternate screen (eager).** If the application is on the alternate screen:
   - emit `CSI ? 1049 h`;
   - render the alternate screen's rows in the session rectangle (no history);
   - establish its saved cursor and its current cursor and pen, including any
     pending wrap as in step 4.

   When the application later leaves the alternate screen, the host shows the
   correct normal content.
6. **Modes.** Emit DECSET/DECRST so that the host matches the virtual
   terminal's modes that affect display or input:
   - cursor visibility (25);
   - application cursor keys (1);
   - application keypad (`ESC =` / `ESC >`);
   - bracketed paste (2004);
   - mouse reporting modes (1000/1002/1003) and encodings (1006/1005);
   - focus reporting (1004);
   - alternate scroll (1007);
   - autowrap (7) and origin (6);
   - insert (`CSI 4 h/l`) and line-feed/new-line (`CSI 20 h/l`);
   - cursor style (DECSCUSR from `Term::cursor_style`).
7. **Title.** Emit `OSC 2 ; <encoded title> ESC \` with the application's
   current title, or with the default title when none is set (section 1.8.1).
8. **End.** Mark the end of restoration (`RestoreEnd`, section 3.4). Live output
   follows in order.

When the application is on the alternate screen, the underlying normal screen
and history are still restored: step 3 runs before step 5. This is required.

Not restored; document these as limitations:

- scroll region, tab stops and character sets;
- palette changes (OSC 4/10/11), which are also not forwarded live (section
  1.9);
- keyboard-protocol modes;
- hyperlinks;
- the host title on exit.

*(accepted exception E1, [#43][restore-decisions])*: when a restoration is taken
while the application is on the alternate screen, the normal screen's
**saved cursor** becomes the normal current cursor at restoration time
(section 3.2). The normal content, history and current cursor are unchanged.
Usually this matches the previous saved cursor exactly. It can differ after a
hidden resize. Document this.

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
- Every title emission, both restored and live (section 1.9), preserves
  ordinary Unicode. It percent-encodes each UTF-8 byte of these characters as
  uppercase `%XX`:
  - every C0 control (U+0000..U+001F);
  - DEL (U+007F);
  - every C1 control (U+0080..U+009F, that is `%C2%80`..`%C2%9F`).

  Nothing else is encoded.

### 1.9 Live output policy at the VT seam (R4)

One policy, applied by the `climon2-vt` front filter, decides what reaches the
controller's host.

- The daemon reads the PTY continuously, and every byte updates the virtual
  terminal.
- Forwarded tokens go to the controller only, when one exists and its
  restoration prefix has been started (section 1.8).
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
5. **Titles, re-encoded.** Strip raw OSC 0/1/2. Where the tracked title changes
   (`Title` / `ResetTitle` events after feeding the token), emit
   `OSC 2 ; <encoded title> ESC \` (section 1.8.1) in its place.
6. **Hyperlinks, plain text** *(plan choice, an approved degradation per
   [#43][engine-latest])*. Strip OSC 8 open/close sequences. The linked text,
   which is outside the control string, is forwarded normally. This matches the
   restoration. Mechanically encoding the link controls is the permitted
   alternative. Raw OSC 8 is never forwarded.
7. **Every other OSC is stripped.** Examples are palette set/reset, OSC 7 and
   shell-integration marks. They are host side effects that restoration cannot
   reproduce. Document this as a limitation.
8. **Everything else is forwarded unchanged.** This covers text, C0 controls,
   ESC sequences and the remaining CSI sequences: SGR, cursor movement, modes,
   synchronized-output begin/end and so on.

Other rules:

- Control strings (OSC, DCS, SOS, PM, APC) are held until their terminator.
  If a body exceeds **4096 bytes**, the token is dropped from both the engine
  and live output and logged without content; the session continues. The same
  rule applies to CSI sequences longer than 4096 bytes.
- The engine's `Bell` events have no daemon side effect. A bell reaches the
  host as a forwarded C0 byte.
- Hyperlinks never fail or terminate a session.

### 1.10 Internal budgets (not user configurable)

| Budget | Value |
|---|---|
| Control-string / CSI cap | 4096-byte body; oversized tokens are dropped and logged |
| Geometry | Max 500x200; larger sizes clamped; zero rejected (80x24 initial fallback); session rectangle at the host origin |
| Engine step | Feed the engine at most 64 bytes per step, at token boundaries where possible. No engine-side synchronized-update buffering, so no exception to this step bound |
| Synchronized output (`CSI ? 2026 h/l`), D3 | Per [#48 B5][budgets]: **zero engine buffering**; adapter deadline **150 ms from the first begin** (a first-begin latch: repeated or fragmented begins do not extend it). The snapshot for a pending grant waits for an explicit end or that deadline, and `tick` services the deadline without another PTY read. Host presentation is separate: the filter tracks whether this attachment's host has an open update and emits a synthetic `CSI ? 2026 l` on deadline, revocation, release or restoration. This is not a hard wall-clock CPU bound |
| Live queue (per attachment) | 16 MiB of live output (D1). The restoration prefix is not counted. Overflow disconnects that attachment as an unexpected departure |
| Restoration prefix | Produced incrementally; at most 1 MiB produced-but-unwritten (plan choice). No total-size cap |
| Restoration deadline (D2) | One overall deadline per period, from grant to accepted `Ready{period}`, covering preparation (synchronization latch), delivery and the client's write-and-`Ready`. It fails on either condition below, as an unexpected departure: (a) **no socket write progress for 5 s** while prefix or queued bytes are outstanding; (b) **not `Ready` within `30 s + produced_prefix_bytes / 4 MiB/s`**, where the allowance grows as prefix bytes are produced |
| Initial attachment | 5 s after readiness |
| Kill operation | 10 s overall (CLI) |
| Client release write | 5 s bound on writing the final output/release bytes before restoring termios |
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

The CLI never talks to a daemon after hello. For attach and kill it:

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
  application's environment.
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
- **Additively frozen by [#44][cross-version] and ADR 0006 (O2)**. Never change
  the meaning of these. New behaviour needs a new token or schema:
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

### 1.16 Client diagnostics per invocation (R1)

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
  (`crates/climon2-platform/src/logging.rs:83-99`) opens the fixed name
  `client.log` with `create_new(true)`
  (`crates/climon2-client/src/lib.rs:96-101`). The second attach therefore
  fails before hello. Fix this before any continuity path is exercised
  (increment 3).
- Update the assertions that depend on the fixed name directly:
  - `crates/climon2-client/tests/bridge.rs:480,941`;
  - `crates/climon2-e2e/tests/packaged.rs:420,792`.

## 2. Current code map

The shipped crates are all synchronous, with no async runtime.

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
    `CLIMON2_SESSION_ID`, `CLIMON2_SESSION_DEPTH` and the default `TERM`.
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
    (backpressure).
  - Output goes only to the controller.
  - Controller departure hangs up the session.
- **`climon2-client`**:
  - `lib.rs`: argv, metadata, logs (`session_log` at `lib.rs:96-101`, fixed
    `client.log`), hello validation, attach and raw mode.
  - `transport.rs`: framed transport, hello and attach.
  - `bridge.rs`: a `poll` loop over socket, stdin and stdout. SIGWINCH becomes
    `Resize`, signals become `128+signal` and stdin EOF becomes 129.
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
- **`climon2-fixtures`**: `climon2-fixture`, `climon2-tui-fixture`,
  `climon2-cli-fixture` and `climon2-daemon-fixture`.
- **`climon2-e2e`**: `tests/packaged.rs` and `tests/support/mod.rs` (with
  `Package`).

Guards and gates (D7, R5):

- **`xtask/src/quality.rs`**:
  - the `SHIPPED` source list;
  - CRAP threshold `"5"`;
  - the crapper and mutator pins;
  - changed-source selection via `CLIMON2_MUTATION_BASE`.
- **`xtask/tests/architecture.rs`**:
  - the exact package set (`PACKAGES`, `:4-17`, checked by
    `workspace_has_the_agreed_packages`);
  - allowed internal dependency edges (`allowed_internal_dependencies`,
    `:75-84`, checked by `shipped_packages_follow_dependency_direction`).

  Both guards cite ADR 0004.
- **Other xtask modules**: `xtask/src/release.rs` (`PACKAGES`),
  `xtask/src/dist.rs` and `xtask/src/fixtures.rs`.
- **CI, `.github/workflows/check.yml`**:
  - The `macos` job is a matrix of runner x bash profile.
  - The `quality` job runs on `macos-15` and **pins `CLIMON2_TEST_BASH:
    /bin/bash`** (`:75-80`).
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
  - It is platform-neutral: no I/O, sockets or PTYs.
  - It has no internal dependencies.
- **Engine pin (D6). Decision: ship published `alacritty_terminal` 0.26.0.**
  - Identity: `Cargo.lock:15-18`, checksum
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
  - Evidence that does **not** carry over: most actual-engine qualification in
    [#46][engine-research], [#53][embedding-research] and
    [#56 provenance][provenance] used unchanged upstream
    `bbb2cbf…` (0.26.1-dev).
    - Do **not** claim that revision's later bound on combining-mark storage.
      0.26.0 stores combining marks per cell without that bound.
    - Do **not** carry over its provenance findings.
    - Record both points in the new ADR and in the README limitations.
  - Validation is limited to the seams this choice affects: the adapter, the
    front filter, sync timing, alternate-screen swap and serialization. Use the
    focused tests of increment 6, including one cell carrying many zero-width
    marks that still restores and serializes correctly. Do not restart
    engine-family selection or exhaustive upstream testing.
- `climon2-daemon` depends on `climon2-vt`; no other shipped crate does.
- Pure, shared contracts go in `climon2-core`:
  - config parsing (`config.rs`) and key vocabulary;
  - selector parsing and matching (`selector.rs`);
  - the startup envelope (`startup_envelope.rs` or similar);
  - hello extensions, capabilities and state;
  - the title-encoding helper, if shared;
  - new protocol messages.
- Shipped crates must not depend on test-only crates.
- **Same commit that creates the crate (D7)**:
  - Add `crates/climon2-vt/src` to `SHIPPED` in `xtask/src/quality.rs`.
    Confirm that coverage, crapper and mutator see it. Re-run
    `cargo xtask tooling-probe` if selection logic is touched.
  - In `xtask/tests/architecture.rs`:
    - add `climon2-vt` to `PACKAGES`;
    - give `climon2-vt` no allowed internal dependencies;
    - add `climon2-vt` to `climon2-daemon`'s allowed internal dependencies.
  - Amend ADR 0004 for the new crate and the new daemon edge.
  - Never weaken either guard.
- Daemon binary packaging is unchanged: `climon2-vt` is a library and needs no
  packaged binary.

### 3.2 The virtual terminal adapter (`climon2-vt`)

Suggested public surface (adapt as needed; keep it small):

```rust
pub struct Geometry { pub cols: u16, pub rows: u16 }          // already clamped
pub struct VtConfig { pub max_history_rows: u32, pub default_title: String }
pub struct Vt { /* Term, Processor, front filter, event queue, title, eviction flag, sync latch */ }

impl Vt {
    pub fn new(geometry: Geometry, config: VtConfig) -> Self;
    /// Feed PTY output. Returns live tokens (section 1.9 policy applied) and replies.
    pub fn feed(&mut self, bytes: &[u8], now: Instant) -> Fed;  // Fed { live, replies, dropped }
    pub fn resize(&mut self, geometry: Geometry);
    pub fn sync_pending(&self, now: Instant) -> bool;          // first-begin latch open and not expired
    pub fn tick(&mut self, now: Instant) -> Fed;                // expires the latch; synthetic ESU
    pub fn snapshot(&mut self, host: Geometry, presentation: Presentation) -> RestoreCursor; // D1/D4
    pub fn produce(&mut self, cursor: &mut RestoreCursor, budget: usize) -> Chunk; // next prefix bytes
    pub fn release(presentation: Presentation) -> Vec<u8>;      // section 1.7.1 baseline
}
```

`Presentation` is the small per-attachment host-state record from section 3.3.
`RestoreCursor` is the restoration in progress.

`at_boundary()` is **removed** (D2). Snapshots never wait for a partial
front-filter token.

#### Engine setup and events

- Create the engine with `Term::new(config, &dimensions, listener)` and
  `term::Config { scrolling_history: max_rows, kitty_keyboard: false,
  osc52: Osc52::Disabled, ..Default::default() }`. Implement `Dimensions` with a
  tiny struct.
- Events: `EventListener::send_event(&self, Event)` takes `&self`. Use a
  listener holding `Rc<RefCell<Vec<Event>>>` shared with the adapter; the daemon
  is single-threaded.
  - Drain the events after each step.
  - `PtyWrite(text)` becomes replies.
  - `Title(t)` and `ResetTitle` update the tracked title and request the
    re-encoded live title (section 1.9).
  - Ignore all other events.
- Grid access (Alacritty 0.26 public API):
  - `term.grid()` returns a `Grid<Cell>` with `Dimensions::{history_size,
    screen_lines, columns, topmost_line, bottommost_line}`, indexed by `Line`
    and `Column`.
  - `grid.cursor` and `grid.saved_cursor` expose public `point` and
    `template`.
  - Each cell has `c`, `fg`, `bg`, `flags`, `zerowidth()` and
    `underline_color()`. Relevant flags are `WRAPLINE`, `WIDE_CHAR`,
    `WIDE_CHAR_SPACER`, `LEADING_WIDE_CHAR_SPACER`, `BOLD`, `DIM`, `ITALIC`,
    the underline variants, `INVERSE`, `HIDDEN` and `STRIKEOUT`.
  - Modes come from `term.mode()` (`TermMode`). Cursor style comes from
    `term.cursor_style()`.

#### Synchronized output (D3)

Follow [#48 B5][budgets] and the [vte 0.15.0 source][parser-source] behaviour
cited by the review: the default handler re-arms on repeated begins
(`ansi.rs:410-411,459-460`) and flushes its buffer in one call (`:327-339`).

- Parameterise `Processor` with a climon2 `Timeout` implementation whose
  `pending_timeout()` always returns `false`. The engine therefore never enters
  its synchronized buffering path. This is **zero engine buffering**.
  `stop_sync` is never needed, and no exception to the 64-byte step exists.
- The front filter recognises DECSET/DECRST of private mode 2026 and drives a
  **first-begin latch** using the `now` passed to `feed` and `tick`:
  - the first begin opens the update with a deadline of 150 ms;
  - repeated or fragmented begins while it is open do not extend that deadline;
  - an explicit end or `tick` past the deadline closes it.
- `sync_pending(now)` gates only the snapshot of a pending grant. `tick` closes
  an expired latch without needing another PTY read.
- Engine work scheduling (64-byte steps) is independent of host presentation.
  Begin and end tokens are forwarded live like other CSI sequences. The
  per-attachment `Presentation.sync_open` drives the synthetic `CSI ? 2026 l`
  on expiry, revocation, release or restoration.
- Tests use an injected clock. Cover a first begin, a repeated or fragmented
  begin, an explicit end, a timeout and re-arming after close. Include a
  pending grant served through the daemon loop after the deadline with no
  further PTY input. Do not promise a hard wall-clock CPU bound.

#### Front filter / tokenizer

The front filter is pure, inside `climon2-vt`. It splits PTY output into
complete tokens: printable text runs (never split inside a UTF-8 sequence), C0
controls, ESC sequences, CSI sequences and control strings.

- Mirror the state machine in `vte-0.15.0/src/lib.rs` exactly:
  - OSC ends on BEL or ST (`ESC \`);
  - DCS, SOS, PM and APC end on ST only, so BEL is payload;
  - an ESC not followed by `\` ends the string and starts a new escape;
  - CAN and SUB abort;
  - C0 controls inside CSI are executed.
- Differential-test token boundaries against vte on tricky inputs.
- Partial tokens are buffered across reads, bounded by the 4096-byte cap.
- For each complete token:
  - drop it entirely if it exceeds the cap;
  - otherwise feed it to the engine in steps of at most 64 bytes, then apply the
    section 1.9 classification to decide what is appended to `live`.
- Snapshots are taken after a completed token (D2). A pending partial token is
  later fed and forwarded after cutover, so the host and the engine agree at
  the cutover point.

#### Eviction detection (for the notice)

- Track eviction only when `max_rows > 0`.
- Before each engine step, if `history_size()` equals the current capacity,
  remember the topmost history row. After the step, if history is still full
  and that row differs, set `evicted`.
- Clear `evicted` when `history_size()` drops to 0 (history cleared).
- A resize reflow that evicts rows also sets `evicted`, as does trimming back to
  `max_rows` after a restoration (see below).
- This is coarse by design: it does not measure what was lost.

#### Incremental restoration prefix (D1)

*(plan choice; invariants are binding.)*

**Invariants:**

- **Consistent snapshot and cutover.** The prefix describes the engine state
  at snapshot point S. Every live token completed after S follows the prefix,
  in order, with nothing lost and nothing duplicated.
- **No new restore-size cap** and no silent truncation of valid history.
- **Separate queues.** The total prefix size is distinct from the bytes
  outstanding on the socket. The prefix is not counted against the 16 MiB
  live queue.
- **Bounded pump work per loop iteration.** PTY reading, query replies and
  other connections keep progressing while a large prefix drains.

**Mechanism:**

1. **At S, serialize the bounded tail eagerly** into the `RestoreCursor`. The
   tail is the prologue, steps 4–7 and step 5's alternate screen. Its size is
   bounded by geometry (at most 500x200 per screen). Record `h0`, the
   history size at S. The notice (step 2) is decided now.
2. **Produce history rows lazily from the live engine.**
   - On each loop iteration, `produce` serializes history rows (oldest first)
     until the attachment's outstanding prefix bytes reach the refill window
     (1 MiB).
   - After the last history row, the eager tail follows, then `RestoreEnd`.
3. **Hold eviction while any prefix is in progress, so rows can be located.**
   - While a `RestoreCursor` exists, raise the engine's history capacity to
     `max_rows + H` via `Term::set_options`. History rows are then only
     appended at the newest end, so snapshot row `k` sits at
     `topmost_line + (k − trimmed)`.
   - `H` must be at least the most rows one engine step can push into
     history. Derive and test a conservative bound from the 64-byte step and
     the session rows (for example `64 × rows`).
   - Before each engine step, if the remaining headroom is below that bound,
     trim explicitly. Lower the capacity by a known count and add it to
     `trimmed`; this is the only way rows leave history during a prefix.
   - Unsent snapshot rows with `k < trimmed` are skipped. The truncation notice
     line is emitted at that point in the prefix.
4. **Clearing history** (history smaller than accounted for) ends the
   history portion; the eager tail follows.
5. **Resize** happens only at a grant, which abandons the in-progress
   `RestoreCursor` for a new period. Reflow therefore never moves unsent rows.
6. When no `RestoreCursor` remains, restore the capacity to `max_rows`. Any
   eviction this causes sets the coarse `evicted` flag.
   - The extra transient memory (up to `H` rows) is the price of this
     mechanism. Record it in the ADR.
   - An implementation that meets the invariants with less memory may replace
     this mechanism. A full clone of history is not required.

#### Reading the normal screen while the alternate screen is active (D5)

The inactive grid is private. Inside `snapshot`, single-threaded and never
while `sync_pending`, use this sequence:

1. `let alt = term.grid().clone();` This is bounded, because the alternate
   screen has no history.
2. `term.swap_alt();` While in alt mode this only swaps; it does not reset.
   The normal grid, with history, is now `term.grid()`. Serialize its screen
   and cursor (steps 3–4) and record the history mapping.
3. `term.swap_alt();` This switches back. It resets the alternate grid,
   overwrites its cursor **and sets the normal grid's
   `saved_cursor = cursor`**. Alacritty 0.26.0
   `alacritty_terminal/src/term/mod.rs:715-725`, [source][engine-source].
4. `*term.grid_mut() = alt;` This puts the alternate grid back exactly.

History rows of the normal grid produced later by `produce` are read through
the same swap sequence. Batch them per `produce` call.

Tests must state exactly what is preserved:

- The alternate screen's content, cursor and saved cursor are unchanged.
- After the application later leaves the alternate screen, the normal content,
  history, current cursor and keyboard modes are unchanged.
- The normal **saved cursor** equals the normal current cursor at restoration
  time. This is accepted exception E1. Assert this exact value; do **not**
  assert that the saved cursor is unchanged.
- Include a case with a hidden resize where E1 differs from the previous saved
  cursor, so that the test distinguishes E1 from unintended corruption.

#### Serializer

The serializer is a set of pure functions over a read-only grid view.

- Build SGR from cell attributes:
  - named colours map to 30–37, 90–97, 40–47, 100–107, 39 and 49;
  - indexed colours map to `38;5;n`;
  - RGB colours map to `38;2;r;g;b`;
  - underline colour maps to `58:…`.
- Emit `ESC [ 0 m` at the start of each row.
- Skip wide-character spacers and append zero-width characters.
- Trim trailing default blank cells on hard-ended rows. Print soft-wrapped rows
  at full width.
- Position screen rows absolutely in the session rectangle (section 1.8, step
  3).
- Encode titles as in section 1.8.1.
- Keep functions tiny. Table-drive SGR and mode emission so that CRAP stays at
  5 or below.

### 3.3 Daemon structure

**One writer.** The existing single-threaded `Session` loop owns the PTY, the
`Vt`, all connections, a pure **control state machine** and the per-attachment
`Presentation` records. No other thread touches them.

**Control state machine** (a new pure module, for example
`crates/climon2-daemon/src/control.rs`). Inputs are events and outputs are
effect lists. It owns:

- lifecycle and attachments;
- the controller and the hand-back recency order;
- the control period and readiness;
- each client's remembered size and the applied session size;
- the application-paste cleanup obligation;
- shutdown state.

```rust
enum Input { Attach{conn, size}, TakeControl{conn}, Ready{conn, period},
             Resize{conn, size}, Detach{conn}, Lost{conn}, Kill{conn},
             InitialDeadline, RestoreDeadline{conn, period}, CommandExited }
enum Effect { Grant{conn, period}, Release{conn}, Revoke{conn},
              ResizeSession(size), AcceptDetach{conn}, Refuse{conn, reason},
              EnableInput{conn}, DiscardPendingInput{conn}, ClosePaste,
              BeginShutdown, DeliverExit, ... }
```

- Every loss of input admission emits `DiscardPendingInput` and, when
  applicable, `ClosePaste` (section 1.5).
- Every daemon-confirmed end of presentation emits `Release` before
  `Revoke`, `AcceptDetach` or `DeliverExit` (section 1.7.1).

The rules of sections 1.5–1.7 live here and nowhere else. Expose
`snapshot() -> State` for hello. Unit-test the module exhaustively; it is the
easiest place to kill mutants.

**`Presentation` (per attachment; D4, R3).** This is the small record of what
this attachment's host has been put into by the daemon's stream: `alternate`
and `sync_open`.

- It is updated as bytes are **queued** for that attachment:
  - restoration step 5 sets `alternate`;
  - the prologue clears it;
  - forwarded live `CSI ? 1049/1047/47 h/l` set and clear it;
  - forwarded live `CSI ? 2026 h/l` and the synthetic end set and clear
    `sync_open`.
- It is used for the replacement-restoration prologue and for
  `Vt::release(presentation)`.
- Release is the section 1.7.1 baseline, plus `CSI ? 1049 l` only when
  `alternate` is set.
- This is not a client emulator.

**Paste cleanup (R2).** A pure tracker runs over input bytes **as actually
written** to the PTY, so it sees partial writes. It records `open` and any
partially written marker.

- `ClosePaste` computes the cleanup bytes (section 1.5) when engine mode 2004 is
  set.
- Those bytes become a PTY-write prefix that must be fully written before any
  later admitted input.

**Loop changes** (`session.rs`):

- Read the PTY every tick, regardless of clients, bounded per tick (for
  example one 32 KiB read). Feed the `Vt` and write replies to the PTY. Enqueue
  `live` tokens to the controller's live queue once its `RestoreCursor` exists.
- Call `vt.tick(now)` on every loop iteration.
- **On `Grant`**:
  1. Apply the size (`ResizeSession` → `pty.resize` + `vt.resize`).
  2. Wait until `!vt.sync_pending(now)`, which is bounded by the latch. Then
     create the `RestoreCursor` with that attachment's `Presentation`, and
     enqueue `Granted{period}`.
  3. On each loop iteration, refill prefix `Output` frames (each at most
     65,536 bytes) up to the refill window.
  4. After the tail, enqueue `RestoreEnd{period}`. Live frames queued since S
     follow.
  5. Abandon any earlier `RestoreCursor` of the same attachment.
- **On `Release`**: enqueue `Vt::release(presentation)` as `Output` after
  queued output, then the terminal message. Stop live output.
- **Input frames** reach the PTY only from the ready controller of the current
  period. They go after any pending paste-cleanup prefix.
  `DiscardPendingInput` drops that attachment's unwritten input.
- Enforce the 16 MiB live queue and the restoration deadline (section 1.10) in
  `connection.rs`:
  - track socket write progress separately from receipt of `Ready`;
  - violations become `Lost{conn}` or `RestoreDeadline`, which are unexpected
    departures.
- **Initial deadline**: 5 s from readiness while `starting` raises
  `InitialDeadline`.
- **Shutdown needs restructuring, not just reuse.**
  - Today `daemon/src/lib.rs` `serve` returns from `session::serve`, dropping
    every connection, and only then calls `Resources::cleanup`. `Pty::leave`
    also discards the command's exit status and blocks the loop with
    `thread::sleep(HANGUP_GRACE)`.
  - Change it as follows:
    1. Make the hangup grace a loop deadline. The loop keeps answering hello
       with `terminating` and refusing attaches.
    2. Capture the actual exit status from hangup or kill.
    3. Have the session perform `Resources::cleanup` itself **while established
       connections stay open**. Sending on an accepted Unix stream after
       unlinking the socket path is valid.
    4. Only then queue each attachment's release (section 1.7.1), `Exit{code}`
       to attachments and `Terminated{code}` to kill requesters. Never send
       `Terminated` if cleanup failed.
    5. Flush all connections non-blockingly under one shared short deadline,
       prioritising kill requesters, then exit.
  - Keep the existing normal-exit drain.
- Hello replies include `capabilities` and `state`.

`args.rs`: make `--scrollback-max-rows` required, and build the default title
from the argv after `--`.

### 3.4 Protocol additions (version-private, `climon2-core/src/protocol.rs`)

Keep the existing codes and framing. Extend the frame-type validation match and
the data-length rule.

| Code | Direction | Body | Meaning |
|---|---|---|---|
| `0x10` Attach | C->D | `{"cols","rows"}` | attach and take control (existing code, new semantics) |
| `0x11` Input | C->D | raw | application input (applied only if ready controller) |
| `0x12` Resize | C->D | `{"cols","rows"}` | remembered; applied only for controller |
| `0x13` TakeControl | C->D | `{}` | takeover request |
| `0x14` Ready | C->D | `{"period":u64}` | restoration written; enable input |
| `0x15` Detach | C->D | `{}` | explicit detach request |
| `0x16` Kill | C->D | `{}` | kill-mode request (instead of Attach) |
| `0x20` Attached | D->C | `{}` | attachment accepted (non-controlling until Granted) |
| `0x21` Output | D->C | raw | restoration, release or live bytes |
| `0x22` Exit | D->C | `{"code":u32}` | command finished (after release) |
| `0x23` Granted | D->C | `{"period":u64}` | this client controls; restoration follows |
| `0x24` RestoreEnd | D->C | `{"period":u64}` | restoration complete |
| `0x25` Revoked | D->C | `{}` | lost control (after release); show placeholder |
| `0x26` Detached | D->C | `{}` | detach accepted (after release) |
| `0x27` Terminated | D->C | `{"code":u32}` | kill confirmed after cleanup |
| `0x7f` Error | both | `{"message"}` | refusal / protocol error |

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
- The drain replaces `tcflush` (R2). A kernel flush could discard a paste's
  closing marker and leave the local paste state wrong. Reading through the
  filter keeps the marker tracking accurate.
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
- **Client state** (pure: role and current period), driven by server messages:
  - `Granted` → Pending.
  - After stdout has drained past `RestoreEnd`: run the readiness sequence
    (section 3.4), send `Ready` and become Controlling.
  - `Revoked`: after writing the preceding release bytes, draw the placeholder
    and become NonControlling.
- **Placeholder drawing**:
  1. Move to the bottom row and emit `rows` line feeds, preserving visible
     content.
  2. Emit `CSI H`, then the two placeholder lines.
  3. Redraw on SIGWINCH while non-controlling.
- **Exit ordering** (R3; section 1.7.1): on `Detached`, `Exit` or a refusal
  after output:
  1. Write all preceding `Output`, including release bytes, within the 5 s
     bound.
  2. End the placeholder if one is shown.
  3. Restore termios exactly.
  4. Print the stderr line or diagnostic, then exit with the status from
     section 1.1.

  On unexpected loss, write the conservative reset of section 1.7.1 instead.
- **Detach**: send `Detach` and wait for `Detached` (bounded, for example 5 s).
  Then run the exit ordering and exit 0. Without confirmation, exit 125.
- **Kill mode**: no raw mode. Send `Kill` and wait.
  - `Terminated{code}` → exit 0.
  - `Error` → exit 125.
  - Connection loss → exit 125.

### 3.6 CLI structure

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
- `list.rs`: add the VERSION, STATE and CLIENTS columns and the JSON `state`.

## 4. Increments

Each increment follows the same cycle: red test, green, refactor, gates,
commit, push.

- Run `cargo xtask test` at least once per increment, and run the quality gate
  on changed shipped source (section 7) before committing.
- Every increment is a passing, gated commit.
- *(O1)* The order below delivers one working attach/detach slice early (by
  increment 8) and then widens it. The slice is not permission to ship an
  incomplete feature, to merge failing increments or to add scaffolding solely
  for staging. The final acceptance (section 6) and the gates are unchanged.

1. **Baseline.**
   - Confirm the architecture (`uname -m`).
   - Select bash 5.x via `CLIMON2_TEST_BASH`, for example
     `/usr/local/bin/bash` or Homebrew bash.
   - Run `cargo xtask test`. Record pre-existing failures and environment gaps
     in the action log.
   - Commit nothing unless something is fixed.
2. **Bash 5-only interactive qualification (D7).**
   - In `climon2-test-term/src/shell.rs`, accept only GNU bash 5.x. Probe
     `CLIMON2_TEST_BASH`, then `/opt/homebrew/bin/bash`, then
     `/usr/local/bin/bash`, but not `/bin/bash`.
   - Update `climon2-fixtures/tests/bash.rs` and the packaged tests that
     iterate `bash_profiles()`.
   - Update the CI matrices: drop interactive `3.2` from both the `macos` and
     the `archive` jobs. **Also select a controlled bash 5 for the `quality`
     job**, replacing `CLIMON2_TEST_BASH: /bin/bash` at
     `.github/workflows/check.yml:75-80`. Use the same controlled-shell policy
     as the ordinary test jobs.
   - Update `README.md` and `crates/climon2-e2e/README.md`.
   - Keep `install.sh` and any installer or non-interactive `/bin/bash` uses
     unchanged, and keep them tested. Do not weaken any guard to make this
     migration green.
3. **Per-invocation client logs (R1).** Implement section 1.16 and update the
   dependent assertions.
   - Red: start A, then try to start a second client on the same session log
     directory. Today this fails on `create_new`.
   - Green: every invocation gets its own file, with the existing permissions,
     privacy and suffix rules.
4. **Core contracts (pure).**
   - `config.rs`: write a lexical normaliser that respects strings and escapes.
     It (a) removes `//` and `/* */` comments and (b) removes a comma whose next
     non-whitespace, non-comment character is `}` or `]`.
   - Then deserialize with `serde_json` into **operation-specific** structs:
     `ExecConfig { client, scrollback }` and `AttachConfig { client }`.
     - The attach struct does not name `scrollback`, so serde ignores it as
       unknown. An invalid, unused value therefore cannot fail attach.
     - serde derive rejects duplicate known fields. Use a three-state field type
       to distinguish missing, `null` and a value.
   - Test with the exact sample in section 1.3 (comments and trailing commas),
     and with an invalid `scrollback` during attach.
   - Then add the key vocabulary, selector parsing and matching, and the startup
     envelope.
   - Add hello extensions with lenient optional parsing: parse the mandatory
     `HelloReply` as today, and parse the optional fields separately from a
     `serde_json::Value`.
   - Add the state consistency rules and title encoding.
   - Write exhaustive unit tests, including the exact 4096-byte envelope
     boundary.
5. **Protocol messages.** Add the new codes and `ConnectionState` rules, with
   framing tests.
6. **`climon2-vt` (D6, D7, D3, R4, D1, D4).**
   - Create the crate and register it with the gates in one commit:
     - add it to `SHIPPED`;
     - update the architecture `PACKAGES` and dependency-edge guards;
     - amend ADR 0004;
     - add the new ADR (section 8).
   - Then build, test-first:
     - the tokenizer: split-across-reads cases, the cap, CAN/SUB, UTF-8
       boundaries and differential boundaries against vte;
     - the section 1.9 classification, including a supported query split across
       reads, unsupported requests, OSC 52, control-bearing titles and links,
       and a CPR-shaped sequence in output;
     - engine feed and replies;
     - the first-begin sync latch with an injected clock;
     - title tracking and live re-encoding;
     - the eviction flag;
     - a plain-text serializer with the session rectangle;
     - the incremental `RestoreCursor` with eviction hold and gap notices;
     - focused 0.26.0 seam checks, including many combining marks on one cell.
   - Rendition, alternate screen and modes may follow in increment 9.
   - The test oracle is described in section 5 (D8).
7. **Daemon state machine (pure).** Exhaustively test sections 1.5–1.7,
   including:
   - `DiscardPendingInput` and `ClosePaste` on every loss of admission;
   - `Release` before every `Revoke`, `AcceptDetach` and `DeliverExit`;
   - `RestoreDeadline`;
   - replacement grants before readiness.
8. **First continuity slice (O1).** Deliver one controller, ordinary text and a
   small history, end to end.
   - **Daemon**: continuous PTY reading, VT feeding and replies, grant and
     incremental restoration, `Ready` gating, detach with release and
     `Detached`, exit with release and `Exit`, unexpected final loss, the
     initial deadline, the restoration deadline, the required
     `--scrollback-max-rows` and hello extensions.
   - **Client**: envelope or config startup, the detach key, roles, the
     readiness drain and the exit ordering.
   - **CLI**: `attach` with selectors, capability and client-path checks,
     self-attach and the envelope.
   - Test through the real daemon binary with raw-socket fake clients, in the
     style of `crates/climon2-daemon/tests/lifecycle.rs`, and through
     `crates/climon2-client/tests/bridge.rs`.
   - Add a packaged detach/reattach smoke test. These tests expose R1–R3 early.
9. **Restoration breadth (D2, D3, D4, D5).**
   - SGR and colours, wide characters and combining marks, pending wrap.
   - The alternate screen, with exact E1 assertions.
   - Modes, title and the truncation notice.
   - Daemon-loop sync-latch integration.
   - Replacement restoration (prologue leaves this attachment's alternate
     screen) and the tall/wide host rectangle.
   - A plain prefix larger than 16 MiB.
   - An unfinished token at grant time, and a withheld `Ready`.
10. **Multi-client control (R2, R3).**
    - Takeover, hand-back and the placeholder.
    - Release at revocation, with no `1049 l` for placeholder clients.
    - Sizing.
    - Stale-period input.
    - The paste cleanup tracker under partial PTY writes.
    - Client paste-state persistence across role changes.
11. **Kill, shutdown, listing and the remaining CLI.**
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
12. **Packaged product tests** (section 6).
13. **Docs, native checks, full gates, push** (sections 8–10).

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
    visibility, mouse, bracketed paste) for release assertions.
  - Record whether the host was asked any query or clipboard operation.
- **Fixtures**: extend `climon2-fixture` and `climon2-tui-fixture` only as
  needed. Possible modes:
  - send DA1/DSR and print the reply received, with a timeout;
  - wait for an acknowledgement file or byte before printing, to produce output
    while detached deterministically;
  - write a large volume (tens of MiB) and report completion;
  - set a title, including control bytes;
  - emit an unfinished `ESC [` or OSC and then wait for input;
  - emit numbered output continuously across a restoration;
  - enable mouse, hidden cursor, alternate screen and bracketed paste, then
    exit without undoing them.
- Use acknowledgements and observable completion for ordering. Sleeps are never
  proof of readiness.

### 5.2 Test-design rules (D8)

- The roundtrip check stays as a compatibility check, but it is not the
  oracle. It feeds the source into `Vt`, feeds the prefix into a **fresh**
  Alacritty `Term` (a test dependency of `climon2-vt` is fine), and compares
  visible text, rendition, cursor, history rows and wrap flags.
- Add **literal expected** assertions for climon2's own renderer and transport:
  - exact expected screen text and positions;
  - expected input admission and ordering;
  - exact expected prefix bytes for small cases.

  They must fail if restoration is omitted, uses the wrong normal/alternate
  screen, loses a live suffix or enables stale input. An existing small
  independent oracle is optional. Do not build a second complete emulator or an
  exhaustive rendition suite.
- Exercise an **occupied host** and **two restorations on one attachment**: a
  pre-readiness resize, and a takeover followed by a hand-back.
- Use **numbered fixture phases with fresh acknowledgements**. Each phase prints
  a marker unique to that phase, and each wait looks for that phase's marker
  only after the phase begins. Accepted duplicate history means a marker that
  is already present in native history must never satisfy a new phase.

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
   - **Run it on an occupied host. Then detach and reattach a second time, and
     verify that each invocation has its own client log (R1).**
2. **Alternate screen.**
   - Run the TUI fixture (or `vim`/`less`) on the alternate screen, detach, and
     reattach. The alternate content is shown.
   - Quitting the TUI shows the restored normal content.
   - **Resize before `Ready` so that a replacement restoration runs; after the
     TUI exits, the current normal content and cursor-addressed output are
     correct (D4).**
3. **Multi-client.**
   - A execs and B attaches. A shows the placeholder with
     `Press Ctrl+T to take control`.
   - A takes control with Ctrl+T. B's input is discarded while it is
     non-controlling.
   - B detaches and A continues.
   - The controller's size drives the session; the fixture reports the PTY
     size.
   - Command exit returns the status to every attached client.
   - **Logs: start A, attach B, detach B, reattach C, then kill. Each
     invocation reaches its operation and has an independently usable log
     (R1).**
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
8. **Query authority and output policy (R4).** Disable harness replies.
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
     both **live** and after restoration.
   - A control-bearing OSC 8 link arrives as plain text, live and restored.
10. **Detached throughput.** A large write completes while detached, and the
    session stays live.
11. **Release (R3).** Use the TUI fixture with mouse reporting, a hidden cursor,
    the alternate screen and bracketed paste enabled:
    - **detach**;
    - **natural exit**, where the fixture exits without undoing its modes;
    - **kill**;
    - a **non-controlling departure**.

    After each client returns, verify ordinary shell input, the documented
    baseline (section 1.7.1) and termios restoration. Include a short or
    blocked stdout write on the client side.
12. **Interrupted paste (R2).**
    - Interrupt a fragmented bracketed paste by takeover and by client socket
      loss. Place both client-action bindings (Ctrl+] `d` and Ctrl+T) in the
      remaining payload.
    - Assert no accidental action and no stale payload.
    - Assert exactly one ordered application-paste closure when one is needed,
      and normal interpretation of B's subsequent input.
    - Also change readiness while a local paste is still streaming.
13. **Large restoration (D1).**
    - A plain-text history producing a prefix larger than 16 MiB reaches a
      healthy reader completely and in order.
    - Numbered output produced during restoration shows no lost suffix and no
      duplicated live span.
14. **Tall host (D4).** Restore into a host taller than 200 rows (if the harness
    allows; otherwise cover it in the daemon integration tests). Subsequent
    application cursor positioning lands in the session rectangle; check
    positions, not just the presence of markers.
15. **Existing tests keep passing**: vim, less, TUI queries/mouse/paste/resize,
    raw-socket abuse, version routing, log privacy and stale cleanup.
16. **Previous-release compatibility.** The existing
    `actual_previous_release_remains_live_across_current_archive_install` gate
    stays. No release exists yet, so it is a no-op locally. Never fabricate a
    previous archive. Synthetic controlled versions cover the
    unsupported-operation paths.

### 6.2 Daemon integration tests with raw-socket fake clients

These are required, and they are deterministic because they do not depend on
host timing.

- **Unfinished token (D2)**: the command writes `ESC [` (or an unterminated OSC)
  and then waits. A fake client attaches and receives a restoration and
  `RestoreEnd` without any further command output.
- **Withheld `Ready` (D2)**: after a small prefix has been fully accepted by the
  socket, `Ready` is never sent. The restoration deadline fires as an
  unexpected departure, and the leave policy applies.
- **Stalled reader (D1)**: a client that stops reading during a large prefix
  hits the 5 s no-progress deadline. A fast, quiet client succeeds.
- **Sync latch (D3)**: with repeated begins, the grant is served at most 150 ms
  after the **first** begin, through `tick` alone.
- **Replacement restoration (D4)**: the first prefix enters alternate (step 5
  delivered), then a resize starts period+1. The new prologue begins with
  `CSI ? 1049 l` exactly once.
- **Paste cleanup under partial PTY writes (R2)**: use a slow PTY reader to
  force partial writes. The cleanup bytes complete before any later admitted
  input.
- **Release ordering (R3)**: byte-exact order of release bytes, then
  `Detached`/`Exit`/`Revoked`. There is no `1049 l` for an attachment that never
  entered the alternate screen.

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
  that archive on both `macos-15` and `macos-15-intel` (R5). If the local host
  is Intel, rely on CI for `dist` and report it.
- Every gate runs under the controlled bash 5 policy, including the `quality`
  job (D7).
- For the new shipped crate:
  - add `crates/climon2-vt/src` to `SHIPPED` and update the architecture guards
    (section 3.1);
  - confirm that crapper and mutator report its functions.

  Never weaken thresholds or guards, add exclusions or mark mutants as
  acceptable.

## 8. Documentation updates

- **`README.md`**:
  - usage: attach, kill, keys, detach and takeover, the placeholder;
  - replace "Leaving the client ends the session; detach, reattach and screen
    restoration are not supported" with the new leave policy;
  - the configuration file and the list columns;
  - the shell matrix: bash 5.x interactive only, with `/bin/bash` installer
    compatibility.
  - limitations:
    - unmarked paste;
    - binding conflicts with applications;
    - native duplicates on reattach;
    - unrestored state (section 1.8), including the host title on exit;
    - the E1 saved-cursor exception;
    - the live output policy: stripped host queries, OSC 52, palette and other
      OSC, and plain-text hyperlinks;
    - self-attach detection only, with other cycles the user's responsibility;
    - legacy-only keyboard input;
    - what unexpected loss cannot restore;
    - 0.26.0's unbounded per-cell combining-mark storage.
- **`GLOSSARY.md`**: attachment, controller, control period, detach, take
  control, hand-back, restoration, restoration prefix, retained history,
  placeholder, leave policy, presentation release, capabilities, startup
  envelope.
- **`docs/adr/`**:
  - Add an ADR recording the daemon-owned virtual terminal:
    - Alacritty 0.26.0 in `climon2-vt`, with the pin justification and the
      evidence limits;
    - eager, incremental restoration, with the eviction-hold memory cost;
    - the live output policy and query authority;
    - presentation release;
    - zero-buffer synchronized output.
  - Amend the crate-layout ADR (0004) for `climon2-vt` **in increment 6**, with
    the crate itself.
- Crate READMEs: `crates/climon2-client/README.md` (per-invocation logs) and
  `crates/climon2-e2e/README.md`.
- Mark `docs/agents/session-continuity-handoff.md` as superseded by this plan.

## 9. Native sanity checks (character presence only; R5)

The checks run in four lanes, per [#45][acceptance] and its
[latest amendment][acceptance-latest]: **Terminal.app** and **VS Code**, each on
**native arm64** and on **native Intel** macOS.

- **Artifact identity**:
  - Every lane uses the **exact final universal archive** whose packaged gates
    passed. Record its SHA-256 and version.
  - Install it into an isolated `HOME`/`CLIMON2_HOME` and invoke the
    **explicit executable path** from that installation. Never use a bare
    `climon2` from `PATH`.
- **Script**: run `exec bash` and print markers unique to each phase. Detach,
  reattach, and confirm that the phase markers and the restored history are
  present.
- **Terminal.app**:
  - Open a fresh window via `osascript`, running the script.
  - Read the window text through AppleScript, using the `contents` and
    `history` of the run's own tab.
  - Close only that window.
- **VS Code**:
  - Run `code --new-window --user-data-dir <tmp> --extensions-dir <tmp>` with an
    integrated terminal running the same script.
  - Proceed only if text can be observed without screenshots or the clipboard.
    Otherwise record the lane as blocked.
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
  and application environment.
- **Fixed client log name.** `client.log` with `create_new` blocks the second
  client; use per-invocation logs (section 1.16).
- **PTY reading.** Today the daemon stops reading the PTY when the controller
  is slow. Continuity requires continuous reading, with the 16 MiB live queue
  and a separate, incrementally produced restoration prefix.
- **Frame-type validator.** It rejects unknown codes before reading the body.
  Add the new codes there, or their frames will be rejected.
- **`HelloReply` parsing.** It uses serde defaults, so unknown fields are
  ignored. Keep mandatory field parsing strict and optional extensions lenient.
  Bad optional data must never make a live session look stale, because stale
  cleanup deletes metadata.
- **Snapshot point.** Snapshot after a completed token, never mid-token, and
  outside the synchronized-update latch. Never wait for the front filter to
  have no partial token (D2).
- **Synchronized output.** Do not use the engine's default synchronized-update
  buffering: it re-arms on repeated begins and flushes up to 2 MiB in one call.
  Use the zero-buffer `Timeout` and the first-begin latch (D3).
- **Alternate-screen swap.** `Term::swap_alt` resets the alternate grid when
  entering alt mode, so always restore the cloned alternate grid afterwards. It
  also rewrites the normal saved cursor (E1), so assert E1 rather than an
  unchanged saved cursor.
- **Output passthrough.** Never forward raw OSC 52, raw titles, raw OSC 8 or
  host-dependent requests (section 1.9). Never filter input that looks like a
  reply.
- **Resets.** Resetting the input filter must not forget an in-progress local
  paste. Discarding input on transfer must not leave an application paste open
  (section 1.5).
- **Release.** Release is computed per attachment from what that attachment's
  host was actually sent, not from the session's engine state.
- **Logs.** They must not contain terminal content, titles, arguments or
  environment values.
- **Function size.** Keep each function tiny. Crapper counts `?` as a branch,
  and a test must kill every mutant of every delegation and constant.

## Appendix A. Critique disposition

| Finding | Severity | Disposition | Sections changed |
|---|---|---|---|
| R1 fixed client log blocks second client | P1 | Applied: per-invocation logs | 1.16, 2, 3.5, 4 (inc. 3), 6.1 (1, 3), 8, 11 |
| R2 filter reset vs interrupted paste | P1 | Applied: split key/paste state; daemon paste cleanup; drain replaces `tcflush` | 1.4, 1.5, 3.3, 3.4, 3.5, 4 (inc. 7, 10), 6.1 (12), 6.2, 11 |
| R3 release not wired to detach/exit | P1 | Applied: ordered per-attachment release on every confirmed exit; documented baseline | 1.1, 1.7, 1.7.1, 3.3, 3.4, 3.5, 6.1 (11), 6.2, 11 |
| R4 live path bypasses query/safe-output policy | P1 | Applied: one classification policy at the VT seam; OSC 8 plain text (plan choice) | 1.8, 1.9, 3.2, 4 (inc. 6), 6.1 (8, 9), 8, 11 |
| R5 CI is not the four native lanes | P2 | Applied: four lanes, exact archive, explicit paths, open lanes reported; CI description corrected | 2, 7, 9, 10 |
| D1 restoration overflows 16 MiB queue | P1 | Applied: incremental prefix separate from the live queue; eviction-hold mechanism (plan choice) with binding invariants | 1.8, 1.10, 3.2, 3.3, 6.1 (13), 6.2 |
| D2 grant waits outside the deadline | P1 | Applied: snapshot after the last completed token; one overall deadline through `Ready` | 1.8, 1.10, 3.2, 3.3, 6.2 |
| D3 sync recipe violates B5 | P1 | Applied: zero engine buffering via the `Timeout` seam; first-begin latch; 2 MiB exception removed | 1.10, 3.2, 6.2, 11 |
| D4 replacement restoration / tall host | P1 | Applied: per-attachment `Presentation`; prologue leaves own alternate; session rectangle at origin | 1.6, 1.8, 3.3, 6.1 (2, 14), 6.2 |
| D5 impossible saved-cursor assertion | P2 | Applied: E1 asserted exactly | 1.8, 3.2, 8, 11 |
| D6 pin vs qualified revision | P2 | Applied (user decision): ship 0.26.0 with justification; `bbb2cbf` evidence not carried over | 0.1, 3.1, 8 |
| D7 architecture guard and quality-job bash | P2 | Applied: guards and ADR 0004 in the crate commit; quality job on bash 5 | 2, 3.1, 4 (inc. 2, 6), 7, 8 |
| D8 roundtrip is not an oracle | P2 | Applied: literal assertions, occupied host, two restores, numbered phases | 5.2, 6.1 |
| O1 vertical slice first | P3 | Applied: increments reordered around a first continuity slice | 4 |
| O2 provenance without a ledger | P3 | Applied: source/supersession table; statuses; #44 additive freeze | preamble, 0.1, 1.15 |

[map-latest]: https://github.com/jackgeek/climon2/issues/40#issuecomment-6552102664
[engine-latest]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6552082936
[protocol-latest]: https://github.com/jackgeek/climon2/issues/48#issuecomment-6552082933
[acceptance-latest]: https://github.com/jackgeek/climon2/issues/45#issuecomment-6552082952
[native-latest]: https://github.com/jackgeek/climon2/issues/52#issuecomment-6552082929
[self-only]: https://github.com/jackgeek/climon2/issues/50#issuecomment-6548465337
[keypad-reduction]: https://github.com/jackgeek/climon2/issues/56#issuecomment-6551550924
[history-amendment]: https://github.com/jackgeek/climon2/issues/49#issuecomment-6531016859
[notice]: https://github.com/jackgeek/climon2/issues/49#issuecomment-6544545933
[paste-obligation]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6541685852
[cleanup-baseline]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6544545973
[unsafe-osc]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6545123700
[safe-osc]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6547004064
[restore-decisions]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6546458475
[acceptance]: https://github.com/jackgeek/climon2/issues/45#issuecomment-6551793762
[budgets]: https://github.com/jackgeek/climon2/issues/48#issuecomment-6546525973
[budget-followup]: https://github.com/jackgeek/climon2/issues/48#issuecomment-6547171245
[engine-research]: https://github.com/jackgeek/climon2/issues/46#issuecomment-6472870113
[embedding-research]: https://github.com/jackgeek/climon2/issues/53#issuecomment-6479019515
[provenance]: https://github.com/jackgeek/climon2/issues/56#issuecomment-6551493754
[cross-version]: https://github.com/jackgeek/climon2/issues/44#issuecomment-6547968217
[parser-source]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs
[engine-source]: https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/term/mod.rs
