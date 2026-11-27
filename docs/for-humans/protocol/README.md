# climon2 protocol reference

This reference is for future [plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts)
authors. It distinguishes implemented behavior, agreed plans, and unresolved
interfaces. The [monitor](../../../GLOSSARY.md#sessions) implements its side of
these interfaces ([#106](https://github.com/jackgeek/climon2/issues/106) to
[#120](https://github.com/jackgeek/climon2/issues/120)), except plugin
extensions and library snapshots.
Plugin implementation is deferred. Title and terminal-signal detection and
[session status](../../../GLOSSARY.md#status-and-attention) belong in
[session daemon](../../../GLOSSARY.md#sessions) core;
the session daemon implements the agreed
[mechanics and state diagrams](../behaviour/session-status.md) ([#91](https://github.com/jackgeek/climon2/issues/91), [#92](https://github.com/jackgeek/climon2/issues/92),
[#98](https://github.com/jackgeek/climon2/issues/98)).

**Audience:** Integrators and Contributors.

**Reference:** [technical terms](TERMS.md)

## Shared terms

Use the [domain glossary](../../../GLOSSARY.md) for climon2 concepts and the
[technical terms](TERMS.md) for protocol vocabulary. Each operation page links
to its owning specification. The [native header](monitor-client/client-v1.h)
assigns complete revision-1 declarations; the [native contract](monitor-client/README.md)
and daemon wire pages assign exact agreed schemas, separately from implementation.

## Contents

| Document or folder | Purpose |
|---|---|
| [TERMS.md](TERMS.md) | Technical vocabulary used across the protocol pages. |
| [library-embedding.md](library-embedding.md) | How a program loads the client library and owns its instances. |
| [terminal-integration.md](terminal-integration.md) | How the monitor drives its host terminal: input, focus, restoration and cleanup. |
| [daemon-client/](daemon-client/README.md) | The frozen hello and wire revision 1 between a client and a session daemon. |
| [monitor-client/](monitor-client/README.md) | The native revision-1 calls and records between the monitor and a client library. |
| [monitor-host/](monitor-host/README.md) | The status reports and bell the monitor writes to its host. |
| [monitor-link/](monitor-link/README.md) | The agreed, unimplemented link revision 1 between a hub monitor and a relay. |

## Status and scope

| Interface | Status | Reference |
|---|---|---|
| [Client](../../../GLOSSARY.md#sessions) to session daemon: [hello handshake](../../../GLOSSARY.md#sessions) | Implemented and frozen | [Request](daemon-client/hello-request.md) and [reply](daemon-client/hello-reply.md). |
| Client to session daemon: after hello | Wire revision 1 only, agreed and partly implemented; the version-private protocol it replaced is retired ([#104](https://github.com/jackgeek/climon2/issues/104)). Session daemons serve passive monitor connections, interactive attachments and standalone Kill, with the command's final result, heuristic `status-v1` status and acknowledgement, `notification-v1` notifications and `progress-v1` progress ([#98](https://github.com/jackgeek/climon2/issues/98)), and monitors that acquire and inherit control with [canonical presentation](../../../GLOSSARY.md#restoration-and-presentation) | [Post-hello boundary](daemon-client/post-hello.md), [complete wire rules](daemon-client/README.md). |
| Monitor to [client library](../../../GLOSSARY.md#sessions) | Complete native revision-1 specification agreed. The library implements its [compatibility check](monitor-client/request-hello.md) bootstrap and description ([#75](https://github.com/jackgeek/climon2/issues/75)) and the instance lifecycle of its [function table](TERMS.md#function-table) ([#85](https://github.com/jackgeek/climon2/issues/85)), opens [passive monitor connections](../../../GLOSSARY.md#attachment-and-control) through monitor-mode Open, Leave and their records ([#93](https://github.com/jackgeek/climon2/issues/93)), takes control and relays display through interactive Open, Acquire, Input, Resize, Ready, Focus and the control, restoration, Display and Geometry records ([#94](https://github.com/jackgeek/climon2/issues/94)), relays status with Set viewing and Acknowledge ([#95](https://github.com/jackgeek/climon2/issues/95)), and relays Command final and Kill on its own auxiliary connection ([#96](https://github.com/jackgeek/climon2/issues/96)), and relays notifications and progress ([#98](https://github.com/jackgeek/climon2/issues/98)). The shared loader that finds, opens and validates libraries is implemented ([#84](https://github.com/jackgeek/climon2/issues/84)). The monitor loads libraries through it and opens one passive client per live session ([#106](https://github.com/jackgeek/climon2/issues/106)), and acquires the session selected with Enter, applying its restorations and submitting Ready and Resize ([#108](https://github.com/jackgeek/climon2/issues/108)), Input ([#109](https://github.com/jackgeek/climon2/issues/109), [#111](https://github.com/jackgeek/climon2/issues/111)) and Focus ([#111](https://github.com/jackgeek/climon2/issues/111)); it reports viewing and acknowledges attention ([#114](https://github.com/jackgeek/climon2/issues/114)), retries lost clients passively ([#116](https://github.com/jackgeek/climon2/issues/116)), sends Kill for Delete ([#117](https://github.com/jackgeek/climon2/issues/117)), shuts every instance down with Leave ([#118](https://github.com/jackgeek/climon2/issues/118)) and reserves each load and client before admitting it ([#120](https://github.com/jackgeek/climon2/issues/120)). | [Library embedding](library-embedding.md), [native contract/header/examples](monitor-client/README.md), and [shared communication contract](#shared-in-process-communication-contract). |
| [Client executable](../../../GLOSSARY.md#sessions) to same-release library | Same-release loading implemented: the client executable loads its own version folder's library through the shared loader and drives ordinary attach (interactive Open, Input, Resize, Acquire, Ready and Leave, requiring only `daemon-wire-v1`; [#101](https://github.com/jackgeek/climon2/issues/101)), with Set viewing and one Acknowledge after an explicit attach or takeover when `status-v1` is selected ([#102](https://github.com/jackgeek/climon2/issues/102)), and kill ([#100](https://github.com/jackgeek/climon2/issues/100)) through it, with a fixed `cm1_limits` table. On Windows it drives only kill so far ([#318](https://github.com/jackgeek/climon2/issues/318)) | [Library embedding](library-embedding.md). |
| Monitor to [host](../../../GLOSSARY.md#sessions) | Program-status advertising ([#114](https://github.com/jackgeek/climon2/issues/114)) and the toast bell ([#115](https://github.com/jackgeek/climon2/issues/115)) implemented | [Reports and bell](#monitor-to-host-status-and-bell). |

Current local runtime communication is implemented on macOS and x86_64
GNU/Linux. The frozen [session metadata](../../../GLOSSARY.md#sessions)
[contract](../../adr/0005-frozen-session-metadata-core.md) includes an optional
clock-independent Linux daemon identity; the eleven frozen fields and hello
messages are unchanged. Native monitor-client communication is implemented
on both systems: each release ships its client library with its bootstrap,
description, local instance lifecycle and every revision-1 family
([#75](https://github.com/jackgeek/climon2/issues/75), [#85](https://github.com/jackgeek/climon2/issues/85), [#93](https://github.com/jackgeek/climon2/issues/93) to [#96](https://github.com/jackgeek/climon2/issues/96), [#98](https://github.com/jackgeek/climon2/issues/98)), and the monitor loads it
through the shared loader ([#84](https://github.com/jackgeek/climon2/issues/84), [#106](https://github.com/jackgeek/climon2/issues/106)). The [Linux decision](https://github.com/jackgeek/climon2/issues/125)
agreed Linux in-process loading in the
[embedding design](library-embedding.md#artifact-and-dependency-lookup): the
monitor and its client library ship on both systems in the same release.
On x86-64 Windows, session daemons host headless sessions
([#317](https://github.com/jackgeek/climon2/issues/317)) and the client
executable drives its `climon2_client.dll` for `c2 kill`
([#318](https://github.com/jackgeek/climon2/issues/318)). Ordinary attach
([#321](https://github.com/jackgeek/climon2/issues/321)) and the monitor
([#327](https://github.com/jackgeek/climon2/issues/327)) do not run there
yet, and no release publishes a Windows build.

The embedding design selects a versioned [C ABI](TERMS.md#c-abi).
Complete [declarations](monitor-client/client-v1.h) and
[compilable examples](monitor-client/examples.c) are published as a specification.
Communication uses tagged C-compatible records, not [JSON](TERMS.md#json),
a pipe protocol, or serialized [envelopes](TERMS.md#envelope).
Native [schemas](TERMS.md#schema) have fixed tags, fields, ownership and lifetimes.
There is no in-process [framing](TERMS.md#frame) or frame limit.
Revision layouts/semantics are frozen independently of release identity.
The existing daemon hello exchange is unaffected.

The agreed [monitor acceptance matrix](../behaviour/monitor/acceptance.md) assigns
source-linked scenarios and required contract, packaged-library, real-monitor,
fault and measured evidence. Release qualification ([#122](https://github.com/jackgeek/climon2/issues/122)) runs it: the
release workflow runs `cargo xtask qualify` on each candidate's artifacts and
publishes only with a passing verdict. No release has been qualified yet.

The [operational-limits specification](../behaviour/monitor/operational-limits.md) supplies
agreed configurable queue/transfer/payload budgets, admission, deadlines, retry
and required measurements. The native contract maps library-applicable limits
to explicit startup fields, without adding framing.
Resource-blocked sessions stay visible; pressure shortens captured history to a
protected recent page, never silently evicts clients or erases that page.
The client library and the monitor implement these requirements as far as the
[operational-limits status](../behaviour/monitor/operational-limits.md) says
([#119](https://github.com/jackgeek/climon2/issues/119),
[#120](https://github.com/jackgeek/climon2/issues/120)).

## Who communicates?

```text
MONITOR PROCESS
  monitor
    <-> version-matched client library
        one client thread per live session
        one loaded library shared by sessions on the same release
          <-> session daemon in its own process
              <-> command through its PTY
```

The [ownership decision](https://github.com/jackgeek/climon2/issues/58)
requires the monitor to load session-version libraries directly. It starts
clients eagerly, newest first, with bounded concurrent progress. Continuous
discovery adds clients for newly discovered sessions without changing selection.
The command remains behind the daemon's [PTY](TERMS.md#pseudo-terminal-pty).
The daemon remains the authority for terminal state and terminal-query replies.

The [agreed daemon wire](daemon-client/README.md) is portable, independent of
today's local private sockets, but it never crosses a machine boundary. A
[hub monitor](../../../GLOSSARY.md#monitor-links) reaches another machine's
sessions through a [relay](../../../GLOSSARY.md#monitor-links) over
[TCP/IP](TERMS.md#tcpip), using the agreed and unimplemented
[monitor link protocol](monitor-link/README.md); the relay is an ordinary
client of its own machine's daemons. Future [WebSocket](TERMS.md#websocket)
forwarding feeds xterm.js in a browser, not a native library in that
browser, and its routing and security remain unresolved. The
[artifact diagram](../architecture/runtime-artifacts.md) shows these planned consumers.
There is no initial application checksum; only a future negotiated integrity
revision could add one if chosen.

`c2 exec --headless <command> [args...]` creates a
[headless session](../../../GLOSSARY.md#sessions) without an interactive client. It
prints the [session ID](../../../GLOSSARY.md#protocol-and-plugin-concepts) and
succeeds only after daemon-confirmed creation. The
session's zero-client survival lasts until command completion or explicit
[Kill](../../../GLOSSARY.md#attachment-and-control), including after later
[attachment](../../../GLOSSARY.md#attachment-and-control) loss. This CLI spelling
and lifetime are implemented as agreed in the
[lifecycle](../behaviour/monitor/lifecycle.md#headless-creation-and-existing-cli-kill).
The CLI passes the headless [lifetime policy](../../../GLOSSARY.md#sessions) to its
same-release daemon as a private startup argument; no wire message or field is
added. Its [hello reply](daemon-client/hello-reply.md) reports `detached`
whenever it has no attachment. Existing `c2 kill <session>` ends headless sessions; no
duplicate command is added. [Ordinary sessions](../../../GLOSSARY.md#sessions) retain
[leave policy](../../../GLOSSARY.md#attachment-and-control), and this exception does
not make all sessions persistent. Succession to
[passive monitor connections](../../../GLOSSARY.md#attachment-and-control) also
applies to headless sessions under
[wire revision 1](daemon-client/README.md)
([#89](https://github.com/jackgeek/climon2/issues/89)).

Ratatui powers the monitor's rendering. The agreed
[terminal integration contract](terminal-integration.md) selects exact ordered
terminal bytes through client-library records, parsed into the monitor's own
per-[session](../../../GLOSSARY.md#sessions) `climon2-vt`, with read-only Rust views
for painting, which `climon2-vt` provides and the monitor paints from
([#108](https://github.com/jackgeek/climon2/issues/108)).
Session bytes never go directly to the host. Monitor-mode
daemon presentation is canonically serialized at effective session
[geometry](TERMS.md#geometry),
not outer-host dimensions; ordinary attach is unchanged. Session daemons
implement this [canonical presentation](../../../GLOSSARY.md#restoration-and-presentation)
([#89](https://github.com/jackgeek/climon2/issues/89)), following every
effective geometry change with a restoration at that geometry.
[Restoration](../../../GLOSSARY.md#restoration-and-presentation) replaces
the whole model/history transactionally. Applied
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) permits wire progress;
selected input additionally waits for a coherent current frame to be painted.
The monitor implements its side of these #63 amendments
([#108](https://github.com/jackgeek/climon2/issues/108),
[#109](https://github.com/jackgeek/climon2/issues/109)).

Each client has its own connection and per-session state. The
[embedding reference](library-embedding.md) specifies exact-release lookup,
revision negotiation, thread ownership, memory rules, and library lifetime.
The [shared communication contract](#shared-in-process-communication-contract)
selects bounded per-instance queues and explicit ownership, without callbacks
into monitor application code. [Exact declarations](monitor-client/client-v1.h)
and [lifetime calls](monitor-client/README.md#function-calls-and-ownership) are agreed.
No client may repurpose process-global arguments, signals, or
[standard streams](TERMS.md#standard-streams) as though it owned the monitor.

Native client crashes or aborts can terminate the entire monitor and all its
clients. Ordinary errors and recoverable panics should remain local where
possible. There is no new session-survival guarantee:
the current [leave policy](../../../GLOSSARY.md#attachment-and-control) can terminate
a session that unexpectedly loses its final
[attachment](../../../GLOSSARY.md#attachment-and-control). The agreed wire
[succession rules](daemon-client/README.md#membership-and-succession) try
remaining attachments, then eligible passive registrations, before
final-participant policy; they do not guarantee survival of losing every client.

## Monitor-to-host status and bell

The [selector specification](../behaviour/monitor/selector.md) adds monitor-generated
[OSC](TERMS.md#osc) 7501 reports and a separate [BEL](TERMS.md#bel).
Session sequences are never forwarded for this purpose.
Reports use the upstream
[Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status):
[ASCII](TERMS.md#ascii) key/value pairs after `ESC ] 7501;`, colon-separated, terminated by
`ESC \`. Omit `id` to address its
[root record](https://www.superlogical.com/rex/docs/build/program-status#records-and-ids).
Every replacement repeats `app=climon2-monitor`; no session IDs, titles,
messages, progress or child records are exported.

Send initial aggregate state and changes only, without heartbeat or capability
query. Any connected `needs_attention` session means pending attention,
independent of toast visibility. Acknowledged, disconnected and final reports
do not qualify. These are not per-session messages or new daemon-wire operations.
The monitor sends the three reports ([#114](https://github.com/jackgeek/climon2/issues/114)): the initial state after its
first frame, each change after the frame that shows it, and the root
`clear` after it releases the host on exit. It rings the bell
([#115](https://github.com/jackgeek/climon2/issues/115)) once for each toast,
after the frame that first shows its full title has been written, unless
`monitor.bell` is false.

| Monitor -> host operation | When | Reference |
|---|---|---|
| Root `blocked`, `kind=question` | Connected pending attention exists | [Attention report](monitor-host/report-attention.md). |
| Root `working` | Monitoring continues without pending attention | [Monitoring report](monitor-host/report-monitoring.md). |
| Root `clear` | Orderly exit cleanup; attempted before a fail-stop | [Clear report](monitor-host/report-clear.md). |
| Bell | Each toast's full title is first written, if configured on | [Bell](monitor-host/bell.md). |

Upstream framing/field/record limits and unknown-key behavior apply.
These fixed reports fit the existing supported states; no new negotiation,
acknowledgement, [request IDs](TERMS.md#request-id),
[control periods](../../../GLOSSARY.md#attachment-and-control) or retry/replay policy is added.
Unsupported hosts may ignore them. Writing does not prove host interpretation,
sound or user acknowledgement. The
[agreed lifecycle](../behaviour/monitor/lifecycle.md#coordinated-shutdown) requires
restoration and root clear before final diagnostics; broken output makes these
best-effort with explicit failure, not a successful cleanup promise.
These signals do not amend daemon activity/attention [FSMs](TERMS.md#fsm) or acknowledge sessions.

## Startup and control

The [key introduction](../behaviour/monitor/selector.md#key-introduction-and-help) opens
automatically when key hints are enabled and its separate saved suppression
preference is unset. It introduces configured-prefix+x, +s, +j and +?.
Prefix+? always opens it, including with hints disabled or startup suppression
saved. The modal gates application input without blocking background startup
or changing control/selection.

1. Discover live sessions in deterministic newest-first order.
2. Load each required session-version library and validate identity and interface
   support before instance creation or a daemon connection.
3. Start one client per live session, with bounded concurrency. A slow or broken
   session must not indefinitely prevent older sessions from getting clients.
4. Establish passive background connections that receive core title/status and
   structured notification/progress data
   without taking [control](../../../GLOSSARY.md#attachment-and-control).
   [Open monitor mode](daemon-client/request-open.md) registers a
   [passive monitor connection](../../../GLOSSARY.md#attachment-and-control),
   always eligible as a successor but not counted as an attachment until
   explicit acquisition or [promotion](../../../GLOSSARY.md#attachment-and-control). Registration alone never takes over a
   detached session; there is no observation-only option.
5. Collect current daemon status and its change time passively, with bounded
   progress. Choose the first candidate in the
   [status/time ordering](../behaviour/session-status.md#reporting-ordering-and-initial-selection),
   not by creation time. Unknown status remains explicit uncertainty.
6. Acquire control for initial selection or explicit user selection/takeover.
   Enable application input only after the current complete
   [restoration](../../../GLOSSARY.md#restoration-and-presentation) has been applied.

The [startup/discovery decision](https://github.com/jackgeek/climon2/issues/59)
is implemented by the monitor ([#106](https://github.com/jackgeek/climon2/issues/106), [#266](https://github.com/jackgeek/climon2/issues/266)). Run an immediate discovery pass, then start the next
pass one second after the preceding pass completes, without overlap. Preserve
the two-second hello deadline and bounded [probes](../../../GLOSSARY.md#sessions).
Reconcile by full [session ID](../../../GLOSSARY.md#protocol-and-plugin-concepts),
reusing starting and retained clients; discovery uncertainty never overrides an
established client's state or tears it down.

Pin the chosen initial candidate while it starts. New arrivals
receive passive clients without displacing it. The later
[lifecycle amendment](../behaviour/monitor/lifecycle.md#monitor-and-selection-lifetime)
preserves chosen identity through loss, completion and failed acquisition,
without automatically acquiring a replacement. Startup failures stay visible
on that candidate without silent fallback. Startup
selection succeeds only after control and the current complete restoration
have been applied. [Revocation](../../../GLOSSARY.md#attachment-and-control) before
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) leaves a non-controlling
[placeholder](../../../GLOSSARY.md#attachment-and-control) requiring explicit user action, not repeated automatic takeover.
The monitor remains open on empty startup, all-unavailable or all-final states
and continues discovery. Explicit selection is never redirected because its
target fails.

## Session status and monitor recovery

The daemon owns independent
[activity/attention states](../../../GLOSSARY.md#status-and-attention) and reports
their effective status plus its [UTC](TERMS.md#utc) change timestamp on connection
and on changes.
The monitor orders rows by status, then newest change time, with an internal
[GUID](TERMS.md#guid) tie-breaker. Later sorting must preserve selected/highlighted identity.
See the [canonical mechanics](../behaviour/session-status.md) for both diagrams, the
snapshot strategy, exact ranking, and notification behavior. Both internal
working states report the existing `running` status; detector changes alone do
not update its timestamp.

### Terminal signals and metadata

The agreed revision interprets [OSC](TERMS.md#osc) 7501 program status,
OSC 133 shell lifecycle, and OSC 9/99/777 notification/progress signals in daemon
core before host-output filtering. The session daemon implements the OSC 7501
and OSC 133 rows below, including the local capability reply
([#92](https://github.com/jackgeek/climon2/issues/92)), and the OSC 9/99/777
rows ([#98](https://github.com/jackgeek/climon2/issues/98)); none of this is a
plugin contract. External sequence framing
and validation belong to their upstream references:

| Program/daemon interaction | Direction | Reference and local policy |
|---|---|---|
| OSC 7501 root/child reports and clearing | Program -> daemon | [Program Status Protocol](https://www.superlogical.com/rex/docs/build/program-status); [root-only effects and fallback](../behaviour/session-status.md#explicit-program-status-and-fallback). |
| OSC 7501 [capability](../../../GLOSSARY.md#sessions) query/reply | Program -> daemon -> program | Same upstream reference; daemon replies locally, and a query does not activate explicit detection. The reply is `ESC ] 7501 ; ? : states=idle,working,done,blocked,error : kinds=permission,question,auth ESC \` (without spaces), in order with the daemon's other terminal replies. |
| OSC 133 prompt/command markers | Shell -> daemon | [Shell integration](https://learn.microsoft.com/en-us/windows/terminal/tutorials/shell-integration); [scope retirement and prompt exclusion](../behaviour/session-status.md#shell-boundaries-and-prompt-rendering). |
| OSC 9 text notification | Program -> daemon | [iTerm2 sequences](https://iterm2.com/documentation-escape-codes.html); [notification policy](../behaviour/session-status.md#notification-and-progress-signals). |
| OSC 9 progress/control operations | Program -> daemon | [ConEmu sequences](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC); progress is metadata, not attention. |
| OSC 99 logical notification and auxiliary operations | Program -> daemon | [kitty notifications](https://sw.kovidgoyal.net/kitty/desktop-notifications/); only completed notifications request attention. |
| OSC 777 `notify` | Program -> daemon | [foot syntax](https://codeberg.org/dnkl/foot/src/branch/master/doc/foot-ctlseqs.7.scd) and [parser](https://codeberg.org/dnkl/foot/src/branch/master/osc.c); [climon2 normalization](daemon-client/README.md#source-normalization), not every upstream quirk. |

These are not new climon2 wire encodings. The daemon remains responsible for
terminal-query replies; raw sequences do not pass through to
[host](../../../GLOSSARY.md#sessions) terminals.
Notification detection does not promise a full desktop-notification protocol
implementation; unsupported notification queries/control operations are ignored.
The [notification](monitor-client/event-notification.md) and
[progress](monitor-client/event-progress.md) interactions relay structured
metadata through clients to subscribers, including background/noncontrolling
ones. They are distinct from terminal display updates and do not authorize
screen mirroring. Session daemons send their
[wire records](daemon-client/README.md) and the client library relays them as
the native records of [revision 1](monitor-client/README.md)
([#98](https://github.com/jackgeek/climon2/issues/98)). [UI](TERMS.md#ui) terminal behavior is now
specified in the [terminal contract](terminal-integration.md); selector layout
is in the [selector specification](../behaviour/monitor/selector.md), and lifecycle policy
in the [lifecycle specification](../behaviour/monitor/lifecycle.md). The monitor implements
them ([#106](https://github.com/jackgeek/climon2/issues/106) to [#118](https://github.com/jackgeek/climon2/issues/118)).

[Viewing](../../../GLOSSARY.md#status-and-attention) is reported per
[attachment](../../../GLOSSARY.md#attachment-and-control);
retained control alone is not viewing. Merely being visible does not
acknowledge attention. The [viewing operation](monitor-client/request-viewing.md), which the monitor uses ([#114](https://github.com/jackgeek/climon2/issues/114)),
and [status reports](monitor-client/event-session-status.md) map the agreed wire
through the [native declarations](monitor-client/client-v1.h). Menus clear this monitor's viewing
without releasing control/restoration. Only this monitor's own unobstructed
presentation suppresses its toasts; another viewer does not.
The [terminal rules](terminal-integration.md) also gate viewing on a live
viewport, usable painted canvas and reportable host focus. Nonmodal coverage
clears viewing but retains ready keyboard input; host focus-in/toast disappearance
never acknowledges. Application focus uses optional non-user core Focus.

The monitor retains a per-session `climon2-vt` model and bounded local scrollback
for disconnected read-only inspection. Overlay exactly `Reconnecting...` while
automatic passive recovery is scheduled or active. Permanent rejection instead
shows an explicit disconnected/error overlay. Connected revoked cache has no
inspection route; reconnect removes the overlay but leaves a non-controller
placeholder until explicit acquisition. No prefix+h/prefix+r binding is added.
Automatic exponential backoff covers uncertain transport loss, recoverable
startup failure (including missing libraries), and safe worker panics.
Permanent protocol/interface rejection, invalid configuration and definitive
completion stop retry. Each fresh attempt waits for prior instance cleanup/
join; never overlap replacement instances or replay indeterminate work.
Reconnect never automatically takes control; cached output/status is not proof
of a live or completed session.
Detailed [connection/retention rules](../behaviour/session-status.md#connection-loss-and-retained-terminal-views)
preserve the existing leave policy and native-crash failure boundary.
The [selector Delete amendment](../behaviour/monitor/selector.md#delete-terminate-and-remove)
reopens the map's prior killing/removal exclusion. A default-Cancel modal pins
immutable GUID identity. Confirmed removal waits for definitive termination and
local instance cleanup/join, then dismisses that GUID/cache for this run only.
Lost Kill responses keep the row and permit only passive outcome recovery, never
Kill replay. Discovery cannot re-add dismissed GUIDs in the same run. Selection/
highlight fallback after removal does not acquire control.

## Session menu titles

The menu shows the daemon's latest non-empty terminal title, falling back to
command plus arguments, never session IDs. IDs remain internal identity:
duplicate or changing titles cannot retarget selection, highlight, or
confirmation. Render titles and metadata literally, never as terminal commands.
Command, working directory, creation time, depth, and availability provide
distinguishing details; exact layout belongs to the selector decision.

The [title state/change interaction](monitor-client/event-title.md)
uses daemon core, not a plugin or plugin extension. When supported, it provides
the authoritative current title and gap-free ordered updates regardless of
control, menu visibility, or input readiness, without screen mirroring.
Session daemons already send it over wire revision 1 as
[Initial metadata](daemon-client/event-initial-metadata.md) and
[Title changed](daemon-client/event-title-changed.md); the client library
relays it ([#93](https://github.com/jackgeek/climon2/issues/93)) and the monitor labels its rows with it ([#106](https://github.com/jackgeek/climon2/issues/106)).
Cached titles must be marked stale/unavailable when delivery becomes uncertain.
An empty title restores the command/arguments fallback.
The permanent top row uses title/fallback as a label only; overflowing label
text is truncated statically, ending with `…` at a
[display-cell](TERMS.md#display-cell)-safe width, and nothing marquees;
status/control/error stay stationary and full details remain in the selector. No host-title change is made.

Plugin implementation, configuration, and interface examples are deferred;
the earlier title-plugin release requirement is superseded. Older daemons
without title support remain usable by the monitor with command/arguments
fallback; lack of title support must not block takeover.

Users do not set terminal titles through climon2. User-assigned persisted session
names and renaming remain future work; a name will appear alongside a non-empty
terminal title, not replace or change it. Persisting renames would
require revisiting the [write-once metadata rule](../../adr/0005-frozen-session-metadata-core.md).

## Retained control

Switching away retains a client and its existing control until
[revocation](../../../GLOSSARY.md#attachment-and-control). Background clients receive
neither application input nor monitor resize requests. A revoked client shows a
[placeholder](../../../GLOSSARY.md#attachment-and-control), not live screen mirroring,
and must not automatically take control back. Core title/status data continues
to flow regardless of control, as does structured notification/progress
metadata ([#98](https://github.com/jackgeek/climon2/issues/98)); future plugin implementation remains deferred.

## Shared in-process communication contract

**Status:** [communication design agreed in #66](https://github.com/jackgeek/climon2/issues/66).
The client library implements its delivery ownership, notification and release
rules ([#85](https://github.com/jackgeek/climon2/issues/85)), and request
admission, correlation and the Response, Initial metadata, Title and
diagnostic records of passive monitor connections
([#93](https://github.com/jackgeek/climon2/issues/93)), and the control, status, Kill, notification and progress requests
and records ([#94](https://github.com/jackgeek/climon2/issues/94) to [#96](https://github.com/jackgeek/climon2/issues/96), [#98](https://github.com/jackgeek/climon2/issues/98)); the monitor uses them
([#106](https://github.com/jackgeek/climon2/issues/106) to [#120](https://github.com/jackgeek/climon2/issues/120)). Extension requests and records are not implemented.
This section owns the
common rules for every monitor-client
operation and delivery. The [embedding contract](library-embedding.md) remains
authoritative for loading and lifetime. [Native revision 1](monitor-client/README.md)
publishes complete declarations without claiming an implementation.

### Execution and representation

Each instance has bounded submission and delivery queues. A per-instance
thread-safe [communication handle](TERMS.md#communication-handle) lets the
monitor submit work and drain deliveries without blocking. The dedicated
monitor-created client thread executes connection work and operations; it also
creates and cleans up the instance. There are no calls into monitor application
code and no callback reentrancy. No lock-free implementation is required.

Submit, drain, and delivered-object release perform bounded local work: no
daemon [I/O](TERMS.md#io), terminal processing, waiting for queue space, or joining threads.
Copying accepted data may scale with its bounded payload; nonblocking does not
mean zero work. There is one monitor consumer per instance. The separate
thread-safe shutdown control handle remains serviceable independently of full
ordinary queues. Shutdown can interrupt ordinary execution order, but cannot
fabricate successful outcomes.

Records use a [discriminator](TERMS.md#discriminator), fixed-width values,
and length-delimited buffers across the C ABI, never Rust layouts or unbounded
C strings. Binary fields preserve every [byte](TERMS.md#byte), including
[NUL](TERMS.md#nul) and invalid [UTF-8](TERMS.md#unicode-and-utf-8).
[Native paths](TERMS.md#native-path) use absolute Unix bytes; text uses validated
UTF-8. [Native shared values](monitor-client/README.md#shared-values) fix widths,
presence and identity representations; [terminal representation/application](terminal-integration.md)
and [numerical budgets](../behaviour/monitor/operational-limits.md) retain their owners.
Exact tags, fields and results are in the header. Invalid/unsupported core tags
reject explicitly, never become extensions.

This in-process contract uses no checksum. Copies, immutable
deliveries, explicit lengths, and ownership address its relevant data-lifetime
risks; checksums provide neither memory safety nor authentication.
The [agreed daemon wire](daemon-client/README.md#framing-and-validation) also
has no initial checksum; a future negotiated integrity revision is a separate
decision.

### Ownership, wakeup, and cleanup

Submit borrows arguments only during the call. Before returning local acceptance,
the library copies all accepted data. Rejected submissions retain no ownership
and produce no later [response](TERMS.md#response). Drain transfers an immutable library-owned
delivery object, including any response or [event](TERMS.md#event), which the monitor releases
through the same loaded library. No monitor allocator crosses the boundary.
Delivered-object release is thread-safe subject to coordinated instance lifetime.

All delivered objects must be released before instance destruction. Outstanding
objects explicitly prevent successful destruction rather than leaving dangling
data. Thread-safe handles do not permit concurrent destruction: stop new calls
and release deliveries before dedicated-thread cleanup completes. A closed
stream or final session event does not prove that the instance is cleaned up,
the thread returned, or the monitor joined it. Exact [destruction/release
calls](monitor-client/README.md#function-calls-and-ownership) keep these stages distinct;
shutdown coordination follows the
[agreed lifecycle](../behaviour/monitor/lifecycle.md#coordinated-shutdown).
Prefix+x and catchable signals stop new input/acquisition/discovery/retries and
request every owned instance's shutdown concurrently through independent control
handles. Include passive, attached, partially started and failing instances.
Each attempts required atomic [Leave](daemon-client/request-leave.md), closes
and cleans workers on its owner thread. Drain/release objects, end concurrent
handle use and join every owned
thread before clean exit. Lost Leave remains unconfirmed; closure or a final
event cannot substitute. Deadline stalls fail-stop the whole process after
best-effort host restoration/root clear and diagnostics, never an individual
thread kill or running-code unload. Numerical deadlines remain #69.
Explicit exit succeeds only after required current Leave outcomes, local
cleanup/joins and host restoration succeed; it never inherits selected command
status. Fatal monitor error, required Leave failure/uncertainty, incomplete
cleanup or restoration failure uses the
[climon2 error code](../../../GLOSSARY.md#installation). Catchable-signal exit uses
signal-style status after successful cleanup, otherwise that error code.
Previously resolved session-local failures alone do not poison a later clean
exit; prior transport loss remains recorded as unconfirmed, not accepted detach.

Each instance supplies a library-owned
[pollable notification descriptor](TERMS.md#pollable-notification-descriptor),
borrowed by the monitor until instance cleanup. It indicates that draining can
make progress or observe stream closure, not a record count. The monitor polls
it, but never directly reads, writes, or closes it; it calls interface drain.
The library manages race-free signaling and rearming so work or closure cannot
be missed. Cleanup coordination prevents polling a closed descriptor. This
avoids periodic polling and an extra monitor waiter thread per instance without
freezing platform-specific descriptor internals into the ABI.

### Admission, correlation, and authority

Local acceptance means validated and queued, not daemon acceptance or application
consumption. Submission explicitly distinguishes accepted, full, and closed,
and rejects malformed records, invalid length/null-buffer descriptions,
unsupported operation tags, and invalid [request IDs](TERMS.md#request-id).
Length validation does not make invalid native pointers safe.

Monitor-assigned accepted request IDs increase strictly within the instance
lifetime, without wrap. Reject an ID at or below the last accepted ID.
Locally rejected submissions do not consume an ID. The monitor serializes
per-instance submissions when allocating IDs; instances are independent.
A [high-water mark](TERMS.md#high-water-mark), not an unbounded used-ID set,
is sufficient. Exhaustion explicitly requires a new instance; exact width
is nonzero `uint64_t` in [revision 1](monitor-client/README.md#shared-values).
IDs are not [idempotency keys](TERMS.md#idempotency-key) and
provide no retry mechanism.

Every accepted [request](TERMS.md#request) has one correlated eventual success/failure outcome
while communication remains healthy. No locally rejected request has a later
outcome. Execution follows accepted submission order except independent
shutdown. Runtime execution rechecks connection, generation, control, and
restoration authority that may have changed since admission. Known unexecuted
stale work gets an explicit asynchronous rejection; uncertain execution is
[indeterminate](TERMS.md#indeterminate-outcome). Never silently discard stale work or reassign it to new authority.

### Delivery order and bounded progress

Each instance has one [FIFO](TERMS.md#fifo) response-and-event stream preserving
causal control, restoration, and result order. Initial title/status and later
changes have no subscription gap. There is no ordering between instances.
Receiving restoration completion does not prove the monitor applied it.

Full queues apply [backpressure](TERMS.md#backpressure) to ordinary production
and socket reading as needed.
Required records cannot be dropped, overwritten, or reordered. In particular,
status transitions, attention episodes, responses, and control/restoration
boundaries cannot be lossily coalesced. Other clients and global monitor
actions remain responsive; shutdown and failure detection remain serviceable.

Bounds account for bytes, records, and drained-but-unreleased objects.
Draining without releasing must not permit indefinite growth. If the progress
policy in #69 cannot be met, fail explicitly instead of growing indefinitely.
Stream closure/failure reason is observable outside ordinary queue capacity;
already queued records remain ahead of the terminal indication when draining.
An instance stream's closure is distinct from recoverable daemon connection
loss. This contract does not require reconnect to close/reopen the stream.

### Capabilities and connection generations

After interface-revision and exact-release validation, the instance reports
[capabilities](../../../GLOSSARY.md#sessions) actually available through both library
and daemon. Required support is checked before control acquisition and operation
submission. Unsupported operations fail explicitly. Optional title/status
support is reported unavailable/unknown, never fabricated as an empty title or
cleared attention. A supported empty title remains valid and selects the
command-name fallback. Optional core `focus-v1` enables
[application focus](monitor-client/request-focus.md); missing library/daemon
support is a visible focus limitation, not a reason to block an otherwise usable
session. It is not a plugin extension or release-number inference.
Lifecycle uses the existing agreed Leave/Kill/control contract; this amendment
assigns no new capability or operation schema.

Unknown core operation/event tags are invalid, not implicit extensions.
Future [namespaced](TERMS.md#namespaced-name) extensions require explicit name/revision negotiation of the
intersection including relay support; their [APIs](TERMS.md#api) and delivery implementation
remain deferred.

Each instance distinguishes a
[connection generation](TERMS.md#connection-generation) from request IDs,
[control periods](../../../GLOSSARY.md#attachment-and-control),
[restoration IDs](../../../GLOSSARY.md#protocol-and-plugin-concepts), and attention episodes. Unfinished work from
a lost connection is indeterminate, with no automatic retry/replay. Queued
authority-sensitive work for the old generation never crosses to the new
connection. Reconnect is passive; retained metadata and the monitor-owned
terminal model remain stale until refreshed. Application input requires fresh
control, restoration, and monitor-applied barriers. Confirmed final command
outcomes remain distinct from loss and stop reconnecting.

For each generation, explicitly delimit initial core metadata synchronization
from subsequent changes. Report current title, effective status, and stored
UTC change time, or an explicit unsupported indication. Supply reliable attention
episode and final-outcome identities; UTC timestamps are not unique identities.
This gap-free metadata barrier is independent of control, readiness, and
selection, and never makes stale terminal content fresh. Exact event forms,
timestamps, identities, and daemon mapping are specified by
[wire revision 1](daemon-client/README.md) and
[native revision 1](monitor-client/README.md).
Native recovery uses a fresh caller-assigned generation/instance only after prior
cleanup/join, never an implicit reconnect inside an existing instance.
The [lifecycle contract](../behaviour/monitor/lifecycle.md#history-and-passive-recovery)
owns exponential passive recovery, recreation and cleanup. Numerical schedules
and budgets remain #69. Pause new retry scheduling during shutdown or confirmed
termination/removal; passive outcome recovery may establish an indeterminate
Kill result without replay.

### Semantic examples

These are valid behavioral timelines, not assigned calls or serialized records.
Every operation page inherits them. [C examples](monitor-client/examples.c)
and each core message page now show valid native records/calls.

| Situation | Required observations |
|---|---|
| Library compatibility | A library supports the requested revision but reports the wrong session release. Reject it before creating an instance or daemon connection; retain the opened library until monitor exit. Supporting the revision does not waive exact-release validation. |
| Local admission | ID 10 is accepted and copied. ID 11 is rejected as full: no ownership or later outcome. Retrying local admission with ID 11 can succeed; submitting ID 10 again cannot. |
| Literal binary data | A supported binary submission contains hexadecimal `00 ff 41`; its accepted copy preserves all three bytes. Display deliveries preserve exact ordered terminal bytes too. Neither NUL nor invalid UTF-8 truncates or changes them. This does not imply input echo or direct host output. |
| Control and restoration | A [grant](../../../GLOSSARY.md#attachment-and-control) precedes its restoration start, ordered updates, and completion in the single stream. The complete current restoration replaces the model/history; application permits Ready without selection/paint, while selected input additionally waits for coherent paint. Revocation invalidates readiness. |
| Passive metadata | A generation's initial title/status synchronization is delimited, then changes follow gap-free while unselected or revoked. Unchanged status retains its stored UTC time; a new attention episode uses its own identity, not timestamp uniqueness. |
| Resize and viewing | Only the selected [controller](../../../GLOSSARY.md#attachment-and-control) receives monitor resize requests; local acceptance is not actual geometry. Report viewing once ready and unobstructed. Switching away or opening menus clears viewing without releasing control/restoration. Explicit selection/focus or menu close/cancel may separately acknowledge current pending attention once ready and viewing; opening/browsing never acknowledges. Cancel deferred acknowledgement if target context changes. Visibility alone never acknowledges a new request. |
| Menu notifications | Attention arriving while a menu is open toasts immediately over it without a daemon round trip. Still-pending attention never toasted becomes eligible on menu opening. Deduplicate per episode over this monitor lifetime, including reconnects. A final first learned viewed stays suppressed forever here; another client's viewing never suppresses this monitor. |
| Canonical geometry | Outer host 600x251 gives desire 600x250; daemon effective size 500x200 is used for every monitor-mode display/restoration byte. Paint that rectangle below the top row with blank margins; ordinary attach still uses its own direct/derived host policy. |
| Focus | Host focus loss clears viewing, not control/readiness. Dedicated non-user Focus stores current desired application focus even with reporting off; enabled reporting emits matching bytes once, never attention acknowledgement. Nonmodal toasts/scrollback retain keyboard focus; modal menus do not. |
| Attention jump/hints | Configured prefix + lowercase j chooses the newest currently connected needs_attention row at keypress, not a toast's saved target. Close overlays, acquire only if needed, apply/paint, then acknowledge when unobstructed/ready. No eligible row keeps selection and reports it. show_key_hints hides implicit hints, not bindings or application text. Enabled selector hints appear in a bottom footer inside the menu; disabling them reclaims that space. |
| Succession | On any controller departure, most-recent-control existing attachment wins first; otherwise newest accepted passive registration is promoted before final-participant policy. Inheritance does not select, resize, send input or acknowledge. Promoted membership persists through switching/revocation. Committed termination overrides succession. |
| Core notification/progress | A recognized logical notification requests attention; repeated requests while pending are not new episodes. Progress metadata alone does not request attention or confirm command completion. Delivery remains independent of control and does not permit screen mirroring. |
| Deliberate detach | Local acceptance queues detach. Only daemon acceptance confirms deliberate detach; neither that outcome nor stream closure proves instance cleanup or a joined thread. No timeout or shutdown fabricates success. |
| Loss during work | An accepted unfinished request on the lost generation is indeterminate. Passive reconnect does not replay it or carry old-generation input forward. Retained terminal content stays stale until fresh control/restoration/application. |
| Stalled consumer | Capacity includes drained unreleased objects. Ordinary production backpressures; other instances and shutdown progress. Closure reason remains observable, with queued records drained before the terminal indication. Objects are released and polling/calls coordinated before cleanup; closure alone is not a join. |

### Required future evidence

These are specification requirements. Automated tests now cover parts of
them, such as simultaneous mixed-release libraries
([#97](https://github.com/jackgeek/climon2/issues/97)) and library limits and
accounting ([#99](https://github.com/jackgeek/climon2/issues/99)); this
release qualification ([#122](https://github.com/jackgeek/climon2/issues/122)) runs the rest on each candidate, and no release
has been qualified yet:

- Automated public-ABI evidence with multiple same-release instances and
  simultaneous mixed-release libraries, including explicit unsupported/rejection
  behavior and ownership on accepted, rejected, and outstanding deliveries.
- Causal ordering and applied-restoration readiness, passive initial metadata
  and changes, reconnect generations, attention/final-outcome identities,
  arbitrary-byte integrity, stall/backpressure isolation, and truthful failure,
  detach, cleanup, and thread-completion reporting.
- Representative input/display latency, event throughput, memory use, and
  global responsiveness measurements under normal operation, load, and stalls.
  Numerical thresholds are in the [agreed operational limits](../behaviour/monitor/operational-limits.md).
- Compile the [native examples and layout assertions](monitor-client/check.py)
  for both macOS ABIs and x86-64 GNU/Linux. These declaration checks are automated here, not evidence
  of working libraries. [Required runtime evidence](monitor-client/README.md#required-runtime-evidence)
  must use real packaged libraries.

### Remaining decision boundaries

The queue, ownership, notification, admission, ordering, and generation rules
above are agreed; they are not placeholders for #71 to redesign. Agreed
contracts and remaining open decisions are distinguished below:

| Owner | Agreed contract or unresolved details |
|---|---|
| [Daemon wire #67](https://github.com/jackgeek/climon2/issues/67) | Agreed [wire revision 1](daemon-client/README.md), partly implemented: passive monitor connections ([#86](https://github.com/jackgeek/climon2/issues/86)), interactive attachments with optional core Focus ([#87](https://github.com/jackgeek/climon2/issues/87)), Command final with standalone Kill ([#88](https://github.com/jackgeek/climon2/issues/88)), heuristic `status-v1` with Set viewing and Acknowledge ([#91](https://github.com/jackgeek/climon2/issues/91)), explicit and fallback status from program status and shell markers ([#92](https://github.com/jackgeek/climon2/issues/92)), monitors that acquire and inherit control ([#89](https://github.com/jackgeek/climon2/issues/89)), and notifications and progress ([#98](https://github.com/jackgeek/climon2/issues/98)). No remaining schema decision is delegated back to #67. |
| [Terminal integration #63](https://github.com/jackgeek/climon2/issues/63) | Agreed [terminal contract](terminal-integration.md), including canonical monitor geometry and optional core Focus; session daemons implement wire Focus ([#87](https://github.com/jackgeek/climon2/issues/87)) and canonical monitor-mode presentation ([#89](https://github.com/jackgeek/climon2/issues/89)); the monitor implements selection, painting, input, paste, mouse and focus ([#108](https://github.com/jackgeek/climon2/issues/108) to [#111](https://github.com/jackgeek/climon2/issues/111)), history ([#112](https://github.com/jackgeek/climon2/issues/112)), the selector ([#113](https://github.com/jackgeek/climon2/issues/113)), attention jump, hints and host reports ([#114](https://github.com/jackgeek/climon2/issues/114)), toasts ([#115](https://github.com/jackgeek/climon2/issues/115)) and read-only history with recovery ([#116](https://github.com/jackgeek/climon2/issues/116)). Numeric transfer budgets and native declarations are agreed in #69/#71; the client library implements the native declarations and its limits ([#99](https://github.com/jackgeek/climon2/issues/99)), and the monitor its budgets ([#119](https://github.com/jackgeek/climon2/issues/119), [#120](https://github.com/jackgeek/climon2/issues/120)). |
| [Lifecycle #61](https://github.com/jackgeek/climon2/issues/61) | Agreed [lifecycle contract](../behaviour/monitor/lifecycle.md), implemented by the monitor: exponential passive recovery ([#116](https://github.com/jackgeek/climon2/issues/116)), Delete/removal ([#117](https://github.com/jackgeek/climon2/issues/117)), concurrent truthful shutdown and bounded whole-process fail-stop ([#118](https://github.com/jackgeek/climon2/issues/118)). Headless creation and lifetime are implemented. Not implemented: the coordinated failure when required identities cannot be retained. Numerical schedules remain #69; no general native-crash survival guarantee. |
| [Operational limits #69](https://github.com/jackgeek/climon2/issues/69) | Agreed [numerical specification](../behaviour/monitor/operational-limits.md). Its `monitor.limits` configuration is validated by the monitor and `exec` and reported by the monitor ([#82](https://github.com/jackgeek/climon2/issues/82)); `exec` passes the notification assembly limits to the session daemon, which enforces them with the release-constant notification text and record bounds ([#98](https://github.com/jackgeek/climon2/issues/98)). The monitor alone also refuses limits that cannot hold one session's mandatory current model and recent page ([#167](https://github.com/jackgeek/climon2/issues/167)); the monitor reserves each release load and client create against `memory.total_bytes`, the operating-system descriptor limit and the `resources` reservations, raised to a loaded release's qualified requirements, and starts clients fairly ([#120](https://github.com/jackgeek/climon2/issues/120)); it bounds discovery ([#106](https://github.com/jackgeek/climon2/issues/106)), retries ([#116](https://github.com/jackgeek/climon2/issues/116)), shutdown ([#118](https://github.com/jackgeek/climon2/issues/118)), and history, reports in flight and cadence ([#119](https://github.com/jackgeek/climon2/issues/119)). Not implemented: what the [specification's status](../behaviour/monitor/operational-limits.md) leaves out, such as `discovery.stall_ms`, refusing a live model's growth for aggregate pressure before allocation, and charging renderer, search, toast and identity state. |
| [Interface schemas #71](https://github.com/jackgeek/climon2/issues/71) | Agreed [native revision 1](monitor-client/README.md), header, operation/event schemas, ownership/cleanup calls, errors, limits/measurements and C examples. The client library implements every revision-1 family except plugin extensions, as the [native status](monitor-client/README.md) lists. Declaration checks do not replace future real same/mixed-release compatibility/rejection evidence. |
| [Linux integration #125](https://github.com/jackgeek/climon2/issues/125) | Agreed Linux loading in the [embedding design](library-embedding.md#artifact-and-dependency-lookup) and [native revision 1](monitor-client/README.md). Every Linux release ships `libclimon2_client.so` as one self-contained library, and release checks enforce its single export, no [soname](TERMS.md#soname), [RPATH or RUNPATH](TERMS.md#rpath-and-runpath), and [glibc](TERMS.md#glibc)-only dependencies ([#75](https://github.com/jackgeek/climon2/issues/75)). Its notification descriptor is one close-on-exec, nonblocking `eventfd` ([#85](https://github.com/jackgeek/climon2/issues/85)). The monitor loads it by exact path ([#106](https://github.com/jackgeek/climon2/issues/106)). Release qualification ([#122](https://github.com/jackgeek/climon2/issues/122)) runs on Linux alongside both macOS architectures; no release has been qualified yet. |

## Compatibility, errors, and limits

The agreed library design uses a multi-instance, cross-release-safe embedding
interface and the communication contract above;
[complete revision-1 declarations](monitor-client/client-v1.h) are specified.
Exact-release executable/library matching alone is not sufficient when a monitor
loads libraries from several releases simultaneously. Never substitute a
different installed release or
silently ignore a missing library.

Preserve structured operations and explicit outcomes; terminal output is not a
control interface. A grant is not [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
input acceptance is not application consumption, and daemon-accepted
[detach](../../../GLOSSARY.md#attachment-and-control) is not completed client cleanup.
Communication failure leaves unfinished outcomes
[indeterminate](TERMS.md#indeterminate-outcome); it does not authorize retries
or fabricated success.

Use bounded per-client and monitor-wide queues with
[backpressure](TERMS.md#backpressure), explicit failure reporting, and responsive
monitor actions. Numerical budgets, startup concurrency, deadlines, and
admission/exhaustion policies are specified in
[Monitor operational limits](../behaviour/monitor/operational-limits.md).
There is no monitor-client frame maximum. Request IDs are nonzero `uint64_t`;
lengths/identities, errors, rejection and incompatible-revision rules are in the
[native contract](monitor-client/README.md).

## Future plugin authors

Plugins are anticipated in the monitor and daemon, not the client library.
The client relays negotiated [extensions](../../../GLOSSARY.md#protocol-and-plugin-concepts)
without interpreting plugin-owned [payloads](TERMS.md#payload) as core commands.
Support must be explicitly negotiated by
[namespaced name](TERMS.md#namespaced-name) and revision, including relay support.
Required support must be checked before control acquisition; missing optional
support disables that extension. Exact plugin negotiation schemas remain deferred;
revision 1 publishes no extension calls/tags. A separately specified revision
must add them explicitly, without delaying core implementation.
These future extension requirements do not require implementing plugins now.
Core title/status and structured notification/progress delivery do not use
plugin negotiation.

Bidirectional extension [requests](TERMS.md#request), outcomes, and [events](TERMS.md#event) must work for passive,
background, and revoked clients as well as the selected
[controller](../../../GLOSSARY.md#attachment-and-control). Receiving
plugin data does not require control: an attention status must reach the
monitor even while another attachment controls that session. This does not
permit core screen mirroring, application input, or resize without control.

Preserve core ordering, readiness, control, truthful outcomes, and resource
limits. Injected work needs its own correlation and outcome; removal before
execution requires explicit rejection. Replacement work must account truthfully
for the original outcome. [Responses](TERMS.md#response) and essential state changes cannot silently
disappear. The shared in-process contract preserves these guarantees without
assigning a plugin API.

Future daemon plugins need distinct [interception seams](TERMS.md#interception-seam)
for PTY traffic and structured daemon/socket traffic, observation before
filtering, and read-only terminal snapshots. The agreed OSC 7501, OSC 133,
OSC 9/99/777, input, and viewport strategies now belong to daemon core.
[BEL](TERMS.md#bel) and other future plugin strategies remain motivating
examples, not implemented hooks or frozen plugin schemas.
Plugin interfaces, examples, implementation, configuration, and
[sandboxing](TERMS.md#sandboxing) are deferred. The agreed core
[attention strategy and toast behavior](../behaviour/session-status.md) are in scope;
additional detection strategies and plugin delivery are not prerequisites.

[QUIC](TERMS.md#quic), request-retry deduplication, outcome caches, and replay
policies remain out of scope. Passive connection retry is in scope; it is not
permission to retry indeterminate operations. [Request IDs](TERMS.md#request-id)
alone do not provide retry safety or exactly-once execution.

## Sources and unresolved decisions

- [Define monitor client ownership and control](https://github.com/jackgeek/climon2/issues/58):
  canonical thread-based hosting, eager clients, passive connections, and control;
  later core-status mechanics defer plugin implementation.
- [Define monitor startup and session discovery](https://github.com/jackgeek/climon2/issues/59):
  agreed startup/discovery integration, revised status/time ordering and core
  title/status reporting; implemented by the monitor ([#106](https://github.com/jackgeek/climon2/issues/106),
  [#107](https://github.com/jackgeek/climon2/issues/107), [#266](https://github.com/jackgeek/climon2/issues/266)).
- [Define version-matched client library loading and lifetime](https://github.com/jackgeek/climon2/issues/65):
  agreed [multi-instance embedding, compatibility, and lifetime](library-embedding.md);
  the library side of instance lifetime is implemented
  ([#85](https://github.com/jackgeek/climon2/issues/85)), and the monitor
  loads and hosts it ([#106](https://github.com/jackgeek/climon2/issues/106), [#118](https://github.com/jackgeek/climon2/issues/118), [#120](https://github.com/jackgeek/climon2/issues/120));
  [complete native declarations](monitor-client/client-v1.h) are agreed.
- [Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66):
  agreed queues, tagged records, ownership, notification, correlation/order,
  validation, compatibility, bounded progress, core metadata, and future
  evidence requirements above. The client library implements delivery
  ownership, notification and release
  ([#85](https://github.com/jackgeek/climon2/issues/85)), and request
  admission, correlation and the records of passive monitor connections
  ([#93](https://github.com/jackgeek/climon2/issues/93)), and the other
  revision-1 records ([#94](https://github.com/jackgeek/climon2/issues/94) to [#96](https://github.com/jackgeek/climon2/issues/96), [#98](https://github.com/jackgeek/climon2/issues/98)); the monitor consumes
  them ([#106](https://github.com/jackgeek/climon2/issues/106) to [#120](https://github.com/jackgeek/climon2/issues/120)). Plugin implementation remains deferred.
- [Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71):
  agreed [native revision 1](monitor-client/README.md), exact fields/calls/events,
  results/errors, ownership/cleanup/measurements and examples. The shipped
  library implements the bootstrap and description
  ([#75](https://github.com/jackgeek/climon2/issues/75)), the instance
  lifecycle calls ([#85](https://github.com/jackgeek/climon2/issues/85)) and
  passive monitor connections
  ([#93](https://github.com/jackgeek/climon2/issues/93)), control and display
  ([#94](https://github.com/jackgeek/climon2/issues/94)), status ([#95](https://github.com/jackgeek/climon2/issues/95)), Kill ([#96](https://github.com/jackgeek/climon2/issues/96)) and notifications and progress
  ([#98](https://github.com/jackgeek/climon2/issues/98)). Release qualification ([#122](https://github.com/jackgeek/climon2/issues/122)) judges final monitor
  acceptance; no release has been qualified yet.
- [Freeze daemon-client communication contract](https://github.com/jackgeek/climon2/issues/67):
  agreed [revision-1 post-hello wire](daemon-client/README.md), passive
  succession, metadata, outcomes and validation; session daemons implement
  its passive monitor connections ([#86](https://github.com/jackgeek/climon2/issues/86)),
  interactive attachments ([#87](https://github.com/jackgeek/climon2/issues/87)),
  Command final with standalone Kill ([#88](https://github.com/jackgeek/climon2/issues/88))
  monitors that acquire and inherit control
  ([#89](https://github.com/jackgeek/climon2/issues/89)), bounds ([#90](https://github.com/jackgeek/climon2/issues/90)),
  status ([#91](https://github.com/jackgeek/climon2/issues/91), [#92](https://github.com/jackgeek/climon2/issues/92)) and notifications and progress ([#98](https://github.com/jackgeek/climon2/issues/98)).
- [Choose the monitor terminal integration contract](https://github.com/jackgeek/climon2/issues/63):
  agreed [terminal/input/geometry/restoration contract](terminal-integration.md),
  optional core Focus and #64/#59 input/config amendments; session daemons
  implement wire Focus ([#87](https://github.com/jackgeek/climon2/issues/87))
  and canonical monitor-mode presentation
  ([#89](https://github.com/jackgeek/climon2/issues/89)), and the monitor
  implements its side ([#108](https://github.com/jackgeek/climon2/issues/108) to [#116](https://github.com/jackgeek/climon2/issues/116)).
- [Define monitor exit and failure behavior](https://github.com/jackgeek/climon2/issues/61):
  agreed [lifecycle](../behaviour/monitor/lifecycle.md), implemented by the monitor:
  recovery ([#116](https://github.com/jackgeek/climon2/issues/116)), selector removal ([#117](https://github.com/jackgeek/climon2/issues/117)), and concurrent
  shutdown/fail-stop with truthful outcomes ([#118](https://github.com/jackgeek/climon2/issues/118)). Its headless
  zero-client lifetime is implemented.
- [Define monitor operational limits](https://github.com/jackgeek/climon2/issues/69):
  agreed [numerical bounds, progress, admission and exhaustion](../behaviour/monitor/operational-limits.md),
  partly implemented ([#119](https://github.com/jackgeek/climon2/issues/119), [#120](https://github.com/jackgeek/climon2/issues/120); its status lists what), with
  boundary/stall/cleanup and measurement obligations; the measurement
  harness is implemented ([#121](https://github.com/jackgeek/climon2/issues/121)).
- [Final automated monitor acceptance](https://github.com/jackgeek/climon2/issues/62):
  agreed [acceptance matrix](../behaviour/monitor/acceptance.md). Release qualification
  ([#122](https://github.com/jackgeek/climon2/issues/122)) runs real-monitor/library conformance, lifecycle, boundary/fault
  coverage and measurements on every candidate before it is published; no
  release has been qualified yet.

The implemented hello remains specified by
[current framing/messages](../../../crates/climon2-core/src/protocol.rs),
[hello extensions](../../../crates/climon2-core/src/hello.rs), and
[its ADR](../../adr/0001-version-matched-client-and-frozen-hello.md).
The [protocol-docs skill](../../../.agents/skills/protocol-docs/SKILL.md) owns
synchronized updates, terminology links, examples, and completeness requirements.
