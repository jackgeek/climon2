# Monitor terminal integration

**Status: agreed specification from [#63](https://github.com/jackgeek/climon2/issues/63), almost entirely unimplemented.**
This also records the confirmed input/configuration amendments to
[#64](https://github.com/jackgeek/climon2/issues/64) and
[attention](../../GLOSSARY.md#status-and-attention)-jump amendment to
[#59](https://github.com/jackgeek/climon2/issues/59). This document supplies no
executable changes or evidence itself.

**Implemented with packaged evidence ([#74](https://github.com/jackgeek/climon2/issues/74)):**
only the empty [monitor](../../GLOSSARY.md#sessions), which has no
[sessions](../../GLOSSARY.md#sessions) to show. It takes its own alternate screen
with one permanent top row, enables [host](../../GLOSSARY.md#sessions)
bracketed-paste, mouse and focus reporting, and never changes the host title.
Only the typed [client escape prefix](../../GLOSSARY.md#attachment-and-control)
followed by x acts: pasted [bytes](TERMS.md#byte) are data, never complete a
pending prefix and leave none pending. A host resize repaints the whole screen
without blanking it or showing the cursor. Every exit writes the
[release](../../GLOSSARY.md#restoration-and-presentation) bytes to the host
before any diagnostic. Everything else in this document remains agreed and
unimplemented.

The later [selector and attention-presentation decision](../monitor-selector.md)
confirms the captionless double-bordered selector, always-focused search,
explicit read-only disconnected/final history selection and animated
[toasts](TERMS.md#toast).
It also adds the monitor's own
[host program-status reports and configurable bell](README.md#monitor-to-host-status-and-bell).
These are planned presentation/host signals, not session-byte passthrough or changed daemon
[FSMs](TERMS.md#fsm); the shipped empty monitor emits none of them.

The [monitor](../../GLOSSARY.md#sessions) provides an
[RDP-like](TERMS.md#rdp) remote terminal experience, not a read-only viewer.
Use the [overview](README.md), [domain glossary](../../GLOSSARY.md), and
[technical terms](TERMS.md) with this contract. The
[session-status mechanics](../session-status.md) and their state machines remain
unchanged. Selector layout belongs to [#60](https://github.com/jackgeek/climon2/issues/60),
lifecycle/cleanup/selection fallback to the [agreed lifecycle contract](../monitor-lifecycle.md),
numerical limits to the [agreed operational specification](../monitor-operational-limits.md), native
declarations to the agreed [native revision 1](monitor-client/core-v1.md), and the final
acceptance matrix to [#62](https://github.com/jackgeek/climon2/issues/62).

## Ownership, representation, and fidelity

The [session daemon](../../GLOSSARY.md#sessions) remains the sole authority for
terminal state and terminal-query replies. In-process exact-release
[client libraries](../../GLOSSARY.md#sessions) deliver ordered, exact terminal
[bytes](TERMS.md#byte) and explicit
[control](../../GLOSSARY.md#attachment-and-control)/
[restoration](../../GLOSSARY.md#restoration-and-presentation) authority through
the existing bounded [C-compatible](TERMS.md#c-abi)
[record queues](README.md#shared-in-process-communication-contract).
There is no new serialized in-process [envelope](TERMS.md#envelope) or
cross-release cell [schema](TERMS.md#schema).

The monitor parses those bytes into its own per-[session](../../GLOSSARY.md#sessions)
[virtual terminal](../../GLOSSARY.md#sessions), using `climon2-vt`, and Ratatui
paints its read-only Rust views. These views of cells, modes, cursor, and history
are a planned addition; current `climon2-vt` exposes no public cell views, and the
monitor uses it only for its release bytes. The monitor runs as its own
`climon2-monitor` executable, beside the other executables of its release
([ADR 0004](../adr/0004-crate-layout-and-dependency-direction.md)); this contract
chooses no workspace crate.
The [allocation-aware prerequisite](../monitor-operational-limits.md#allocation-aware-terminal-prerequisite)
explicitly reopens the unbounded combining-data/infallible-interface assumptions
for future monitor qualification. Current engine behavior is unchanged.
Never write session bytes directly to the [host](../../GLOSSARY.md#sessions).
The monitor parser must not send duplicate query replies; host replies remain
monitor-local and bypass shortcut detection and real-user application input.
There is no arbitrary terminal-reply injection operation.

Preserve supported normal/alternate-screen storage, wide/combining characters,
colors/rendition, cursor, modes, and
[retained history](../../GLOSSARY.md#restoration-and-presentation).
This is fidelity to the
[qualified engine and filtering/restoration limits](../adr/0007-daemon-owned-virtual-terminal.md),
not a claim of complete xterm support. The current engine supports alternate
screen mode 1049, not 47/1047. Current restoration omits scroll regions, tab
stops, character sets, palette changes, keyboard-protocol modes, and hyperlinks;
the normal saved-cursor exception and moving history restoration still apply.
Keyboard-protocol negotiation, [OSC](TERMS.md#osc) clipboard and hyperlinks
remain filtered, not newly supported by choosing a renderer. Invalid
[UTF-8](TERMS.md#unicode-and-utf-8) application bytes alone are not a protocol error.

## Canonical monitor presentation: amendment to daemon wire revision 1

For connections opened in monitor mode, **all daemon display/restoration bytes
are serialized against effective session [geometry](TERMS.md#geometry)**,
including passive promotion and background
[grants](../../GLOSSARY.md#attachment-and-control). They are not serialized
against the outer host or an unclamped requested size. The geometry in
[Grant](daemon-client/event-control-granted.md),
[Restoration begin](daemon-client/event-restoration-begin.md), and
[Geometry](daemon-client/event-geometry.md) is authoritative.
The monitor parses at those dimensions and paints local margins itself.
No additional presentation-geometry field is introduced.

This is an agreed, unimplemented amendment, not existing behavior at `cb585d0`.
Current ordinary-client restoration can use host coordinates: a 600x250 host
around a 500x200 session cannot safely be reparsed as a 500x200 model.
The single wire geometry suffices only with this new monitor policy.
Ordinary interactive attach,
[direct/derived presentation](../../GLOSSARY.md#restoration-and-presentation),
[native scrollback](../../GLOSSARY.md#restoration-and-presentation), and
[release](../../GLOSSARY.md#restoration-and-presentation) remain unchanged.
Monitor-mode canonical presentation must not
inherit ordinary host-history preservation or host-size-dependent serialization.

Background [controllers](../../GLOSSARY.md#attachment-and-control) apply fresh
restorations and maintain live models without
selection or paint. Passive clients that have never controlled have no terminal
history. [Revocation](../../GLOSSARY.md#attachment-and-control) still means a
[placeholder](../../GLOSSARY.md#attachment-and-control), not live core mirroring.

## Host canvas, geometry, and coherent readiness

The monitor uses its own alternate screen regardless of the application's
normal/alternate mode. Reserve one permanent top status row, with no border or
sidebar. Desired application geometry is host width by `(host height - 1)`.
While the selected monitor client [controls](../../GLOSSARY.md#attachment-and-control)
the session, the daemon must resize toward that desire subject to explicit
supported limits. Effective daemon geometry, not desired geometry, determines
the session rectangle: top-left immediately below chrome, unused right/bottom
cells blank. Do not locally fake [PTY](TERMS.md#pseudo-terminal-pty) reflow.
Background clients receive neither monitor resize nor application input.

Opening/closing overlays does not resize the application. Actual host resize
still updates the selected controller under an overlay. Overlay layout follows
host dimensions immediately, independently of daemon acknowledgement; the daemon
does not size or move monitor [UI](TERMS.md#ui).
If there is too little supported application space, show explicit
`Terminal too small`, retain clients/control/models, set
[viewing](../../GLOSSARY.md#status-and-attention) false, and send neither
application input nor invalid resize. Global commands remain usable.
Marquee applies only to overflowing chrome labels, never to the application canvas.

During resize/restoration retain the last coherent frame clipped/padded to the
current area; do not repeatedly blank or flash a restoring screen. Construct the
replacement model separately and swap one new coherent frame when ready.
[Restoration](../../GLOSSARY.md#restoration-and-presentation) transactionally
replaces the complete model/history; never union or deduplicate old and new rows.
Partial or superseded models never become current. A held transition
frame is not a current/input-ready model and adds no connected revoked-cache
inspection route. Replacement can discard older local
history absent from the authoritative restoration.

There are distinct barriers:

1. Matching complete restoration received and applied to the current model
   permits [Ready](monitor-client/request-ready.md) and resize progress, including
   in the background. It does not require painting an old oversized screen.
2. Selected application input additionally requires current authority,
   applied [readiness](../../GLOSSARY.md#protocol-and-plugin-concepts),
   a coherent current frame painted in a usable area, and a
   live keyboard target with no modal UI or stale inspection.
3. Viewing additionally requires the live unobstructed viewport and host focus
   when reportable. A nonmodal notification may gate viewing without gating keys.

Coalesce only **unsent** resize intent. Preserve every submitted request's
correlated outcome; never replay an
[indeterminate](TERMS.md#indeterminate-outcome) request. New restoration,
revocation, disconnect, and invalid target gate input immediately.
Respect supported synchronized updates for coherent frames, but do not stall
required UI, selector, or exit progress. Numerical scheduling/rendering budgets
are in [operational limits](../monitor-operational-limits.md#ui-sampling-and-responsiveness).

## Normal history, alternate screen, and stale inspection

Normal-screen history is bounded internal
[retained history](../../GLOSSARY.md#restoration-and-presentation), not host
[native scrollback](../../GLOSSARY.md#restoration-and-presentation).
Normal live scrolling is terminal-style: wheel without application mouse capture
scrolls retained normal history. The viewport stays anchored while output updates
the live model. Typing returns to live and forwards to the selected controlling
ready target; no separate live-history inspection mode consumes ordinary keys.
Scrolling alone changes neither application keyboard focus nor attention.

On eviction, move an invalidated anchor to the oldest retained row and show an
explicit [truncation notice](../../GLOSSARY.md#restoration-and-presentation).
Do not pin unbounded history or jump automatically to live.
Under memory pressure, [operational limits](../monitor-operational-limits.md#memory-history-and-admission)
preserves the newest captured page and current screen state; never erase that
page to admit another client.
The alternate screen displays its own buffer; wheel never reveals normal history.
Returning to normal restores that buffer/history. Without application mouse
capture, alternate wheel becomes arrows only when alternate-scroll mode is on;
otherwise it does not scroll normal history.

Disconnected retained output is read-only; selector Enter explicitly opens it
without Acquire or acknowledgement. Overlay exactly `Reconnecting...` while
automatic passive recovery is scheduled or active. Permanent rejection instead
shows an explicit disconnected/error overlay. Connected revoked selection shows
the placeholder, with no separate revoked-cache inspection route.
Completed/failed sessions may show retained terminal content directly read-only.
Passive clients with no cache say no terminal content is available.
On reconnect remove the overlay; a non-controller shows its placeholder until
explicit acquisition. Cache alone cannot become a live model.
The [lifecycle contract](../monitor-lifecycle.md) keeps the monitor open through
empty/all-final states, preserves selected identity and forbids automatic
replacement acquisition. Recovery is automatic exponential passive backoff for
transport loss, recoverable startup failure (including missing libraries), and
safe worker panics; permanent rejection/invalid configuration and final outcomes
stop it. Fresh instances wait for prior cleanup/join. No indeterminate work is
replayed. Numerical backoff/budgets are in
[operational limits](../monitor-operational-limits.md); all behavior is unimplemented.

## Chrome, cursor, and host copy

Only the title/fallback **label** text marquees on overflow.
Status/control/error information stays stationary. Short labels remain static;
label changes restart the marquee; offsets and clipping respect
[display cells](TERMS.md#display-cell), including wide/combining characters.
Full details remain accessible in the selector. Session titles are inert labels,
not host-title updates.

Support the qualified cursor's position, visibility, shape, and blink. Hide it
when the live cursor is outside the scrolled viewport, covered, or unavailable;
never place it in chrome.

Native host selection/copy is allowed through the host's mouse-capture bypass
gesture, for example Shift-drag where supported. It copies visible text only.
There is no monitor clipboard or multipage selection engine, and no promise that
every host's default drag or bypass gesture is identical. **The developer's
acceptance of these limitations is provisional**, subject to implementation trial;
honest native-selection coverage may require manual checks.

## Transparent keyboard and exact paste

The goal is transparent terminal input, not prescribed fixed host modes or a
mandatory translation mechanism. Honor supported application cursor/keypad modes
through host-mode management or encoding as appropriate. Every ordinary key
outside agreed monitor actions/UI routing reaches the application with expected
encoding. Preserve literal/control/unrecognized bytes when encodings match;
do not invent distinctions between identical host encodings.
Supported legacy semantics do not imply new keyboard protocols.

Use the shared `client.escape_prefix` and `client.take_control_key`, existing
defaults and [C0](TERMS.md#c0)-key vocabulary from
[#64's resolution](https://github.com/jackgeek/climon2/issues/64#issuecomment-6599174136).
The monitor rejects a null prefix before entering the terminal; null takeover is
allowed. Ordinary attach semantics are unchanged.
Lowercase prefix+s opens the selector (no-op if already open);
prefix+x requests exit; prefix+? opens the
[key introduction/help dialog](../monitor-selector.md#key-introduction-and-help),
even when startup help or implicit hints are disabled (no-op if already open).
Prefix+d has no monitor detach meaning.
No prefix+h/prefix+r or additional history/retry binding is added.
Prefix twice emits one literal prefix; unknown suffixes emit prefix+suffix without
second interpretation, including a prefixed take-control key.
A lone prefix waits indefinitely; Escape is not cancellation.
The unprefixed takeover key only takes control of the selected revoked target,
is consumed rather than replayed, and is ordinary input when controlling/ready.
Selector/modal UI owns nonglobal keys/paste and never takes over via that key.
Empty, revoked, restoring, stale-inspection, and too-small targets do not buffer
application input for later.

Ctrl+[ remains allowed as prefix: bare Escape waits, and legacy
Alt+s/Alt+x/Alt+j/Alt+? can be identical to prefix+s/x/j/?.
Identical encodings receive identical treatment.
Recognized mouse reports, host replies, and non-key terminal events bypass
shortcut detection. Unbracketed paste remains indistinguishable from typing.
Behavior must be independent of read fragmentation/coalescing; process subsequent
input against the screen/state then in force, not discard an arbitrary read tail.
Once exit starts, accept no further application input.
Selector Delete is modal, default-Cancel and tied to the captured immutable
[session ID](../../GLOSSARY.md#protocol-and-plugin-concepts); it never retargets
or leaks input. [Termination/removal](../monitor-selector.md#delete-terminate-and-remove)
requires definitive termination followed by owned-instance cleanup/join before
run-local dismissal; fallback selection never acquires another session.

Host bracketed-paste detection is on. Preserve the payload exactly; send
application start/end markers **only if the application's bracketed-paste mode
is enabled**, otherwise payload alone. Paste never executes shortcuts, including
prefix+j. If paste begins after a pending prefix, emit one literal prefix to the
current accepting target (otherwise discard it), then handle paste as data.
Pasted bytes never complete a pending command; paste end leaves no pending prefix.

Opening/closing modal UI, switching target, changing control period, new
restoration, disconnect/revocation, or otherwise losing input validity clears
pending prefix and invalidates an open paste. Discard remaining payload through
its end marker even if a target later becomes ready. Never forward an old paste
or gesture to another session/generation/authority.

## Mouse, gestures, and notifications

Hit-test the application rectangle and translate coordinates past the top row
before encoding supported application mouse modes. Chrome and selector input
never goes to the application. Wheel follows application capture when enabled,
otherwise the normal/alternate rules above.

Each gesture owns its initial target. A newly appearing
[toast](TERMS.md#toast) does not steal an existing application drag. A new click
on a toast is monitor-owned and dismisses it, without implicit takeover.
If target/control becomes invalid, discard the remainder through gesture end;
do not send it to a background or replacement session. Paste and gestures share
the same target-validity principle.

Nonmodal notifications may cover application cells. They clear viewing when
covering the application but retain control, applied restoration, and keyboard
focus: ready keys still forward. Hiding one restores viewing when the viewport is
live, focused, ready and unobstructed, **without automatically acknowledging**.
Modal menus clear viewing and application keyboard focus without releasing control.
Toast appearance is not a target change for an existing paste or drag.

## Host focus and application focus

When the host reports focus loss, clear viewing, not control or applied readiness.
Focus-in alone never acknowledges attention. Without host focus reports there is
no invented detector: use best-effort assumed host focus, and document the limitation.

Application focus is a separate non-user core path,
[Focus](monitor-client/request-focus.md), mapped to
[wire Focus](daemon-client/request-focus.md). A selected ready host-focused
session without modal UI is keyboard-focused; normal scrollback and nonmodal
toasts do not change it. Switch/menu opening sends focus-out when authority and
readiness remain valid. A retained background controller may send this non-user
focus-out, but not application input. While restoring keep the latest local
desired focus; when newly ready synchronize current state, not old requests.

The daemon remembers desired focus for the current [attachment](../../GLOSSARY.md#attachment-and-control) even when
application focus-reporting mode is off. With reporting on and known state, emit
the matching sequence. Repeated state emits no duplicate; newly enabled reporting
emits current known state once. Focus never clears/acknowledges attention or counts
as real user input. Revocation/generation change never replays indeterminate work.

`focus-v1` is an **optional core [capability](../../GLOSSARY.md#sessions)**,
not a [plugin extension](../../GLOSSARY.md#protocol-and-plugin-concepts). Missing
library/daemon support keeps the session usable with an explicit focus limitation;
do not claim support silently or infer it from a release number.
Frozen hello remains unchanged apart from optional capability advertisement.

## Attention jump and configurable hints

Global **configured prefix + lowercase `j`** selects the most recent currently
connected `needs_attention` session at keypress time, by newest daemon status-change
time (attention time), with the existing ascending full
[GUID](TERMS.md#guid) tie-breaker. Exclude acknowledged, final, and stale/disconnected
rows; never use an old toast's session as the decision.
It works from selector, cache, placeholder, and empty screens. Close overlays,
explicitly select, acquire only if needed, restore/apply/paint, then submit pending
acknowledgement once unobstructed and ready. Cancel deferred acknowledgement if
the target context changes. No eligible session keeps selection and visibly says
`No sessions need attention`. Uppercase `J` is unknown literal input; pasted
prefix+j is data; s/x behavior is unchanged.

Needs-attention toasts hint the actual configured prefix and lowercase j.
The hint describes a dynamic jump to the most recent attention, not necessarily
the toast's own session. `monitor.show_key_hints` is a boolean setting,
default `true`, controlling **all implicit monitor hints** in toast, chrome,
selector, and empty UI. It changes neither bindings nor application text.
So far only the empty monitor's hint exists to honor it
([#82](https://github.com/jackgeek/climon2/issues/82)).
The [selector](../monitor-selector.md#selector) shows its enabled hints in a
footer below the session list, inside the menu. Disabling hints removes the
footer and its reserved space without changing selector actions.
Automatic startup introduction also requires enabled key hints and no saved
suppression preference. Explicit prefix+? remains available regardless of
either condition. The [dialog reference](../monitor-selector.md#key-introduction-and-help)
owns its content, reversible persistence and modal behavior.
Validate one startup snapshot before terminal changes. If present, `monitor`
must be an object and `show_key_hints` a boolean; missing means default,
wrong type or duplicate known field is an explicit error, unknown/unrelated
settings remain ignored, and there is no live reload.
No new monitor-specific binding settings are introduced.
`monitor.bell` is independently boolean and defaults to true; it is validated,
but toasts and the bell are not implemented.
It emits one [host bell](monitor-host/bell.md) when each eligible toast first
begins drawing, not per animation frame. Validate it in the same startup
snapshot; wrong types/null or duplicate known fields fail before terminal setup.
Disabling it does not hide toasts or suppress monitor-generated OSC 7501 reports.
The [selector reference](../monitor-selector.md#toasts-and-bell) defines the
three-row title toast/four-row hinted toast and clockwise draw/fade/erase order.
The [README configuration example](../../README.md#configuration) documents both
fields, which the current parser validates.

## Required implementation evidence

Automated end-to-end tests must drive the real monitor host and shell/
[TUI](TERMS.md#tui) [fixtures](../../GLOSSARY.md#testing), not just parser/render helpers:

- Normal/alternate storage, internal history/anchoring/eviction, return-to-live
  typing, complete replacement of history, and no native host-history leakage.
- Supported cursor/keypad/mouse modes, translated coordinates, alternate scroll,
  exact paste with modes on/off, fragmented markers/prefixes and stale draining.
- Unicode wide/combining cells, rendition/colors, cursor visibility/shape/blink,
  coherent synchronized updates, and no duplicate query side effects.
- Capped/rapid resize, stable host-relative overlays, retained transition frames,
  applied-versus-painted gates, too-small recovery and responsive global commands.
- Background promotion/restoration, passive absence of history, revoked
  placeholders without a cache-inspection route, disconnected read-only cache/
  exact reconnect overlay, superseded restorations and late authority.
- Host/application focus, disabled/newly enabled reporting, truthful Focus
  outcomes, focus-in not acknowledging, scrollback/toast focus retention.
- Gesture ownership under toast appearance, new toast clicks without takeover,
  target invalidation, modal isolation, and no background/cross-session input.
- Dynamic lowercase j ordering, no eligible target, uppercase/pasted literal
  behavior, actual-prefix hints, hiding every implicit hint, startup defaults/
  validation before terminal changes, and no live reload.
- Startup introduction gated by hints and saved suppression, always-available
  prefix+?, actual-prefix content, persistent/reversible suppression and save
  errors, repeated help, modal isolation and return to the underlying selector.
- Selector borders/search/columns, identity-safe confirmation and cached history,
  nonblocking toast animation/coalescing/dismissal, configured once-per-toast
  bell, and initial/transition-only monitor OSC 7501 advertising/exit clearing.
- Parser/presentation/render failures, exact invalid-UTF-8 binary handling,
  bounded backpressure, and independent selector/exit/other-client progress.

Containable per-target presentation/parser incompatibility makes that target
explicitly unavailable, with app input/viewing off and disconnected retained
cache read-only where safe; other clients/selector/exit remain usable. It is not
evidence of command completion. Rendering, host-output or shared-infrastructure
failure initiates coordinated shutdown, not merely a row error.
The [shutdown contract](../monitor-lifecycle.md#coordinated-shutdown) concurrently
requests all owned instances, drains/releases deliveries, attempts required
atomic [Leave](daemon-client/request-leave.md), closes connections and joins
cleaned-up workers/client threads.
Leave acceptance, closure, final outcome and join are separate evidence.
Prefix+x and catchable signals share this path; Ctrl+C remains application input.
Restore host modes/terminal settings and emit root
[attention](../../GLOSSARY.md#status-and-attention)
[clear](monitor-host/report-clear.md) before diagnostics.
Stalls use bounded whole-process failure, never thread killing/unloading or a
false cleanup claim. Broken output permits only best-effort restoration/clear;
report failure. [Numeric budgets](../monitor-operational-limits.md) and
[native calls](monitor-client/core-v1.md) and the
[acceptance matrix](../monitor-acceptance.md) are agreed; executable
conformance remains future work.
No session-survival promise or containment of a native client crash is added.
Native selection/copy limitations are manually verified where automation cannot
honestly establish host-specific behavior. Documentation checks prove none of
the executable behavior above.
