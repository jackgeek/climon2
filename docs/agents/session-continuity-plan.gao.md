# Session continuity plan: adversarial critique

**Verdict: needs revision.** Implementation can start, but the proposed mechanics
are not yet a reliable implementation contract. Existing client-log creation
blocks reattachment, interrupted paste can contaminate the next control period,
and restoration can overflow its own queue or wait indefinitely before its
delivery deadline starts. Several smaller omissions would also prevent the
promised passing increments.

This reviews [session-continuity-plan.md](session-continuity-plan.md), not an
implementation or the plan author's intentions. Evidence was read on
2026-11-01, including the Wayfinder map, its child issues and their comments.
References to plan sections and lines identify the reviewed document; they are
not instructions to implement the recommendations in this review.

P1 blocks an intended outcome or violates an essential safety/lifetime invariant.
P2 is a material correction. P3 is an optional simplification. Proposed tests
below are **not performed tests**.

## Authority and scope

The latest confirmed amendments matter more than older issue bodies or prototype
requirements. In particular:

- [#40's implementation-start amendment][map-latest] and
  [#43's final-engine/testing amendment][engine-latest] allow implementation now:
  unchanged Alacritty, ordinary product checks, safe title encoding and plain-text
  hyperlink degradation.
- [#48's latest amendment][protocol-latest] rejects pathological hyperlink
  accounting and further elaborate restoration qualification. It does **not**
  approve a serialization strategy, new restore-size cap or seven-second query
  delay. Previously approved ordinary budgets remain.
- [#45's latest amendment][acceptance-latest] and
  [#52's native amendment][native-latest] remove exhaustive native observers and
  pixel-perfect checks, not automated product testing, exact-archive
  compatibility or the agreed host/architecture scope.
- [#50's replacement resolution][self-only] requires only identified direct
  self-attachment rejection. [#56's keypad amendment][keypad-reduction] excludes
  special application-keypad fidelity, not ordinary digits or cursor/paste
  alignment.
- [#49's retention amendment][history-amendment] removes custom history-byte
  accounting and incremental catch-up. Its [later notice decision][notice]
  explicitly requires the coarse truncation notice.

Accordingly, this review does **not** ask for another engine, engine patches,
indirect-cycle detection, a client emulator, extended keyboard translation,
history identity/accounting, lossless hyperlinks, exact native de-duplication,
pixel sampling or a renewed research programme. Their absence is not a finding.
Nor are accepted history duplicates or the reason-sensitive leave policy bugs.

## Requirements findings

### R1 - P1: the existing log path prevents the second client from starting

**Location:** plan sections 1.1, 3.5 and 4 increment 8; current
`crates/climon2-client/src/lib.rs:81-101` and
`crates/climon2-platform/src/logging.rs:40-47,83-99`.

**Problem and impact:** initial `exec` creates
`<home>/logs/<session-id>/client.log`. Every later client calls the same
`LogFile::session(..., Role::Client, ...)`, whose file is opened with
`create_new(true)`. That file remains after detach. Reattachment therefore fails
before hello, and a concurrent second attachment fails for the same reason.
Kill mode will inherit this blocker too if routed through the unchanged startup
logging path.

The plan changes client dispatch and attachment semantics without addressing
this existing single-client assumption. Its first continuity and multi-client
acceptance scenarios cannot pass if this path is retained.

**Evidence and confidence:** high; this is a direct read of the startup call and
file-open policy, not a hypothetical engine failure. Existing tests also assume
the fixed name: `crates/climon2-client/tests/bridge.rs:480,941` and
`crates/climon2-e2e/tests/packaged.rs:420,792`.

**Smallest correction:** give each client invocation its own diagnostic file in
the existing session-log directory. Preserve content-free records, retention,
permissions and diagnostic-directory suffixes; do not truncate, delete or share
the earlier client's log. Update directly dependent log assertions.

**Validation:** start A, attach B, detach B, reattach C and kill through the
matching client. Each invocation must reach its intended operation without a
log-name collision, and all client diagnostics must remain independently usable.

### R2 - P1: resetting the input filter does not safely interrupt a paste

**Location:** sections 1.4-1.5, 3.3 input effects, 3.4 lines 727-733 and 3.5.

**Problem and impact:** A forwards a bracketed-paste opening marker and some
payload. B takes control, or A disappears. Dropping A's unwritten input also
drops its closing marker. The application can remain inside paste when B's
ordinary input arrives. Depending on the application, that input is treated as
pasted data rather than commands or shortcuts.

There is a second side to the same problem. Unconditionally resetting the
client's filter on `Granted` and `Revoked` forgets that a paste is still arriving.
Its remaining payload can now contain apparently actionable Ctrl+] `d` or
Ctrl+T, or be forwarded after readiness as if newly typed. Flushing the terminal
input queue does not remove bytes that arrive later in a long streaming paste.

**Evidence and confidence:** high. This obligation was explicitly confirmed in
[#43's legacy-input refinements][paste-obligation]: the daemon must finish an
already-started application paste before later admitted input, discarding stale
payload. The accepted prototype exposes `FinishPaste`; the plan has no
corresponding daemon obligation and specifies destructive filter resets.

**Smallest correction:** distinguish resetting pending key/header state from
discarding the remainder of an interrupted paste. Record enough ownership to
close an application paste on control/admission loss, including abrupt client
loss. That closing marker is cleanup, not stale application payload to discard.
It must precede the next controller's input, including under partial PTY writes.
No key encoder or client emulator is needed.

**Validation:** interrupt a fragmented paste by takeover and by socket loss;
place both client-action bindings in the remaining payload. Assert no accidental
action, no stale payload, one ordered application-paste closure when needed, and
normal interpretation of B's subsequent input. Also exercise readiness changing
while a paste is still streaming.

### R3 - P1: terminal release is defined but not wired to detach or completion

**Location:** sections 3.3 lines 677,686-694 and 3.5 lines 749-762.

**Problem and impact:** the only explicit daemon path that queues `vt.release()`
is revocation. Detach acceptance is described as an acknowledgement, and shutdown
as sending `Exit` or `Terminated` after cleanup. The client is merely told to
write release bytes **if the daemon sent them**.

Detach or kill a full-screen application while mouse reporting, hidden cursor
or alternate screen is active. Restoring termios alone does not restore those
terminal-emulator modes. The resumed outer shell can remain hidden or receive
mouse/paste traffic it did not request. A command can also exit without undoing
its modes. Non-controlling clients can subsequently depart without another
revocation to reset their locally drawn presentation.

**Evidence and confidence:** high as a missing effect/order contract. The plan
already supplies release bytes, so this is not a request to recover arbitrary
unknown host state. [#43's cleanup baseline][cleanup-baseline] explicitly permits
a documented normal-shell mode baseline while requiring captured OS settings
to be restored exactly.

**Smallest correction:** require ordered presentation release on every
daemon-confirmed attachment exit, before `Detached` or `Exit`, and finish local
cleanup before returning or diagnosing. Use attachment-appropriate state:
do not send an alternate-screen exit to a placeholder client merely because the
session's engine is currently alternate. Keep unexpected-loss cleanup bounded
and honest about what can be restored without a daemon response.

**Validation:** detach, natural exit and kill while the TUI fixture has modes
enabled. After each client returns, verify ordinary shell input and the documented
mode baseline as well as termios restoration. Include a non-controlling departure
and a short/blocked stdout write. A release function unit test alone does not
exercise this wiring.

### R4 - P1: the live path bypasses the claimed query and safe-output policies

**Location:** sections 1.8.1, 1.9 lines 346-360 and 3.2 event handling.

**Problem and impact:** queries unsupported by the engine are deliberately
passed to the host, including colour/pixel/DCS requests and OSC 52 reads.
The resulting terminal input has no distinct origin in this raw client bridge:
it can reach the application as ordinary input. Answers also change or disappear
when control moves or the session detaches.

This is the opposite of the confirmed no-in-band-host-query direction and
daemon-owned query authority. It is particularly inappropriate to solve the
ambiguity by dropping response-looking input: the accepted evidence demonstrates
that a CPR-shaped sequence can also be a legitimate key encoding.

The same bypass affects other output. Ignoring engine `ClipboardStore` events
or configuring `Osc52::Disabled` does not stop a forwarded raw OSC 52 from
reaching the host. Forwarding raw OSC 0/2 bypasses the plan's title encoder.
Forwarding arbitrary live OSC 8 bypasses the approved safe control-byte handling;
the 4096-byte cap does not sanitize a short payload.

**Evidence and confidence:** high. See the [no-host-query and collision
decision][paste-obligation], [confirmed output/failure baseline][cleanup-baseline]
and [latest safe-title/plain-text-link amendment][engine-latest].
[#43's OSC 8 experiment][unsafe-osc] demonstrated C1-bearing reconstruction
erasing an xterm-family host; the [follow-up][safe-osc] demonstrates the
mechanical encoding seam. This review does not claim a new executed exploit.

**Smallest correction:** specify one live-output policy at the VT seam. Supported
application queries terminate at the daemon; unsupported host-dependent requests
must not silently recruit the host. Exclude clipboard operations from forwarded
bytes, encode titles before emitting them, and either mechanically encode live
hyperlink controls as approved or render their text without link metadata.
Keep legitimate reply-shaped key input. Do not add host-query correlation or a
custom keyboard translator.

**Validation:** cover a supported query split across reads, an unsupported
host-dependent request and OSC 52, with harness auto-replies disabled. Verify
exactly the intended daemon reply and no host query/clipboard side effect.
Exercise control-bearing live titles/links as well as restoration, and preserve
the legitimate CPR-shaped key case.

### R5 - P2: green CI is not the retained four-lane native acceptance

**Location:** sections 6, 7, 9 and 10.

**Problem and impact:** section 9 runs bare `climon2` in two local hosts without
identifying its archive/version. Those checks can exercise an older PATH
installation rather than the final artifact. Section 10's arm64/Intel CI checks
do not establish native Terminal.app and VS Code behavior on both architectures.

The plan can therefore report its native checks and CI green while missing the
retained native lanes or testing different binaries.

**Evidence and confidence:** high. [#45's approved release lanes][acceptance]
require the exact final universal archive in Terminal.app and VS Code on native
arm64 and Intel macOS. Its [latest amendment][acceptance-latest] explicitly retains
the host/architecture scope while simplifying the observer. Current
`.github/workflows/check.yml:117-242` builds one universal artifact on arm64 and
tests its archive on both architectures; it does not run those native hosts.
The plan's statement that the `dist` job runs on both architectures is also
incorrect.

**Smallest correction:** use explicit executables from an isolated installation
of the identified final archive. Record artifact identity and host/architecture
for each simple native check. Keep unavailable lanes open rather than treating
ordinary CI as their substitute. No pixel, selection or elaborate observer gate
is requested.

**Validation:** four explicit character-presence/product-sanity records, or
plainly recorded outstanding lanes. Confirm each check used the same final
archive whose packaged gates passed.

## Design-quality findings

### D1 - P1: a valid restoration can overflow its own 16 MiB queue

**Location:** sections 1.10, 3.2 `restoration() -> Vec<u8>` and 3.3 lines 674-676.

**Problem and impact:** the plan builds the complete restoration and enqueues
all its `Output` frames before `RestoreEnd`. The same plan caps the attachment's
outbound queue at 16 MiB. Splitting a large prefix into 64 KiB frames does not
reduce its total queued bytes.

This is not a pathological hyperlink case. At the allowed 100,000 retained rows,
200 plain characters per hard-ended row alone produce about 20,000,000 bytes,
before line endings, screen, rendition and framing. That exceeds 16,777,216
bytes. A healthy new attachment can therefore be disconnected solely because
its legitimate history is larger than the queue. If it is the final attachment,
the prescribed unexpected-loss policy then terminates the session.

Building the whole prefix synchronously also occupies the single PTY/control
pump. A 64-byte engine feed limit does not bound serialization time.

**Evidence and confidence:** high; the queue contradiction is arithmetic.
[#48's follow-up][budget-followup] already separates a restoration prefix from
the live queue and measures pump stalls. Its [latest amendment][protocol-latest]
withdraws elaborate qualification, not the need to choose correct ordinary
restore/live coordination.

**Smallest correction:** define how restoration larger than the queue is produced
and drained incrementally, with a consistent snapshot/live cutover and bounded
ordinary pump/query delay. Distinguish total prefix size from outstanding queued
bytes. Do not add a new restore-size cap, silently truncate valid history,
increase the approved live queue, or assume a worker/clone is required.

**Validation:** a plain-text prefix larger than 16 MiB must reach a healthy
reader completely and in order. Produce numbered output across restoration and
verify neither a lost suffix nor a duplicated live span. A stalled reader must
still hit the existing deadlines and leave policy.

### D2 - P1: the grant can wait forever outside the restoration deadline

**Location:** sections 3.2 `at_boundary()`, 3.3 lines 674-681 and 3.4 readiness.

**Problem and impact:** the front filter buffers incomplete tokens and only
feeds completed tokens to the engine. A grant waits for that filter to have no
partial token. A command that writes a short unfinished `ESC [` or OSC and then
waits for input can leave the grant pending indefinitely. It never exceeds the
4096-byte cap. The old controller has lost control, the new one is not ready,
and the delivery deadline has no queued restoration to measure.

The other uncovered interval is after a small restoration has drained into the
socket but before the client has written it to stdout and sent `Ready`. Socket
write completion is not client readiness. A quiet command provides no subsequent
queue overflow to end that wait.

**Evidence and confidence:** high for the token-boundary wait; the readiness
interval is a material ambiguity in the stated deadline, not a claim that every
implementation will omit it. Initial attachment acceptance satisfies the separate
startup obligation and therefore does not bound either wait.

**Smallest correction:** since partial front-filter tokens have reached neither
engine nor host, consider restoring the engine's last completed state while
retaining that pending token for later completion. Do not conflate a pending
source token with an unsafe engine/host checkpoint. Define the existing overall
restoration deadline across preparation, delivery and current-period `Ready`,
with socket progress and client completion distinguished.

**Validation:** attach after a command emits an unfinished token and waits; the
client must not remain permanently unready. Separately withhold `Ready` after a
small prefix has been accepted by the socket. Verify the bounded failure and
ordinary leave policy without requiring further command output.

### D3 - P1: the synchronization recipe does not enforce the approved deadline

**Location:** section 1.10's synchronized-output row and section 3.2 parser/tick.

**Problem and impact:** the plan uses the engine's default timeout, waits until
it expires, and calls `stop_sync` on up to its 2 MiB accumulated buffer. It calls
this an accepted exception and claims restoration waits at most about 150 ms.

The default vte timeout is extended by another begin-update. Repeated begins
can postpone a grant indefinitely. Flushing accumulated semantic work in one
call can also monopolize the single pump; feeding it in earlier 64-byte slices
does not slice the work performed by `stop_sync`.

**Evidence and confidence:** high. The exact locked [vte 0.15.0 source][parser-source] resets
the timeout on repeated begins (`src/ansi.rs:410-411,459-460`) and processes the
buffer synchronously (`:327-339`). [#48's approved B5 decision][budgets] calls for
zero engine buffering and an adapter deadline from the **first** begin.
The [latest amendment][protocol-latest] retains approved ordinary budgets; it
does not authorize the new 2 MiB exception.

**Smallest correction:** reconcile the recipe with B5 using the already permitted
public timing seam and first-begin latch. Keep engine work scheduling separate
from host synchronized-presentation state. If a different exception is actually
needed, label it as an unapproved tradeoff rather than a settled requirement.
Do not promise a hard wall-clock CPU bound.

**Validation:** deterministic first begin, repeated/fragmented begin, explicit
end, timeout and rearming cases through the daemon loop. A pending grant must
become serviceable without another PTY read after the deadline.

### D4 - P1: a replacement restoration is not defined from the host's real state

**Location:** sections 1.6, 1.8 steps 0-5 and 3.3 `Grant`.

**Problem and impact:** pre-readiness resize explicitly starts another period
and restoration on the same connection. If the previous restoration has already
entered alternate screen, the next prologue does not leave it. The recipe then
renders the new normal history/screen into the **active alternate screen**.
Its later `1049h` can be a no-op because the host is already alternate.
Returning to normal exposes the old underlying state, not the newly restored
normal state.

A related coordinate assumption is left unspecified when host height exceeds
the 200-row clamp. Preserving/streaming from the host's bottom row does not
automatically place a 200-row session screen at the origin used by subsequent
session cursor-addressing output. The plan handles extra host width but not this
height difference.

**Evidence and confidence:** high for the replacement-restoration ordering gap.
The clamped-height case is a reasoned placement risk. Neither asks for detailed
saved-state or pixel-perfect engine qualification: both arise from the plan's
own re-grant and geometry features.

**Smallest correction:** define the presentation state at every restoration
boundary, including another restore before readiness. End the prior
client-created alternate presentation before seeding normal state, and define
the session screen's coordinate rectangle inside a larger host. Preserve outer
content and FIFO ordering; do not blindly reset an unknown original host screen.
Small presentation-state tracking is not a client emulator.

**Validation:** partially deliver an alternate restoration, resize before
`Ready`, finish the replacement and leave the application's alternate screen.
Verify current normal content and cursor-addressed subsequent output. Also
restore into a host taller than the clamp and verify subsequent application
cursor positioning, not just marker presence.

### D5 - P2: the alternate-grid test requires an invariant the recipe destroys

**Location:** section 3.2 lines 606-621.

**Problem and impact:** cloning and reinstating the alternate grid does not undo
the second `swap_alt()` assignment to the **normal** grid's saved cursor.
The plan nevertheless requires the normal saved cursor to remain unchanged.
A hidden resize can make that cursor differ from the current cursor; the proposed
test can then fail with the proposed implementation.

**Evidence and confidence:** high. The [published Alacritty 0.26.0 source][engine-source]
`alacritty_terminal/src/term/mod.rs:715-725` writes
`grid.saved_cursor = grid.cursor.clone()` on entry. This exact perturbation was
demonstrated in the [restoration experiment][unsafe-osc] and explicitly accepted
as E1 in the [later decision][restore-decisions].

**Smallest correction:** make the plan and its acceptance consistent with the
documented E1 exception, rather than requiring a false unchanged-state assertion.
Retain ordinary normal/alternate restoration checks. Do not reopen upstream
accessor work or intricate saved-state investigation that the
[latest amendment][engine-latest] explicitly declined.

**Validation:** the restoration tests must distinguish the accepted exception
from unintended state corruption and must not falsely report exact preservation.

### D6 - P2: the production pin is not the revision behind the qualification

**Location:** section 3.1 lines 550-553 and section 4 increment 5.

**Problem and impact:** using the harness's locked `alacritty_terminal` 0.26.0
is presented as the obvious production dependency. Most cited actual-engine
qualification instead used unchanged `bbb2cbf...`, identified as 0.26.1-dev.
The published version is not interchangeable with that evidence:
[#46][engine-research] and [#53][embedding-research] specifically distinguish its
unbounded combining-mark storage from the later revision's bound.

This does not require another engine investigation. It requires an honest
dependency choice instead of treating a test-only dependency as a qualified
shipping pin.

**Evidence and confidence:** high. `Cargo.lock:15-18` confirms published 0.26.0.
The inspected registry archive matches that checksum and identifies upstream
commit `94e7c8874e526b1e67b349d9ba30ddf81669119e`, not `bbb2cbf...`.
[#56's provenance findings][provenance] explicitly limit their evidence to their
tested revision and dependencies.

**Smallest correction:** select an exact unchanged upstream source and document
which existing evidence applies. If retaining 0.26.0, explicitly justify the
version change and do not carry over the later combining-bound or provenance
claims. Exercise only the adapter/input/timing seams affected by that choice;
do not restart engine-family selection or exhaustive upstream testing.

**Validation:** the lockfile/source identity and focused seam checks must
describe the actual shipped dependency, not a neighboring development revision.

### D7 - P2: two existing guards are missing from the passing-increment checklist

**Location:** sections 3.1, 4 increments 2 and 5, and 8.

**Problem and impact:** adding `climon2-vt` to `SHIPPED` is necessary but
insufficient. `xtask/tests/architecture.rs:4-17,75-84,105-122` enforces an exact
package set and internal dependency edges. The new crate and daemon dependency
violate both until those guards and ADR 0004 are amended. Deferring the ADR to
the final documentation increment also leaves the declared architecture behind
the introduced structure.

The bash migration misses a second concrete prerequisite:
`.github/workflows/check.yml:75-80` pins the **quality** job to `/bin/bash`.
Once `bash_profiles()` accepts only 5.x, the coverage/mutation workspace tests
will reject that explicit 3.2 path even if the `macos` and `archive` matrices
have been updated.

**Evidence and confidence:** high; both are current executable guards, not
prospective tooling preferences.

**Smallest correction:** in the new-crate increment, update package/dependency
guards and ADR 0004 alongside source and quality registration. In the bash
increment, select controlled bash 5 for the quality job too. Preserve installer
and other intentional non-interactive `/bin/bash` uses. Packaging need not add
a library-only binary.

**Validation:** the new-crate increment passes the architecture tests, and the
quality job exercises bash 5 under the same controlled-shell policy as ordinary
tests. Do not weaken either guard to make the migration green.

### D8 - P2: fresh same-engine roundtrips are insufficient for the serializer

**Location:** section 4 increment 5 and sections 5-6.

**Problem and impact:** replaying restoration into a fresh Alacritty term is a
useful compatibility check, but not an independent correctness oracle. Both
ends can share a wrong terminal assumption. A fresh target also misses precisely
the dirty-host and replacement-restoration cases on which this feature depends.

The retained native checks only promise character presence; they do not make
that same-engine state comparison independent. Existing `wait_for_text`-style
assertions can also find an earlier copy of a marker now that duplicate restored
history is deliberately allowed.

**Evidence and confidence:** high as a test-design gap. [#45's strategy][acceptance]
requires independent small expectations and boundary-specific synchronization.
Its [latest amendment][acceptance-latest] removes exhaustive observers, not those
ordinary product-testing principles. [#56's basic-pen follow-up][pen-probe]
is an example of an independent assertion catching a real restore defect.

**Smallest correction:** retain roundtrips, but add a few literal expected
screen/input/order assertions for the climon2-owned renderer and transport.
Exercise an occupied host and two restores on one attachment. Use numbered
fixture phases and fresh acknowledgements to distinguish retained, restored
and live output. An existing small independent oracle is optional; no second
complete emulator or exhaustive rendition suite is required.

**Validation:** the focused assertions fail if restoration is omitted, uses
the wrong normal/alternate screen, loses a live suffix or enables stale input.
An old marker already present in native history must not satisfy a new phase.

## Opportunities, not blockers

### O1 - P3: deliver one usable continuity slice before the broad renderer

**Location:** section 4's increment sequence.

The current sequence puts the whole VT adapter and substantial daemon work
before a real attach/detach path is exercised. A narrower first vertical slice
with ordinary text, one controller and a small history would expose R1-R3 much
earlier. Expand control transfer, restoration styles and bounded delivery behind
that working path, retaining the same final acceptance and quality gates.

This is a sequencing improvement, not permission to ship an incomplete feature,
merge failing increments or add scaffolding solely for staging.

### O2 - P3: record provenance without making the plan another decision ledger

**Location:** opening paragraph, sections 1.12, 1.15 and 8.

Self-contained execution instructions are useful, but "do not look for
requirements in GitHub issues" hides the distinction between confirmed behavior,
accepted exceptions and newly proposed mechanics. The synchronization exception
and saved-cursor invariant show the cost.

Add a short source/supersession table linking authoritative decisions. Mark
unsettled implementation choices as such. Also explicitly record schema-1
startup and the named capability/state meanings as additively frozen under
[#44][cross-version], not just the older mandatory hello/argv/metadata contracts.
Do not copy the whole issue history into this document or reopen settled choices.

## Unresolved decisions and minimum revision

The necessary decisions are local implementation choices, not reasons for a new
pre-implementation programme: per-invocation client logs; paste cleanup ownership;
ordered terminal release; live-query/output policy; restore-prefix production
and cutover; preparation/readiness deadlines; synchronization integration; and
replacement-restoration presentation state.

Before calling the plan implementation-ready, reconcile those choices with its
existing budgets and focused product tests. Correct the exact engine/evidence
identity, impossible preservation assertion, migration guards and native
artifact/lane reporting. No proposed correction requires a new user setting.

## Review limits and action log

This is a document/source review. No product code was changed, no prototype
branch was merged, no issues were edited or closed, and no user terminal,
clipboard or native automation was used. No runtime, native or full-quality
pass is claimed. Arithmetic examples and source-confirmed behaviors are
distinguished above from proposed tests and reasoned coordination risks.

Actions and reasons:

1. Read repository, domain and issue-tracker guidance, the glossary, relevant
   ADRs, review guidance and the complete plan; establish the applicable contracts.
2. Checked the worktree; preserve the supplied untracked plan.
3. Read map #40, children #41-#56 and inherited #27/#28 discussions; resolve
   supersessions instead of reviving withdrawn requirements.
4. Inspected client logging/transport, daemon PTY/connection/cleanup, discovery,
   environment, architecture tests and CI; verify concrete integration assumptions.
5. Retrieved published Alacritty 0.26.0 and vte 0.15.0 source archives and matched
   their SHA-256 hashes to `Cargo.lock`; distinguish actual source from prototype
   evidence. Discarded an initially mismatched application-tag source and a
   suspected character-size reply gap that exact-source inspection disproved.
6. Ranked substantiated findings and wrote this critique; keep minimal corrections
   separate from optional opportunities and accepted scope limitations.
7. Checked citation identities against the captured discussions, finding counts
   and document whitespace; correct references without changing the supplied plan.

**Counts:** requirements: 5 findings, highest P1 (4 P1, 1 P2).
Design quality: 8 findings, highest P1 (4 P1, 4 P2).
Optional opportunities: 2 P3. These counts do not imply additional engine or
native qualification requirements.

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
[pen-probe]: https://github.com/jackgeek/climon2/issues/56#issuecomment-6551643244
[cross-version]: https://github.com/jackgeek/climon2/issues/44#issuecomment-6547968217
[parser-source]: https://github.com/alacritty/vte/blob/3b3da71c34cc1256c7e20981cf03f8eb95e08ffc/src/ansi.rs
[engine-source]: https://github.com/alacritty/alacritty/blob/94e7c8874e526b1e67b349d9ba30ddf81669119e/alacritty_terminal/src/term/mod.rs
