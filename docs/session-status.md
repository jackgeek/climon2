# Session status, attention, and activity

**Status: agreed specification, not implemented.** The developer confirmed this
revision on 2026-11-06. It supersedes the earlier two-state activity heuristic,
title-plugin requirement, creation-time monitor ordering, and attention rules in
[startup/discovery](https://github.com/jackgeek/climon2/issues/59).
No executable implementation or automated acceptance evidence is claimed here.

The [session daemon](../GLOSSARY.md#sessions) owns both
[activity state and attention state](../GLOSSARY.md#status-and-attention).
The [monitor](../GLOSSARY.md#sessions) displays their effective
[session status](../GLOSSARY.md#status-and-attention); it does not run competing
copies of these machines. Title detection, terminal-signal interpretation, and
these machines belong in daemon core.
[Plugin](../GLOSSARY.md#protocol-and-plugin-concepts) implementation,
enable/disable configuration, and interface examples are deferred. There is no
plugin release prerequisite.

Use the [domain glossary](../GLOSSARY.md), [technical terms](protocol/TERMS.md),
and [protocol reference](protocol/README.md) with this document. The
[daemon contract](https://github.com/jackgeek/climon2/issues/67),
[monitor-client communication](https://github.com/jackgeek/climon2/issues/66),
and [interface schemas](https://github.com/jackgeek/climon2/issues/71) still own
exact messages, fields, compatibility, validation, and examples.

## Events and two FSMs

The two [FSMs](protocol/TERMS.md#fsm) start with activity `idle` and attention
`cleared`. The daemon also owns
the current detection mode: `heuristic` initially, `explicit`, or
`fallback_pending`. This mode selects the activity detector; it is not a
reported session status. Activity and attention evolve independently, connected
by internal [events](protocol/TERMS.md#event).

Both `agent_working` and `user_working` project to the existing `running` status:

```text
activity_status = activity == idle ? idle : running
status = attention == cleared ? activity_status : attention
```

For example, `idle` plus `needs_attention` displays `needs_attention`.
Activity continues to evolve while attention masks it. Switching between
`agent_working` and `user_working` alone does not change effective status.

Dispatch internal events to completion before processing another event. Apply
an emitted `attention_request` to the attention machine before publishing the
resulting effective status; do not report an intermediate `idle` for an
inactivity transition that also requests attention. When accepted input and
timer/sample work are already pending in the same dispatch turn, process input
first. Preserve terminal-signal stream order. Cancel obsolete deadlines and
ignore expiry events from earlier timer arms or detection scopes. This is
daemon-internal dispatch, not a new external event bus or wire schema.

### Activity

```mermaid
stateDiagram-v2
    [*] --> idle
    idle --> agent_working: heuristic viewport_changed
    agent_working --> idle: heuristic inactive_10secs / attention_request
    idle --> user_working: user_input / init_2sec_timer
    agent_working --> user_working: user_input / init_2sec_timer
    user_working --> user_working: user_input / init_2sec_timer
    user_working --> agent_working: timer_expires [explicit root working]
    user_working --> idle: timer_expires [otherwise]
```

This diagram covers heuristic changes and user input. Root
[OSC](protocol/TERMS.md#osc) 7501 reports and shell lifecycle signals can
override any live activity state as specified below.

`init_2sec_timer` is an action, not a persistent activity state. Each qualifying
`user_input` immediately enters/remains in `user_working`, cancels any agent
inactivity deadline, and arms/restarts a hardcoded **2,000 ms** deadline.
Viewport changes during this window neither enter `agent_working` nor restart
the user timer.

At user-timer expiry, restore `agent_working` only if detection is `explicit`
and the current root record in that scope still says `working`; do not require
the application to repeat an unchanged report. Otherwise enter `idle`. This
expiry never requests attention. In heuristic mode, capture a fresh viewport/
title baseline and resume idle change detection. Output confined to the input
window is deliberately ignored; continuing changes after the window can start
another agent-working period. With ordinary sampling, detection can occur
roughly 2,500-3,500 ms after the last input, assuming subsequent sampled changes.
The window is a heuristic, not proof of who produced output.

Explicit root reports supersede `user_working` immediately and cancel its
deadline. Leaving `user_working` cancels that deadline. The fallback grace
period, when armed, is independent of the user timer.

### Attention

```mermaid
stateDiagram-v2
    [*] --> cleared
    cleared --> needs_attention: attention_request
    needs_attention --> acknowledged: session_takeover
    needs_attention --> cleared: user_input
    acknowledged --> needs_attention: attention_request
    acknowledged --> cleared: user_input
    cleared --> failed: session_close_error
    needs_attention --> failed: session_close_error
    acknowledged --> failed: session_close_error
    cleared --> completed: session_close_ok
    needs_attention --> completed: session_close_ok
    acknowledged --> completed: session_close_ok
```

The heuristic `agent_working -> idle` inactivity transition emits
`attention_request`. Root OSC 7501 attention-bearing reports and recognized
notification signals also emit it, without requiring an activity transition.
User-timer expiry, fallback expiry, cleanup, and shell boundaries do not.
Requests are not restricted to `cleared`: an acknowledged session can need
attention again. A repeated request while already `needs_attention` leaves it
unchanged, rather than starting another attention episode.

`session_takeover` here denotes explicit selection/takeover acknowledgement,
including selecting a session whose monitor client already retains
[control](../GLOSSARY.md#attachment-and-control). That case need not request a
new control grant. Merely being visible does **not** acknowledge a new request.
If attention arrives while the user is already viewing the session, it remains
`needs_attention` until actual user input clears it or an explicit selection/
takeover acknowledges it.

`user_input` is actual user-originated application
[bytes](protocol/TERMS.md#byte), including paste, accepted from the current
ready controller. It clears either `needs_attention` or `acknowledged`.
Monitor shortcuts, selector navigation, terminal-query replies, and injected
work do not count. Acceptance is not proof of application consumption.

A confirmed command exit of zero produces `completed`; a confirmed nonzero
exit or signal termination produces `failed`. Those are terminal attention
states: late input, requests, or viewing changes cannot clear them.
Confirmed completion/failure cancels all activity deadlines and stops
observation; late viewport, timer, and terminal-signal events cannot restart
activity or generate attention requests.
[Detach](../GLOSSARY.md#attachment-and-control) is not completion. Connection
loss without a confirmed command result does not invent a final outcome.

## Snapshot sampling and the inactivity deadline

The **agent inactivity algorithm runs only in heuristic mode while activity is
`agent_working`**. Lightweight idle sampling still detects the changes needed
to enter that state. Explicit mode does not infer activity from viewport
changes or expire silent `working` reports. Fallback mode observes quiet for
its separate grace period, not to infer agent work.

Capture a baseline before processing command output. Each sample reads the
currently active viewport as a string of characters plus terminal height and
width. Compare title separately. This samples terminal state, not raw output
bytes or restoration escape sequences.

Choose a fresh random delay from **500 to 1500 ms** for each sampling iteration.
While heuristic `agent_working`, shorten it to the remaining agent deadline:

```text
remaining = max(0, 10000 ms - elapsed)
next_delay = min(random_delay, remaining)
```

Elapsed time uses a [monotonic clock](protocol/TERMS.md#monotonic-clock).
The **10,000 ms threshold is hardcoded**, not a setting in the climon2
configuration file. The user timer and fallback deadline also use this clock;
the user timer is scheduled independently of random sampling.
[UTC](protocol/TERMS.md#utc) time used for status reporting is separate from
elapsed-time measurement.

At each heuristic sample outside `user_working` and prompt-rendering exclusion:

1. If height or width changed, replace the character baseline and restart the
   timer. Do not compare differently sized grids or change activity solely
   because of resize. An independently changed title still counts as activity.
2. With matching dimensions, different characters count as activity. A changed
   title also counts. Either sets activity to `agent_working` and arms/restarts
   the agent inactivity timer. Retain the current characters/title as the next
   comparison baseline, so a change is not counted repeatedly.
3. Check changed content before expiring the timer. If activity remains
   `agent_working` and elapsed time reaches 10,000 ms, set it to `idle`, emit
   `attention_request`, and disarm the inactivity deadline.
4. Continue ordinary random sampling while `idle`. A new meaningful change
   starts another agent-working period and deadline.

Do not alternate `idle` and `agent_working` during uninterrupted silence, or spin on
an expired zero-delay timer. A session silent from startup remains
`idle`/`cleared`; it must first become heuristic `agent_working` before agent
inactivity can request attention.

Styles/colors, cursor-only movement, and identical redraws do not count.
Switching normal/alternate screens with identical sampled characters and
dimensions does not count either. Changes that appear and disappear wholly
between samples are not individually tracked by this strategy.

Sampling must not deliberately overshoot an armed deadline. Execution can
still be late because of scheduler stalls. Schedule work without blocking
daemon connection handling; do not copy the viewport for every output byte.
Exact character-string representation and allocation budgets remain
implementation/interface work, not an additional activity signal.

## Explicit program status and fallback

Use the [Program Status Protocol, revision 0.4](https://www.superlogical.com/rex/docs/build/program-status)
for OSC 7501 framing, validation, record replacement, identifiers, limits,
and capability detection. Interpret it in daemon core before filtering; do
not forward raw sequences to the host. A valid root status report proves
application support; a capability query does not. No prior shell-integration
signal is required. The daemon answers queries locally with its supported
states/kinds. It does not query applications for support or liveness.

Only the root record (no `id`) drives these FSMs and detection mode. Child
reports and child-subtree clearing affect records, not session activity,
attention, or detector selection. Do not aggregate child `working`/`blocked`
into session status. Record validation and lifecycle still follow the
upstream protocol; unsupported/invalid reports do not become `idle`.

Apply root reports from any live activity state:

| Root report | Activity | Detection mode | Attention action |
|---|---|---|---|
| `working` | `agent_working` | `explicit` | None |
| `idle` | `idle` | `explicit` | Emit `attention_request` |
| `done` | `idle` | `explicit` | Emit `attention_request` |
| `blocked` | `idle` | `explicit` | Emit `attention_request` |
| `error` | `idle` | `fallback_pending` | Emit `attention_request` |
| `clear` | `idle` | `fallback_pending` | None |

Each report cancels activity deadlines and replaces the previous root-driven
decision. Every root `idle`/`done`/`blocked`/`error` report requests attention,
even if activity was already `idle` or the report repeats a value. Requests
while `needs_attention` coalesce; a subsequent report after acknowledgement or
input-clearing can start a new episode. A task's `done`/`error` is not a
confirmed exit of the session command and never sets `completed`/`failed`.

Explicit `working` may remain silent indefinitely: there is no heartbeat and
no ten-second agent inactivity deadline. A new root report can override it,
as can accepted input or a lifecycle boundary.

### Ten-second fallback grace period

Root `error` or root `clear` starts/restarts a separate hardcoded **10,000 ms
of viewport inactivity** before returning to heuristics. Root `clear` deletes
all records, including children, but neither requests attention nor clears
existing attention. Absence of records during grace is not immediate fallback.

At grace start, capture a viewport/title baseline and arm its deadline.
Observe using the same random sampling, dimension checks, character/title
comparison, and content-before-expiry ordering as above. Clamp delays to the
remaining grace deadline. Meaningful character/title changes or resize
replace the relevant baseline and restart the grace deadline, without
entering `agent_working` or emitting attention. Do not arm the agent inactivity
deadline during grace.

| Event during grace | Effect on grace |
|---|---|
| Root `working`/`idle`/`done`/`blocked` | Cancel; apply the report and use explicit detection. |
| Root `error`/`clear` | Restart; apply the report's attention rule. |
| Shell boundary `A`/`D`/`C` | Cancel; resume heuristics immediately as described below. |
| Confirmed session command exit | Cancel all observation/timers; apply terminal attention status. |
| Child report or notification signal | No detection-mode change. |
| Accepted user input | No direct grace restart/cancellation; apply the ordinary input rules. |

Grace observation continues during `user_working` solely to measure quiet;
resulting viewport changes can restart grace but cannot infer agent work or
extend the two-second user deadline. Input bytes alone do not restart grace.

On grace expiry, select `heuristic`, capture a fresh baseline, and emit no
attention request. Activity remains `idle`, except that an unexpired
`user_working` window and its deadline must be preserved. Continue ordinary
heuristic detection after that window expires. Disarm the grace deadline
instead of spinning on an expired timer.

A full terminal reset (`RIS`) removes every OSC 7501 record. In explicit or
fallback mode it follows root `clear` activity/grace semantics, with no
attention request. In heuristic mode it refreshes the observation baseline
without manufacturing a status change. A soft reset does not clear records.

### Shell boundaries and prompt rendering

Observe [OSC 133 shell integration](https://learn.microsoft.com/en-us/windows/terminal/tutorials/shell-integration)
locally before filtering:

| Marker | Meaning | Detection action |
|---|---|---|
| `A` | Prompt begins | Retire the prior explicit scope; resume heuristics, excluding prompt rendering. |
| `B` | Prompt ends; command-line input begins | End prompt-rendering exclusion and capture a fresh baseline; do not otherwise reset detection. |
| `C` | Command execution/output begins | Retire the prior explicit scope; resume heuristics and end prompt-rendering exclusion. |
| `D` | Command finishes | Retire the prior explicit scope; resume heuristics. |

Each `A`/`D`/`C` boundary cancels activity deadlines, enters `idle`, and captures
a fresh baseline, without requesting or clearing attention. Repeated boundaries
are harmless. Require a fresh root report to activate explicit detection in the
new scope; stale retained records must not reactivate it on user-timer expiry.
Apply upstream record-lifetime rules: at `A`, discard `working`/`blocked`;
`done`/`error` survive but cannot suppress heuristics for the next command.
An OSC 133 command result is not the session command's confirmed exit.

Between `A` and `B`/`C`, exclude viewport/title changes from heuristic activity
inference. This exclusion does not reject accepted input or explicit reports,
and does not prevent independent fallback-grace observation. If neither
closing marker arrives, exclusion persists; correct shell integration is
required.

Fallback has a deliberate limit. A supporting app can report `working` and
then exit into a still-live shell that emits no OSC 133, without sending
`error`/`clear`. There is no reliable signal to end explicit mode in that case.
Do not invent a heartbeat or time out silent explicit work. A later lifecycle/
cleanup signal or confirmed session command exit resolves it.

## Notification and progress signals

Recognized notifications emit `attention_request` independently of activity,
detection mode, or viewing. They do not cancel activity timers, end explicit
mode, or imply task/session completion.

| Source | Requests attention | Does not request attention |
|---|---|---|
| [OSC 9](https://iterm2.com/documentation-escape-codes.html) | Text notification form | [Progress and other control operations](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC), including OSC 9;4. |
| [OSC 99](https://sw.kovidgoyal.net/kitty/desktop-notifications/) | A complete logical notification | Incomplete chunks, capability/liveness queries, close operations, and auxiliary/control data on their own. |
| [OSC 777](https://sw.kovidgoyal.net/kitty/desktop-notifications/) | The `notify` subcommand | Other extension subcommands. |

Accumulate OSC 99 notification chunks according to its protocol, requesting
attention only when the logical notification is complete, not once per chunk.
Auxiliary data does not independently create an attention event. Parse complete
validated sequences, not number substrings in terminal text. Unsupported
operations are not notifications. No complete desktop-notification protocol
implementation, external action execution, or activation/close acknowledgement
is implied; unsupported notification queries/control operations are ignored.

In addition to attention detection, relay structured notification and progress
metadata through the client to subscribed monitors/clients, including background,
noncontrolling, and revoked subscribers. Progress operations such as OSC 9;4
can support future progress bars/spinners without requesting attention.
This metadata is not live terminal-content mirroring and never grants control.
Do not forward raw OSC sequences into outer terminals for interpretation.
Exact structured schemas, capabilities, ordering/correlation, queue limits,
retention of current progress, and compatibility remain interface work.
Implementing progress UI is not part of this revision.

## Viewing is not merely control

[Viewing](../GLOSSARY.md#status-and-attention) is reported explicitly per
[attachment](../GLOSSARY.md#attachment-and-control). Only a connected controller
whose selected session has reached displayed
[readiness](../GLOSSARY.md#protocol-and-plugin-concepts) can count as viewing.
A background client may retain control without viewing.

Switching away, exiting, disconnecting, or
[revocation](../GLOSSARY.md#attachment-and-control) clears that attachment's
viewing status. Opening the overlaid selector preserves viewing of the selected
session; merely highlighting a different row does not acknowledge it. An
ordinary interactive attach counts as viewing while it controls the session.

Viewing suppresses a monitor's [toast](protocol/TERMS.md#toast) notification;
it does not automatically change the attention [FSM](protocol/TERMS.md#fsm).
Explicit selection/takeover and accepted real input are the acknowledgement/
clearing events.

## Reporting, ordering, and initial selection

On connection, report the current effective daemon status and the UTC timestamp
at which that effective value last changed. Continue reporting changes
regardless of control, menu visibility, or input readiness. Establish current
state and subsequent ordered changes without a subscription gap.

Only effective-status changes update its timestamp. Hidden activity changes
under `needs_attention`, transitions between the two working states, and
detection-mode changes alone do not reorder the row. A new monitor receives the
stored timestamp, not its own connection time. Exact timestamp encoding,
initial timestamp representation, and reliable attention-episode identity
remain schema decisions; a wall-clock timestamp is not an implicit unique ID.

Order session rows by:

| Priority | Status |
|---|---|
| 1 | `needs_attention` |
| 2 | `acknowledged` |
| 3 | `running` |
| 4 | `idle` |
| 5 | `failed` |
| 6 | `completed` |
| 7 | `disconnected` |

Within a group, sort newest status-change UTC timestamp first, then ascending
full session [GUID](protocol/TERMS.md#guid) for deterministic ties. GUIDs are
internal identity only; never display them in the menu. This replaces
creation-time row ordering.

Collect status passively with bounded progress before initial selection, then
select the first row in this order. Unknown/unavailable status is not
`cleared`; report incomplete discovery explicitly rather than fabricate a clean
result. Compatibility and placement of unsupported/unknown rows still require
the discovery/interface decisions.

Initial choice remains pinned while it starts. Definitive loss before readiness
permits reranking remaining eligible candidates; other startup failures remain
visible rather than silently skipping the candidate. At most one attempt per
candidate per discovery pass remains the rule. Subsequent reports re-sort rows
without changing selected/highlighted identity or redirecting confirmation.

Discovery still finds live sessions; final views retained within one monitor
do not create a persisted startup archive. Eager all-session client creation
and its existing bounded newest-first scheduling remain distinct from the
revised status-based row ordering and initial selection.

## Connection loss and retained terminal views

[Connection status](../GLOSSARY.md#status-and-attention) is monitor-owned,
separate from the two daemon FSMs. Keep one `climon2-vt` model per monitor-owned
session, not per short-lived socket. Retain received terminal content and
bounded local scrollback after disconnection for read-only inspection. Mark it
disconnected/stale; never forward application input while disconnected.

Preserve a definitively received `completed` or `failed` status after the
connection closes. Otherwise use the `disconnected` ordering group and the
monitor's recorded UTC disconnection time; retain the last daemon status/time
as stale detail rather than overwriting them.

Periodically retry uncertain disconnections with bounded scheduling/backoff.
Reconnect passively and refresh status/title without automatically taking
control, even if the session remains selected. Explicit user action resumes
control; apply a fresh valid restoration before presenting content as current
or enabling input. Stop retrying definitively completed sessions.

This does not permit live read-only mirroring after revocation, recover output
that was never received, or record every alternate-screen update. Per-session/
aggregate retention limits and exhaustion behavior belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
Reconnect does not change the existing
[leave policy](../GLOSSARY.md#attachment-and-control): unexpected loss of the
final attachment can end the session. No new survival guarantee is implied.

## Titles and notifications

Daemon core supplies its current terminal title and ongoing changes without
requiring control or scraping a monitor's screen. Use the latest non-empty
title in the menu; otherwise use command plus arguments. Unsupported title
delivery on older daemons does not block takeover. Render titles literally and
mark cached data stale when delivery becomes uncertain.

Users do not set terminal titles through climon2. Future user-assigned,
persisted session names are separate and will appear alongside a non-empty
terminal title. Naming remains deferred, including revisiting the
[write-once metadata rule](adr/0005-frozen-session-metadata-core.md).

Show one [toast](protocol/TERMS.md#toast) for an unviewed attention episode or a
confirmed unviewed completion/failure. Acknowledgement and activity-only changes
do not notify. Repeated state reports and reconnects do not duplicate an
episode's notification. `acknowledged -> needs_attention` is a new episode; a
new monitor may notify once about an existing pending episode. Cached
disconnected state alone cannot manufacture a new episode.

## Remaining work and required evidence

The owning tickets must complete exact schemas, compatibility/rejection,
terminal-signal validation and budgets, structured notification/progress relay,
viewing validation, notification identity and rate limits, reconnect intervals/
backoff, retention budgets, terminal integration, and orderly shutdown. These
mechanics do not settle every question in those tickets.

[Automated acceptance](https://github.com/jackgeek/climon2/issues/62) must cover
the following, with no claim that this documentation supplies that evidence:

- Activity/attention transitions, two-second re-arming, stale expiry rejection,
  both input-clearing paths, and no visibility auto-acknowledgement.
- Snapshot/title/resize cases, random scheduling, content-before-expiry and
  deadline clamping, initial silence, and no activity inference during the
  user window or prompt rendering.
- Both working states projecting to `running`, effective-only timestamps,
  unchanged ordering, and atomic attention-request dispatch without transient
  idle reports.
- Root-only OSC 7501 effects, repeated attention-bearing reports, validation
  and record limits, capability queries not activating explicit detection,
  indefinite silent explicit work, and restoration of explicit working after
  user-timer expiry.
- Error/clear grace entry, restart and cancellation, reset behavior, resize/
  title/character quiet measurement, independent user deadlines, and expiry
  without another attention request.
- OSC 133 boundaries, prompt exclusion and baseline refresh, retained
  done/error not pinning detection, no invented session exit, and the documented
  missing-lifecycle limitation.
- OSC 9 progress/control exclusions, OSC 99 complete-notification chunking,
  OSC 777 subcommand filtering, and structured metadata delivery without control
  or raw host passthrough.
- Passive startup/reconnect, retained disconnected inspection, terminal-state
  immunity to late events, and duplicate-free episode notifications.

Actual monitor-rendering evidence remains required; isolated helpers are not
sufficient. No plugin implementation or progress-UI evidence is required by
this revision.
