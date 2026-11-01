# Reviewed session-continuity plan: adversarial critique

**Verdict: revise the restoration mechanism before integrating it.** The plan
still contains incompatible requirements and recipes: its prefix can emit setup
after history, its eviction hold changes what a later attachment retains, and
its clamped session rectangle cannot constrain unchanged live output. These are
ordinary product problems, not reasons to resume the abandoned engine research.

This independently reviews
[session-continuity-plan.reviewed.md](session-continuity-plan.reviewed.md),
not an implementation. Findings identify what an implementer would have to
correct or invent to execute that document. Evidence was read on 2026-11-01.
Plan line numbers refer to this reviewed revision. Suggested tests below have
**not** been implemented or run.

P1 blocks a promised behavior or essential invariant. P2 is a material
implementation correction. Opportunities are bounded improvements, not new
release requirements.

| Finding | Priority | Gap |
|---|---|---|
| F1 | P1 | The incremental recipe does not implement the specified restoration order |
| F2 | P1 | Eviction hold conflates snapshot retention with configured session history |
| F3 | P1 | Clamping geometry and forwarding live bytes unchanged cannot stay aligned |
| F4 | P2 | The client release timeout can be unreachable behind blocked stdout |
| F5 | P2 | The selected command-facing terminal identity is missing |
| F6 | P2 | Comparing history-row contents misses real eviction |
| F7 | P2 | Comment deletion can turn malformed JSONC into valid settings |
| F8 | P2 | The first continuity slice can regress existing exec before restoration breadth lands |

## Authority and review boundary

I read Wayfinder #40, all sixteen children (#41-#56), their comments, and the
inherited #27/#28 constraints. The relevant architectural records are ADRs
0001, 0003, 0004 and 0006. I also checked the current daemon/client transport,
session environment, discovery and quality-gate code, and the exact Alacritty
source revision cited by the plan.

The latest confirmed amendments govern this review:

- [#40][map-latest] permits implementation now.
- [#43][engine-latest] fixes Alacritty as the engine, permits plain-text
  hyperlinks, and replaces exhaustive qualification with focused product
  checks. Detailed saved-state/nonstandard-cursor investigations are not
  implementation blockers.
- [#48][protocol-latest] withdraws elaborate hyperlink accounting and
  qualification. It approves neither a new restore-size cap nor a particular
  serialization strategy or seven-second query delay. Ordinary budgets and
  control/leave semantics remain.
- [#45][acceptance-latest] and [#52][native-latest] simplify native observation,
  not automated product testing or the retained host/architecture lanes.
- [#49][history-amendment] removes custom history-byte accounting and
  incremental catch-up; its [notice amendment][notice] requires coarse
  truncation presentation.
- [#50][self-only] requires only identified direct self-rejection.
  [#56][keypad-reduction] removes special application-keypad fidelity, not
  ordinary digits or supported cursor/paste alignment.

Accordingly, this critique does not request another engine, engine patches,
indirect-cycle machinery, a client emulator, extended-key translation, lossless
hyperlinks, stable history identities, total-memory accounting, exact native
de-duplication, or elaborate native observers. Accepted history duplicates and
the reason-sensitive leave policy are not findings.

## Findings

### F1 - P1: the prefix recipe puts prerequisites after their consumers

**Location:** section 1.8, lines 469-549; section 3.2, lines 1148-1156 and
1181-1199.

Section 1.8 requires this order: prologue, preservation of visible outer content,
notice, normal history, normal screen, cursors, alternate screen, modes and
title. The incremental mechanism instead defines an eager *tail* containing
the prologue and steps 4-7, produces history first, and sends that tail
afterward. It assigns no production phase to step 1, or to the normal-screen
rows and post-history scrolling in step 3. The alternate-access subsection
says to serialize the normal screen, but does not reconcile that omission.

This is not cosmetic naming. On a replacement restoration whose host is still
alternate, history emitted before `1049l` goes to the alternate buffer rather
than native normal history. On a host with origin mode or non-default margins,
history emitted before the prologue relies on precisely the state that section
1.8 says must first be reset. Repainting before preserving outer content also
defeats the preservation requirement.

**Smallest correction:** make the production recipe follow section 1.8
literally. Separate the leading setup/preservation bytes, streamed history, and
the captured normal-screen/cursor/alternate/mode/title suffix. State where every
step is captured and emitted. There is no need for another rendering
abstraction; this is an ordering correction to the existing renderer.

**Decisive test:** prime an occupied host, complete an initial alternate
presentation, then request a replacement restoration. Assert that its alternate
exit and setup precede any restored history, that outer sentinels survive in
normal native history, and that both normal and alternate screen rows appear in
their specified phases. Include a small normal-only case with literal expected
bytes so a missing normal-screen phase cannot hide behind a roundtrip.

### F2 - P1: the eviction hold changes session retention, not just snapshot storage

**Location:** sections 1.3 and 1.8; section 3.2, lines 1136-1177; section 3.3,
lines 1297-1308.

The mechanism raises the authoritative engine's normal-history capacity from
`max_rows` to `max_rows + H` while a prefix exists. It restores the configured
capacity only when no `RestoreCursor` remains. But another grant can snapshot
the engine before the old restoration finishes; the grant recipe even creates
the new cursor before abandoning the previous one.

Concrete counterexample: start a restoration with `max_rows: 0`, leave it in
progress, and produce normal-screen scrolling. The temporarily increased
capacity retains those rows. A takeover or replacement snapshot can now record
`h0 > 0` and replay history, although zero explicitly disables retained history.
For a small positive limit, the same path can replay more rows than configured.
There is no specified logical-retention window excluding the extra held rows.

The exact public API confirms the distinction:
[`Term::set_options`][engine-options] changes the real normal grid's history
capacity, including when normal is inactive;
[`Grid::update_history`][grid-history] sets its actual scroll limit and removes
rows when necessary. It does not create independent snapshot storage.

There is a second inconsistency: the declared prefix describes state at S, but
the recipe skips unsent S rows after trimming and ends history early after a
live history clear. A notice makes truncation explicit; it does not make the
result the complete snapshot at S that the invariant promises.

**Smallest correction:** distinguish rows retained for the current session
under its configured limit from rows held only to finish an earlier snapshot.
Define the retained range at every new S, including zero history and replacement
grants, and retire the old restoration before it can influence that range.
Resolve whether post-S eviction may alter the prefix; do not claim both an
immutable S snapshot and a moving, truncated history view.

This does not justify a new cap, full-history clone requirement or history-ID
subsystem. The plan choice must meet the retention and cutover contract; its
current eviction-hold recipe has not established that.

**Decisive tests:** with a deliberately unfinished prefix, feed more scrolling
and take a second snapshot at limits 0 and 2. Assert respectively no history and
at most the configured retained rows. Separately force explicit trimming and
history clear between production slices and check the chosen snapshot/cutover
semantics against literal numbered rows, not only final screen text.

### F3 - P1: a top-left rectangle does not constrain raw live scrolling or wrapping

**Location:** sections 1.6 and 1.8 step 3, lines 505-514; section 1.9 class 8,
lines 647-649; section 6.1 scenario 14.

The plan clamps the PTY/engine to 500x200 but leaves the host at its real size.
Absolute row positioning makes the initial restored image land correctly.
It does not change the host's wrap width or bottom scrolling boundary.

Two ordinary continuations demonstrate the mismatch:

- On a 501-column host with a 500-column session, print 500 narrow characters
  followed by another character. The engine wraps that next character to the
  following row; the host prints it in column 501.
- On a 201-row host with a 200-row session, put the cursor on session row 200
  and send LF plus text. The engine scrolls at its bottom; the host advances
  into row 201. Cells outside the promised rectangle no longer stay blank.

These require neither exotic cursor state nor pixel inspection. Unchanged live
bytes execute against different geometries. The existing tall-host acceptance
case checks subsequent *cursor positioning*, which misses both failures.

**Smallest correction:** specify how continuation stays aligned when host and
session dimensions differ. Reuse daemon-owned VT/rendering state where a
geometry mismatch requires derived output, or explicitly reopen the clamp
policy if unchanged passthrough is indispensable. Do not assert that CUP alone
solves the problem, and do not add a client emulator.

**Decisive tests:** use 501x24 and 80x201 hosts, reach readiness, then exercise
ordinary wrapping and bottom-row LF. Assert literal engine/host row and column
results and history behavior after the live suffix, not just restoration
positions.

### F4 - P2: the five-second client release deadline needs a reachable trigger

**Location:** section 1.7.1, lines 404-414; sections 3.3 shutdown and 3.5 exit
ordering; current `crates/climon2-client/src/bridge.rs:173-210`.

The plan tells the client to drain preceding output within five seconds on
`Detached`/`Exit`/refusal. The current bridge reads server messages only when
its stdout buffer is empty. With a blocked stdout write, it cannot consume the
terminal message queued behind that output. Its socket polling can also be
disabled while output is pending. Merely adding a timeout in the terminal-message
handler therefore leaves an infinite wait before that handler is reached.

The daemon's bounded final flush and subsequent socket close do not by
themselves fix a client that is not observing the socket. Nor does a socket
write-progress deadline prove local stdout progress.

**Smallest correction:** keep termination/connection-loss observation and
deadline servicing alive during stdout backpressure. Define when the local
cleanup bound starts when a terminal message is not yet consumable. On expiry,
restore termios and report the unconfirmed/incomplete result honestly, without
unboundedly buffering the rest of the stream.

**Decisive test:** block the client's stdout with preceding output pending,
queue `Exit` and close the fake daemon connection. Verify bounded return,
termios restoration and the honest failure result. A test that first delivers
`Exit` to the handler does not exercise the missing trigger.

### F5 - P2: the command-facing terminal identity was omitted

**Location:** sections 2, 3.1 and 3.6; current
`crates/climon2-core/src/session.rs:29-58`.

The [confirmed engine contract][engine-contract] selects
`TERM=xterm-256color` and `COLORTERM=truecolor` for the command-facing terminal.
The latest simplification does not revoke that identity. The plan specifies
Alacritty configuration and query authority but gives no implementation step
for the environment contract.

Today `SessionEnvironment::for_session` preserves an inherited TERM and only
defaults it if absent; it does not establish COLORTERM. Following the code map
and CLI changes as written can therefore leave applications selecting a
different terminal description from the daemon's chosen profile.

**Smallest correction:** explicitly implement the selected identity in the
daemon/application environment projection and update dependent environment
expectations. Keep the invoking/client environment's enclosing-session identity
unchanged; this is not a reason to disturb the self-attachment handoff.

**Decisive test:** start through public exec with conflicting inherited TERM and
COLORTERM values. The fixture must observe the selected command-facing values;
unrelated inherited environment and enclosing-session behavior remain intact.

### F6 - P2: content comparison cannot establish history eviction

**Location:** section 3.2, lines 1119-1128.

The only specified steady-state eviction signal is a difference between the
old and new topmost rows while history is full. Repeated identical lines can
replace the oldest row indefinitely without changing its content. An ordinary
`yes`-like stream is sufficient. The proposed algorithm then fails to set
`evicted`, so the [required coarse notice][notice] can be absent on reattachment.

This is not a request to restore stable history identity. Equality of text and
rendition simply does not prove that no row was removed.

**Smallest correction:** base the coarse flag on observable row-removal/scroll
effects at the adapter boundary, including resize and explicit trimming,
rather than content inequality. Retain the specified reset on history clear.

**Decisive test:** overflow a two-row history with identical hard-ended lines,
then restore. Expect the notice despite equal oldest-row contents. Clear
history afterward and check that the stale notice does not persist.

### F7 - P2: JSONC normalization must preserve token separation

**Location:** section 4 increment 4, lines 1499-1508.

The lexical normalizer is told to remove comments before deserializing. Literal
deletion can join tokens. For example:

```jsonc
{"scrollback":{"max_rows":1/* comment */0000}}
```

Deleting the comment produces `10000`, silently accepting a setting from
malformed input. Comments occupy whitespace positions; they do not authorize
joining number or keyword fragments. Respecting strings and escapes alone does
not prevent this error.

**Smallest correction:** replace comments with whitespace, preserving
separation, then remove genuine trailing commas. State this in the recipe.

**Decisive tests:** reject the split-number example and `tr/* comment */ue`;
accept comments between otherwise valid tokens and inside the supplied sample.
These are focused boundary tests, not a new configuration framework.

### F8 - P2: the thin slice is not automatically a passing replacement for exec

**Location:** section 4, lines 1457-1465, increment 6 and increments 8-9;
section 6.1 scenario 15.

Increment 8 integrates continuous PTY consumption and restoration for the
initial exec attachment, but its supported slice is ordinary text and small
history. Rendition, alternate screen and modes may arrive in increment 9.
Every increment must also preserve existing behavior, pass all tests and be
pushed to main.

Those commitments conflict unless existing exec restoration breadth is present
at the integration point. A TUI can enter alternate screen and finish its
initial display before the first client attaches. The new daemon has already
consumed that output; an ordinary-text restoration cannot replace the previously
buffered live display. Assuming the application redraws later is not a
preservation strategy.

**Smallest correction:** complete the restoration features needed by existing
exec behavior before switching that path to the new pump. Move that work into
the integration increment, or finish it in the library first. Keep new
multi-client and kill breadth separate. No temporary product flag or weakening
of the existing gates is needed.

**Decisive test:** hold the initial attachment until a TUI fixture has completed
its alternate-screen display and is waiting for input. Initial exec must show
the display and apply its input modes without another application redraw.
Run this at the first integration commit, not only after increment 9.

## Bounded opportunities

### O1 - Specify period retirement before adding more queue machinery

Section 3.3 says to abandon an earlier `RestoreCursor`, but that does not state
what happens to its already-produced frames or its queued live suffix. It also
tracks `Presentation` when bytes are queued, not when the host has written them.
That can be valid **if** all earlier queued state transitions remain in FIFO
order. Removing an unsent alternate-entry frame while retaining its tracked
state would invalidate the replacement prologue.

Choose one retirement rule and test it: preserve the earlier queued bytes ahead
of the replacement, or cancel only removable unsent frames while preserving any
partially written frame and accounting for the transitions actually retained.
Do not add input-period tags merely to solve this output problem; the existing
FIFO-plus-Ready argument is a separate mechanism.

A focused case with a partially sent frame, a queued alternate entry and a
replacement grant is more useful than another broad list of race scenarios.

### O2 - Carry the confirmed failure boundary into the design

The [#43 cleanup/failure baseline][cleanup-baseline] and
[#48 protocol baseline][protocol-baseline] distinguish recoverable
attachment/rendering failure from authoritative VT integrity loss. The former
isolates the failed transfer where safe; the latter terminates explicitly.

The plan links that baseline but omits the distinction from its product rules
and state-machine inputs/effects. Its suggested VT surface is infallible.
Before implementing fallible paths, give each real failure a destination:
rejected admission, accepted attachment loss/hand-back, or authoritative
shutdown. Do not invent failure variants for operations that cannot fail.

This is a small completion of the error contract, not a request for
catch-unwind containment, recovery emulation or a speculative error framework.

### O3 - Reuse the corrected native setup, not the abandoned observer programme

Section 9 restates VS Code launch isolation as user-data and extensions
directories. [#56 already records][vscode-isolation] a launch that reached
default shared storage despite an isolated user-data profile, followed by
explicit shared/agent-data isolation corrections. Its
[later input report][vscode-history] also records shell-history isolation
corrections.

Reuse those verified setup safeguards for the simple character-presence run;
do not rediscover them by interacting with the user's default profile. Reusing
setup is not permission for clipboard access, old-window control, or the
abandoned elaborate native observer.

### O4 - Make resize coalescing an instruction, not an option

Section 1.6 says intermediate resizes *may* be coalesced. [#48 B8][budgets]
already says back-to-back resizes **must** be merged; its
[latest amendment][protocol-latest] retains ordinary approved budgets.
Use one pending controller size and apply the latest before a reflow/restoration
decision. A focused test that a queued burst causes one reflow is sufficient;
do not reopen the old comparative performance programme.

## Recommended revision order

1. Correct the prefix phases and settle snapshot/configured-retention semantics
   (F1-F2). These determine the data shape of `RestoreCursor`.
2. Reconcile oversized-host continuation with the approved clamp (F3).
3. Wire reachable client cleanup and the omitted environment/eviction/config
   boundaries (F4-F7).
4. Adjust the first integration increment so it preserves existing exec
   behavior (F8), with explicit retirement and failure outcomes at that seam.

Do not mark Appendix A's D1/D4 dispositions as fully resolved merely because
the reviewed text now names an incremental cursor and a session rectangle.
They need mechanisms that meet their stated invariants.

## Review action log

- Read repository guidance, domain vocabulary, relevant ADRs and the complete
  reviewed plan; consulted the prior critique to distinguish remaining gaps
  from already-addressed findings.
- Read the current Wayfinder map, all child issue bodies/comments and inherited
  constraints; applied later confirmed amendments rather than older prototype
  requirements.
- Checked current daemon/client backpressure, shutdown, environment and discovery
  paths, and verified the retained shipped-code quality gate.
- Checked the exact Alacritty public history-capacity and alternate-swap source
  cited by the plan; used that evidence to bound the restoration critique.
- Wrote this critique only. No product code, dependency, source plan, existing
  staged document, GitHub issue, native window or release state was changed.

[map-latest]: https://github.com/jackgeek/climon2/issues/40#issuecomment-6552102664
[engine-latest]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6552082936
[protocol-latest]: https://github.com/jackgeek/climon2/issues/48#issuecomment-6552082933
[acceptance-latest]: https://github.com/jackgeek/climon2/issues/45#issuecomment-6552082952
[native-latest]: https://github.com/jackgeek/climon2/issues/52#issuecomment-6552082929
[history-amendment]: https://github.com/jackgeek/climon2/issues/49#issuecomment-6531016859
[notice]: https://github.com/jackgeek/climon2/issues/49#issuecomment-6544545933
[self-only]: https://github.com/jackgeek/climon2/issues/50#issuecomment-6548465337
[keypad-reduction]: https://github.com/jackgeek/climon2/issues/56#issuecomment-6551550924
[engine-contract]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6537394310
[cleanup-baseline]: https://github.com/jackgeek/climon2/issues/43#issuecomment-6544545973
[protocol-baseline]: https://github.com/jackgeek/climon2/issues/48#issuecomment-6544545850
[budgets]: https://github.com/jackgeek/climon2/issues/48#issuecomment-6546525973
[vscode-isolation]: https://github.com/jackgeek/climon2/issues/56#issuecomment-6549823458
[vscode-history]: https://github.com/jackgeek/climon2/issues/56#issuecomment-6550963985
[engine-options]: https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/term/mod.rs#L498-L516
[grid-history]: https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/grid/mod.rs#L153-L161
