# Monitor acceptance criteria

**Status: developer-confirmed specification for
[Define automated monitor acceptance criteria](https://github.com/jackgeek/climon2/issues/62),
not implementation or executed acceptance evidence.** The
[monitor](../GLOSSARY.md#sessions) exists only as the empty walking skeleton of
[#74](https://github.com/jackgeek/climon2/issues/74); packaged
[client libraries](../GLOSSARY.md#sessions) and new daemon wire remain
unimplemented. Headless creation is implemented; its HDL-01 Contract cases and
its Monitor-layer cases through the packaged CLI are mapped in the daemon and
packaged-test READMEs, while the case that needs a running monitor waits for it.
Resolving this specification does not pass a
[release gate](../GLOSSARY.md#testing).

This document specifies evidence for the agreed thread-based Ratatui monitor.
Read the [domain glossary](../GLOSSARY.md), [technical terms](protocol/TERMS.md)
and [protocol reference](protocol/README.md) together with it.
The owning specifications below supply exact values and behavior; this matrix
does not introduce another protocol, compatibility policy or configuration.
Later lifecycle amendments supersede earlier startup reranking and
connected-revoked history proposals.

## Evidence policy

Every scenario has a stable ID, source links, setup/action, observable result
and required evidence layers. Source links in each section apply to every row
in its table. Scenarios may share one test when all their outcomes are asserted;
splitting a scenario preserves its ID through named cases. Tests must fail when
the promised behavior is absent, not merely check constants or helper output.

| Layer | Required meaning |
|---|---|
| Contract | Deterministic behavior through the relevant public boundary, including daemon transitions, numerical boundaries and rejection. Pure state-machine tests supplement integration. |
| Library | Actual packaged production libraries loaded through the published [C ABI](protocol/TERMS.md#c-abi), not direct Rust calls, static linking or a mock function table. |
| Monitor | The actual packaged Ratatui monitor, reached through its public command and driven through a [PTY](protocol/TERMS.md#pseudo-terminal-pty), with real libraries and [session daemons](../GLOSSARY.md#sessions) for normal paths. |
| Fault | Clearly labelled adversarial peers/libraries or narrowly scoped test-only hooks for otherwise impractical malformed input, timing, exhaustion, panic or native failure. |
| Measure | Reproducible resource, startup and sustained-display measurements tied to the tested artifacts and declared environment. |
| Docs | Existing documentation checks plus review of schema, examples, reachability and implemented/planned status. This proves no runtime behavior. |
| Host manual | Only host-native selection/copy limitations that cannot honestly be automated. Record the host, gesture, observed result and limitation separately. |

Multiple layers in a row are cumulative. A fault substitute cannot establish
successful production-library loading, compatibility or real rendering.
Use real-artifact cases as well as adversarial cases. Test hooks must not ship
in release artifacts. Native crash/abort cases run the monitor in a disposable
process: the monitor and every hosted client may fail together. Never assert
process-isolated client survival or invent an ordinary-session survival promise.
Installed libraries are trusted native code, not sandboxed by identity checks.

The terminal oracle combines expected visible cells/history/cursor/modes,
captured application input and host-output side effects as applicable.
Fixture expectations must be independently specified; comparing the monitor
with another invocation of its own renderer is insufficient. Do not require
byte-for-byte equality of entire host escape streams when several correct
paint sequences exist. Check exact bytes where the contract requires them.
Use deterministic clocks, controlled scheduling and bounded waits for races;
real-load latency measurements remain separate from simulated time.

Focused deterministic and packaged-monitor scenarios run for relevant changes.
Before release, require the complete supported-architecture conformance matrix,
native-fault evidence and operational measurements for the exact candidate
artifacts. Dedicated heavy jobs are allowed; an older build or informal demo
cannot discharge the gate. Qualify actual runtime behavior on x86-64 GNU/Linux
(Ubuntu 24.04) and on both supported macOS arm64 and x86_64 architectures; the
[Linux decision](https://github.com/jackgeek/climon2/issues/125) added Linux.
Cross-compilation alone proves no runtime support. Windows monitor support is
not added by this specification.

## Entry and discovery

Sources: [planning map and command spelling](https://github.com/jackgeek/climon2/issues/57),
[ownership](https://github.com/jackgeek/climon2/issues/58),
[startup and discovery](protocol/README.md#startup-and-control),
[status ordering](session-status.md#reporting-ordering-and-initial-selection),
[discovery budgets](monitor-operational-limits.md#discovery-loading-and-start-scheduling),
and [selection lifetime](monitor-lifecycle.md#monitor-and-selection-lifetime).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| ENT-01 | Start `climon2 monitor` and `climon2 mon` from an installed release with equivalent settings and sessions. | Both open the same real monitor behavior; the launcher/install route is exercised, not a renderer fixture. | Monitor |
| DSC-01 | Start with sessions having distinct creation times, effective statuses, change times and tied values; delay passive metadata. | Starts are bounded newest-created-first with ascending full [session ID](../GLOSSARY.md#protocol-and-plugin-concepts) ties. Initial selection is the first eligible status/time row after bounded collection, not newest-created or first-connected; uncertainty is explicit. | Contract, Monitor |
| DSC-02 | Initially stall newer sessions or one release, then let work progress. | Older sessions/other releases get fair first-start progress; selector and exit remain responsive. Every live session is scheduled, never silently capped or omitted. | Monitor, Fault |
| DSC-03 | Start empty, add sessions continuously while the selector/placeholder/live display is active, and request discovery during a pass. | Immediate initial discovery; subsequent pass starts one second after completion, without overlap. New sessions get passive clients automatically; starting/retained clients are reused, and selection does not move. | Contract, Monitor |
| DSC-04 | Supply malformed/oversized metadata, mismatched hello IDs, slow/dead probes, inaccessible discovery and uncertain disappearance. | Validate metadata and the frozen two-second hello deadline with bounded probes. Report incomplete discovery/availability truthfully; do not delete metadata, label uncertainty as death, or tear down established clients on discovery evidence alone. | Contract, Monitor, Fault |
| DSC-05 | Reorder rows or lose/revoke/fail/complete the pinned selected target before or after [readiness](../GLOSSARY.md#protocol-and-plugin-concepts); make every row final/unavailable. | Preserve selected/highlighted identity, never acquire a replacement or redirect input automatically. Empty/all-final/all-unavailable monitor stays open and continues discovery. | Monitor, Fault |
| DSC-06 | Discover nested/controlled-elsewhere sessions in independent resolved homes and exercise CLI list/attach/kill discovery after shared extraction. | Include every eligible live session in the correct home; shared discovery returns structured pass outcomes without owning caller scheduling/presentation. Existing CLI discovery behavior remains unchanged. | Contract, Monitor |

## Ownership and control

Sources: [startup/control and retained clients](protocol/README.md#startup-and-control),
[retained control](protocol/README.md#retained-control),
[membership and succession](protocol/daemon-client/core-v1.md#membership-and-succession),
and [coherent readiness](protocol/terminal-integration.md#host-canvas-geometry-and-coherent-readiness).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| CTL-01 | Discover sessions already controlled elsewhere and a detached session; start background clients and open/browse/cancel menus. | Passive registration never briefly acquires control, never acquires a detached session by registration alone, and is not an [attachment](../GLOSSARY.md#attachment-and-control). Core metadata flows before control. | Contract, Library, Monitor |
| CTL-02 | Explicitly select/take control, switch away and back, and let another client take control. | Last accepted acquisition wins. Switching retains clients/connections and background control. Revoked selection shows the [placeholder](../GLOSSARY.md#attachment-and-control), not current cached output or live core mirroring; no automatic retake. | Library, Monitor |
| CTL-03 | Update title/status/progress/notifications before control, while background, under menus and after revocation. | Required supported core reports remain ordered and independent of screen access. Unsupported/unset/empty/stale values stay distinct; title fallback remains usable without optional support. No plugin endpoint is required. | Contract, Library, Monitor |
| CTL-04 | Depart a controller with remaining attachments and passive registrations; race promotion with Leave and committed Kill. | Most-recent controlling attachment has priority, then newest accepted passive registration. Promotion updates background models without selection/input/resize/acknowledgement. Atomic Leave removes membership before succession; termination overrides succession; final-participant leave policy remains applicable. | Contract, Library, Monitor, Fault |
| CTL-05 | Send keys and resize while selected, background, revoked, restoring or unavailable; inject stale authority and superseded restoration. | Only the selected valid controller gets application input/monitor resize. Current complete applied [restoration](../GLOSSARY.md#restoration-and-presentation) permits Ready/resize progress; selected input additionally waits for a coherent usable painted frame. Late authority never re-enables input. | Contract, Library, Monitor, Fault |

## Packaged library qualification

Sources: [embedding and lifetime](protocol/library-embedding.md),
[native declarations and compatibility](protocol/monitor-client/core-v1.md),
and [runtime evidence](protocol/monitor-client/core-v1.md#required-runtime-evidence).

Each supported architecture requires several simultaneous instances sharing
one packaged release and simultaneous independently packaged different-release
libraries. Distinct private-dependency behavior must expose accidental
cross-binding. Qualify each advertised supported revision/capability combination
and the specified rejection surface. Historical releases without this interface
are unsupported cases, not invented positive compatibility evidence.

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| LIB-01 | Install several release directories; alter `current`, working directory and ambient search paths; exercise monitor and standalone executable lookup. | Absolute exact-session-release library lookup and executable's same-release sibling lookup prevail. Eager local symbol resolution and release-relative private dependencies avoid cross-binding; same-release instances share one load. | Library, Monitor |
| LIB-02 | Supply missing library/dependency/symbol, wrong release, unsupported revision, malformed description or incompatible architecture. | Explicit loader/identity/interface errors before create/connection; no substituted release or fabricated session completion. Opened handles, including later-rejected ones, stay loaded until process exit. | Library, Monitor, Fault |
| LIB-03 | Run instances with independent homes, native paths, settings, diagnostics and resources; mutate/free accepted caller startup buffers. | Creation copies validated inputs atomically and returns all three handles or none. Native paths survive losslessly; no instance mutates process-global arguments/environment/cwd/signals/standard streams or another instance's state. | Library, Fault |
| LIB-04 | Submit accepted/rejected buffers; retain batches/diagnostics, release through the owning library, race descriptor polling/closure and handle uses. | Accepted data is copied before return; rejected data is not retained. Immutable deliveries/diagnostics retain documented lifetimes; allocator pairing is correct. Borrowed descriptor ownership never transfers; busy destruction fails with live objects/handles. | Library, Fault |
| LIB-05 | Stall one instance/release, fail partial startup, trigger contained panics at exported boundaries and complete normal cleanup. | Other instances progress where safe; bounded submit/drain/release perform no daemon I/O or waits/joins. Owner-thread create/run/cleanup, worker completion, handle release and actual thread join remain distinct. No unwinding crosses the ABI. | Library, Monitor, Fault |
| LIB-06 | Crash/abort in loading or running a library; separately stall a loader/worker/client beyond cleanup budget. | Disposable monitor process reports whole-process failure exposure. Bounded fail-stop for stalls never kills one thread, unloads executing code or reports false joins/detach. Ordinary session survival is not assumed. | Library, Monitor, Fault |

Memory/lifetime evidence must exercise the public interfaces, including release
and destruction sequencing; a successful screenshot or absence of a crash is
not proof of safety. Use applicable allocator/ownership diagnostics and
instrumented fault cases, clearly identifying instrumentation and its limits.
Invalid native addresses and wrong-library handles are not made safe by length
validation and must not be advertised as recoverable input.

## Communication and compatibility

Sources: [frozen hello](protocol/daemon-client/hello-request.md),
[daemon wire revision 1](protocol/daemon-client/core-v1.md),
[in-process communication](protocol/README.md#shared-in-process-communication-contract),
and [native revision 1](protocol/monitor-client/core-v1.md).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| COM-01 | Exchange published hello/wire examples with compatible and incompatible peers; test additive fields/capabilities and required/optional support. | Frozen hello and agreed wire negotiation/intersection/rejection hold; support is never inferred from release identity. Ordinary attach/input/resize/escape/detach/Kill parity is retained. | Contract, Library, Monitor, Fault |
| COM-02 | Fragment/coalesce I/O; truncate frames; send wrong direction/type/fields, invalid JSON/text/IDs, unknown native tags or nonzero reserved fields. | Apply each boundary's exact framing, types and rejection/connection effects. Native active-member validation ignores padding/inactive storage; it is not serialized JSON or a wire frame and has no unknown core-tag skipping. | Contract, Library, Fault |
| COM-03 | Exercise every published request/result/event, local acceptance/rejection, queue-full, cancellation, timeout and lost daemon responses. | Rejection consumes no public ID, retains no arguments and produces no later response. Acceptance is not daemon effect. Public-to-wire correlation, ordering and exactly stated outcomes remain truthful; indeterminate work is never replayed. | Contract, Library, Monitor, Fault |
| COM-04 | Transfer arbitrary binary Input/Display including literal `00 ff 41`, embedded [NUL](protocol/TERMS.md#nul), invalid [UTF-8](protocol/TERMS.md#unicode-and-utf-8), and fragmented payloads. | Binary bytes survive unchanged at their boundaries. Text/native paths follow their own validation. Invalid application UTF-8 alone is not a protocol error; no implicit C-string truncation. | Contract, Library, Monitor |
| COM-05 | Race grants, restoration begin/chunks/end, geometry, metadata and operation results; reconnect with old deliveries in flight. | Causal ordering and immutable generation/period/restoration identities hold. Initial metadata is atomic and updates gap-free; old work never crosses a fresh attempt. Ready/applied/painted and Focus-state/forwarding outcomes stay distinct. | Contract, Library, Monitor, Fault |
| COM-06 | Deliver exit zero, arbitrary nonzero exit, signal, partial Kill result, known-final result, inconsistent repeats and disconnect without final evidence. | Exit differs from signal; status and result agree. Closure never invents completion, successful Leave or cleanup. Final result appears once, not as a duplicate status transition; known final evidence survives disconnect. | Contract, Library, Monitor, Fault |

## Actual terminal and input integration

Sources: [terminal integration](protocol/terminal-integration.md),
[engine qualification and limitations](adr/0007-daemon-owned-virtual-terminal.md),
and [shared input configuration](https://github.com/jackgeek/climon2/issues/64).

Every terminal scenario runs through the real monitor, including continuous
output during overlay interaction. Isolated parser/render snapshots are
supplementary. Assert application observations as well as the painted host.

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| TRM-01 | Emit normal/1049 alternate screens, wide/combining characters, colors/rendition, cursor positions/visibility/shape/blink and synchronized updates. | Qualified terminal fidelity survives monitor painting; cursor stays out of chrome/covered or unavailable cells. Unsupported modes, clipboard/hyperlinks and restoration omissions are not falsely advertised as supported. | Monitor |
| TRM-02 | Send session queries, host replies, title/control-looking text and invalid application bytes. | Daemon is the sole session-query responder; monitor does not duplicate replies or forward session bytes directly to the host. Host replies bypass shortcuts/application input. Titles are inert labels, never host-title instructions. | Contract, Monitor |
| TRM-03 | Use host sizes 80x24, 200x60, boundary geometry and a larger-than-supported host; rapidly resize under selector/help/toasts and restoration. | One permanent top row; desired app area is width by height minus one. Canonical monitor bytes use effective session geometry, with blank local margins. Overlay layout is host-relative and never itself resizes the app; only actual host resize targets selected control. | Monitor |
| TRM-04 | Delay/supersede restoration and resize results, shrink too small and recover while output continues. | Retain clipped/padded coherent transition frame without flashing partial models; transactionally replace all history/model, never union/deduplicate. Only unsent resize intents coalesce. `Terminal too small` disables viewing/input/invalid resize without buffering; global actions and clients keep progressing. | Monitor, Fault |
| TRM-05 | Scroll normal history while output continues, type from an anchored viewport, evict history and return from alternate screen. | Normal viewport stays anchored; valid typing returns live and forwards. Evicted anchors move to oldest retained with explicit truncation. Alternate wheel never exposes normal history; capture/alternate-scroll modes determine app events. No monitor output leaks into native host history. | Monitor |
| TRM-06 | Change short/overflowing/empty titles with wide/combining text while chrome status/control/error changes. | Latest nonempty title or command/arguments fallback is literal. Only overflowing label text marquees, with cell-safe offsets and restart on label changes; short labels/status/control/error stay stationary and the application canvas never marquees. | Monitor |
| INP-01 | Use default/custom prefix and takeover settings, null/invalid configuration, literal double prefix, unknown suffix, prefix+d, uppercase J and a lone prefix. | Shared settings/defaults apply; null monitor prefix fails before terminal changes, null takeover is allowed. Fixed s/x/j/? actions work; double prefix emits one literal; unknown suffix emits both without reinterpreting takeover; lone prefix waits. d/h/r add no monitor action. | Contract, Monitor |
| INP-02 | Fragment/coalesce prefix, ordinary keys and mode-dependent cursor/keypad encodings; use Ctrl+[ prefix and identical Escape/Alt encodings. | Routing is independent of read boundaries and honors supported app modes. Subsequent bytes use the current screen/state. Identical encodings receive identical treatment; accepted Escape/Alt ambiguity is documented, not guessed away. | Monitor |
| INP-03 | Paste exact binary payload with app bracketed-paste on/off, prefix commands inside it, a pending prefix, and fragmented markers. | Preserve payload; app markers appear only when app mode is on. Paste never executes shortcuts/completes pending commands. Pending prefix becomes one literal only for a currently accepting target; paste end clears it. Unbracketed paste remains indistinguishable from typing. | Contract, Monitor |
| INP-04 | Start paste/prefix/gesture, then open/close modal UI, switch target, revoke, restore or disconnect; later regain readiness. | Invalidation clears pending prefix and drains stale paste/gesture remainder to its end without cross-session/generation forwarding or delayed buffering. Toast appearance alone does not retarget an existing gesture/paste. | Monitor, Fault |
| INP-05 | Send mouse/drag/wheel events to app, chrome, selector and toast; make a toast appear during app drag. | Hit-testing translates app coordinates below chrome. Initial gesture owns its target; new toast clicks only dismiss. Modal/chrome input stays local; takeover key acts only on selected revoked target and is consumed, otherwise ordinary ready input. | Monitor |
| FOC-01 | Report host focus loss/in; scroll history, open menus, cover/uncover with toasts and toggle application focus-reporting mode. | Viewing and app keyboard focus follow separate rules. Toasts/history retain ready keys; menus gate them. No visibility/focus-in/toast disappearance auto-acknowledges. Desired optional Focus emits no duplicates, newly enabled reporting emits current known state, and new readiness synchronizes latest intent without replay. | Contract, Library, Monitor |
| FOC-02 | Omit host focus reporting or optional library/daemon `focus-v1`; use host-native capture-bypass copy. | Session remains usable with explicit best-effort focus limitations. Record visible-only native copy and host-specific bypass limitations; no monitor clipboard/multipage selection promise. | Monitor, Host manual |

## Selector, help and monitor notifications

Sources: [selector and help](monitor-selector.md),
[terminal attention jump](protocol/terminal-integration.md#attention-jump-and-configurable-hints),
[notification identities](session-status.md#titles-and-notifications),
and [host reports/bell](protocol/README.md#monitor-to-host-status-and-bell).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| SEL-01 | Open/reopen selector with live output; use narrow/large hosts and scrolling/empty matches. | Captionless double border, exact separator joins, focused search, status/title columns and distinct highlight render without app resize. No IDs/details pane/current marker/buttons. Enabled bottom hints remain visible; disabled hints reclaim space; Unicode clipping is cell-safe. | Monitor |
| SEL-02 | Search exact/subsequence/Unicode/short/spaced/status-crossing text; navigate/edit/paste/click/wheel/Tab and update titles/status asynchronously. | Specified ordered-subsequence matcher, no typo/trigram cutoff or score reordering. Search uses displayed status/label only. Paste normalization stays local; navigation does not wrap. Reordering preserves identity; async removal/nonmatch clears highlight and requires explicit navigation before Enter. | Contract, Monitor |
| SEL-03 | Enter/cancel during reorder, loss or takeover; use a prefix that consumes Escape. | Enter pins exact highlighted identity; no misdirected acquisition/input. Cancellation keeps control/selection where available; consumed Escape cannot cancel. Confirmation waits for applied/painted readiness before explicit acknowledgement. No create/rename actions. | Monitor, Fault |
| HLP-01 | Vary hints/saved suppression, reopen prefix+?, reverse checkbox and restart across releases/homes; cause saving errors. | Startup introduction appears only when permitted; actual prefix+x/s/? content and initially unchecked choice are correct. Suppression is separate per-home preference, persistent/reversible; failures are explicit. Explicit help works regardless of hints/suppression and repeated help is a no-op. | Monitor, Fault |
| HLP-02 | Open help over selector; Enter/Escape/prefix while clients output; close or exit. | Modal input isolation, no app resize/acquire/acknowledgement, retained control and background progress. Return restores underlying selector search/highlight; prefix-consuming Escape follows its limitation and prefix+x still exits. | Monitor |
| NOT-01 | Request attention repeatedly, re-arm after acknowledgement, reconnect, open menus after suppressed pending attention, and report final while viewed/unviewed. | Once-per-episode/final-outcome deduplication persists for the monitor run. New pending attention while already viewed is not auto-acknowledged. Menu opens make never-toasted pending attention eligible immediately; a viewed final result is suppressed forever, not resurrected later. Other monitors' viewing never suppresses this one's toast. | Contract, Monitor |
| NOT-02 | Queue distinct episodes; redraw/resize/dismiss mid-animation and change hints/bell. | Stable bottom-right yellow toast lane, width at most 40%, title-only content, three/four-row shape and safe ellipsis. Clockwise 400 ms draw, 2000 ms hold, 300 ms text fade and 400 ms erase skip obsolete paints without blocking input/exit; once-per-first-draw bell, never per frame/coalesced request. | Monitor, Measure |
| NOT-03 | Invoke lowercase prefix+j from every UI state with changing attention order, tied times and no eligible target. | Choose newest currently connected pending attention by time/full-ID tie, not toast target; exclude stale/final/acknowledged. Close overlays, acquire only as needed and acknowledge once ready/unobstructed; cancel deferred acknowledgement on context change. None leaves selection and says `No sessions need attention`; pasted j/J are literal. | Monitor, Fault |
| NOT-04 | Change aggregate connected pending attention, dismiss toasts, disable bell, then exit. | Initial/transition-only root `app=climon2-monitor` OSC 7501 blocked/question or working, then clear on orderly exit. No heartbeat/query/session text/child records/raw forwarding. Bell visibility/audibility never proves acknowledgement or host interpretation. | Contract, Monitor |
| CFG-01 | Validate missing/default/unknown settings, wrong types/null, duplicate fields, unreadable/malformed JSONC; change the file while running. | One startup snapshot before terminal setup; explicit known-setting errors, ignored unrelated fields and no live reload. Hints hide all implicit hints, not bindings/app text; bell independently defaults true. Documentation preserves Escape/Alt warning. | Contract, Monitor, Docs |

## Daemon activity and attention

Sources: [both state machines](session-status.md#events-and-two-fsms),
[snapshot timing](session-status.md#snapshot-sampling-and-the-inactivity-deadline),
[explicit status and fallback](session-status.md#explicit-program-status-and-fallback),
[signal normalization](protocol/daemon-client/core-v1.md#source-normalization),
and [viewing/acknowledgement](protocol/daemon-client/core-v1.md#viewing-acknowledgement-and-local-notifications).

Each transition table is covered, including stale/late/no-change cases.
Deterministic [FSM](protocol/TERMS.md#fsm) tests alone do not prove daemon-to-monitor
integration: representative signal/input/timer sequences must reach real
daemon reports, monitor ordering and notifications.

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| STA-01 | Drive every activity/attention transition, repeated input, pending/acknowledged input, simultaneous input/timer work and final states. | Two-second user window re-arms; accepted real input clears both attention paths. Acknowledged attention re-arms on new request; pending requests coalesce. Input wins same-turn dispatch, stale deadlines are ignored, inactivity attention publishes atomically without transient idle. Final states reject late activity/input/attention. | Contract, Monitor |
| STA-02 | Sample characters/title, resize, identical/style/cursor-only redraws, screen swaps and initial/uninterrupted silence at controlled deadlines. | Fresh random 500-1500 ms scheduling clamps to a hardcoded 10000 ms monotonic deadline; content is checked before expiry. Resize resets baseline without manufacturing work; title changes count independently. No quiet toggling/zero-delay spin/per-output-byte copying or user-window activity inference. | Contract, Monitor, Measure |
| STA-03 | Send root/child OSC 7501 reports and queries, repeated attention-bearing reports, silent explicit working and accepted input. | Valid root-only detector effects; queries/children do not activate/aggregate activity. Explicit working can stay silent indefinitely; user-timer expiry restores remembered working only in the current explicit scope. done/error is not session completion. Malformed/unsupported reports follow upstream validation. | Contract, Monitor |
| STA-04 | Send error/clear/full/soft reset; change viewport/title/geometry during fallback; overlap user timer, fresh root reports, shell markers and final exit. | Separate 10000 ms fallback quiet deadline restarts/cancels correctly, no agent inference/extra attention on grace expiry. User deadline stays independent. Full/soft reset differences hold; no invented heartbeat when an app exits into a shell without lifecycle signals. | Contract, Monitor |
| STA-05 | Drive OSC 133 A/B/C/D, repeated boundaries, prompt rendering, retained done/error records and shell command results. | Retire detection scope and stale timers; exclude prompt rendering until B/C, preserve accepted input/explicit reports, refresh baselines and do not invent final session exit or attention. | Contract, Monitor |
| STA-06 | Emit OSC 9 text/progress/control, chunked OSC 99 complete/incomplete/auxiliary messages and OSC 777 notify/other commands. | Only recognized complete notifications request attention. Progress/current structured metadata reaches passive/background/revoked clients without core screen mirroring or raw host forwarding. No external actions/full desktop protocol/plugin/progress-UI promise. | Contract, Library, Monitor |
| STA-07 | Change hidden activity/detector states, effective status, episode IDs and UTC times; reconnect and reorder tied rows. | Only effective status changes update stored UTC time; both working states project to running. Atomic metadata/gap-free reports preserve episode/time identity. Exact status ranking/time/full-ID order holds; stale disconnection time is separate and no fabricated idle/clear/final value appears. | Contract, Library, Monitor |
| STA-08 | Explicitly select, focus, close/cancel menu, take over, type/paste, and separately reveal new pending attention by focus-in or toast dismissal. | Acknowledgement requires current controller/period/restoration/readiness/latest viewing; no-pending success is explicit no-change. Deferred intent cancels on target change. Accepted real input clears attention; shortcuts/query replies/Focus are not input. Mere visibility never acknowledges. | Contract, Library, Monitor, Fault |

## Recovery, removal and shutdown

Sources: [lifecycle and exit](monitor-lifecycle.md),
[Delete](monitor-selector.md#delete-terminate-and-remove),
[passive retry](monitor-operational-limits.md#passive-retry),
and [shutdown/error calls](protocol/monitor-client/core-v1.md#errors-terminal-evidence-and-cleanup).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| REC-01 | Lose transport, fail startup/missing library, contain a panic, repair installation, or permanently reject protocol/configuration. | Visible exponential passive retry with configured jitter/cap for safe recoverable failure; permanent rejection/final outcome stops it. Fresh attempts wait for previous objects/handles/workers/thread cleanup and join; never overlap, retake automatically or replay indeterminate work. | Library, Monitor, Fault |
| REC-02 | Inspect disconnected/final cache with full/short/no history; recover while selected or get revoked while connected. | Retained read-only cache says stale/known final/no cache truthfully. Exact `Reconnecting...` only during scheduled/active retry, error overlay for permanent rejection. Reconnect removes overlay but non-controller placeholder remains; connected revoked cache has no inspection route. | Monitor, Fault |
| DEL-01 | Delete highlighted row; cancel, reorder, finish command during modal, press Delete repeatedly and send modal paste/keys. | Default Cancel and immutable identity; no request on cancel/no input leakage. Completed target may skip Kill, never retarget. Confirmed pending Kill is deduplicated; unrelated sessions remain usable. | Monitor, Fault |
| DEL-02 | Reject/lose Kill response, remove discovery metadata, later learn final outcome, and delay delivery release/thread join. | Keep row/error/uncertainty; only passive outcome recovery, never Kill replay. Definitive termination plus local cleanup/join precedes dismissal, including already-final rows. Run-local dismissal prevents rediscovery; fallback at old index/previous/empty never acquires another session or deletes unrelated data. | Library, Monitor, Fault |
| EXT-01 | Exit passive/attached/partially-started/failing clients with full ordinary queues and live deliveries; race promotion/Leave. | One concurrent shutdown path stops input/acquisition/discovery/retry, uses independent shutdown handles, drains/releases and attempts atomic required Leave. Accepted Leave, closure, final client outcome, worker cleanup, handle release and thread join are separately proved. | Library, Monitor, Fault |
| EXT-02 | Repeat prefix+x/signals; lose Leave response, stall workers/loaders/discovery or hold deliveries beyond budget. | Deadline is not restarted; unrelated cleanup progresses. Current unconfirmed required Leave or cleanup means failure. Bounded whole-process fail-stop attempts host restoration/clear and reports affected identities/unconfirmed work, never false join/detach or forced unload/thread kill. | Library, Monitor, Fault |
| EXT-03 | Exit orderly, send catchable SIGINT/SIGTERM/SIGHUP, type Ctrl+C, break host output and cause shared render failure. | Terminal modes/settings and root clear precede final diagnostics on clean exit. Ctrl+C is app input; signals use common cleanup and signal-style success status. Shared failure/cleanup/restore failure uses climon2 error code; broken output is explicit best effort, native crash/uncatchable signal has no cleanup promise. | Monitor, Fault |
| EXT-04 | Finish/fail a selected command or safely clean a local error, then cleanly exit; separately have prior uncertain transport loss. | Monitor does not inherit command exit code. Resolved local errors/old closed-and-cleaned uncertainty do not poison clean exit, nor retroactively become accepted Leave; uncertainty for a possibly active connection closed during this shutdown does fail exit. | Monitor, Fault |
| HDL-01 | Run `c2 exec --headless <command> [args...]`, fail creation, add/remove later attachments and finally complete/Kill. | Creation success only after daemon confirmation, no initial interactive client. Explicit [headless session](../GLOSSARY.md#sessions) survives zero clients throughout lifetime, including later unexpected loss, until command completion/Kill. Existing CLI Kill works; ordinary leave policy remains unchanged, and CLI Kill does not dismiss another monitor's row. | Contract, Monitor |

## Limits and measured qualification

Sources: [operational limits](monitor-operational-limits.md),
[required implementation evidence](monitor-operational-limits.md#required-implementation-evidence),
and [native limits/resources](protocol/monitor-client/core-v1.md#startup-limits-and-resources).

For each configurable/fixed numerical boundary, exercise exactly at and one
over the valid limit, with below-limit progress where meaningful. Include
zero/minimum/maximum, invalid combinations and arithmetic overflow. Inject near
exhaustion rather than waiting for billions of operations; identify the hook
and prove real rejection/progress at the boundary.
For timers/deadlines exercise just before, at and after expiry, stale timer
arms, intervening progress and scheduler lateness without treating simulated
time as a measured operating-system guarantee.

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| LIM-01 | Vary byte/record/delivery/operation/frame/text/parser/assembly/search/toast budgets and configurations, including UTF-8 boundaries and unsupported release limits. | Checked entry validation and explicit rejection before unsafe allocation. Honor peer frame ceilings, effective instance limits and progress reserves; no silent default raising, invented native frame limit or success-shaped fallback. | Contract, Library, Monitor, Fault |
| LIM-02 | Fill queues, drain without releasing, retain objects and trigger ownership-transfer/copy/staging peaks. | Account capacity/reservations as well as length; drain does not reclaim live delivery storage. Protected progress and shutdown remain serviceable; required traffic is not silently dropped, and delivery deadline failure reports undelivered records. | Library, Monitor, Fault |
| LIM-03 | Grow history/current/alternate/combining/auxiliary data across geometries, resize and replace restoration under pressure; include little/no received history. | Allocation-aware parser/model rejects before unsafe growth. Current state and protected newest captured page survive; shorten eligible unselected disconnected/final histories first, selected cache only later. Explicit truncation/staleness, no fabricated page/client eviction; staging peaks are accounted. | Contract, Monitor, Fault |
| LIM-04 | Exhaust aggregate/per-client identity, memory, stacks/workers/descriptors/shared releases and operating-system resources during partial starts/continuous arrivals. | Reserve before admission, count shared resources once and retain opened releases. Resource-blocked live rows stay visible and retry fairly without busy loops; mandatory identity exhaustion is explicit coordinated failure, never silent omission. | Library, Monitor, Fault |
| LIM-05 | Exhaust request/generation/delivery and session-wide authority/restoration/episode counters; submit rejected work around exhaustion. | No wrap/reuse or side effects needing a new exhausted ID. Rejection consumes no ID. Safe instance/connection replacement never resets daemon session-wide history; explicit resource failure preserves protected cache/known final evidence. | Contract, Library, Monitor, Fault |
| LIM-06 | Saturate retry/start/notification/discovery work and freeze newer starts/consumers while processing active input/selector/exit. | Configured quanta/deadlines/jitter/caps bound work and backlog; indivisible work is bounded too. Fair independent progress, non-overlapping retries/discovery and explicit stalled-stream results hold. | Monitor, Fault, Measure |
| PERF-01 | Run the declared workload matrix below with production candidate artifacts and equivalent standalone clients. | Report total and attributable costs, admission outcomes, sustained display/backlog and latency distributions; demonstrate configured allocation limits and disclose limitations, not presumed savings from fewer processes. | Measure |

Run the #69 evidence points: 1, 10 and 100 live sessions; one release and ten
mixed releases where applicable; 80x24 and 200x60 plus 500x200 boundary geometry.
Cover passive idle, controlling background output, cold/warm loading,
restoration/switching, selected sustained output, reconnect storms, continuous
arrivals and stalled newer starts. Record snapshot comparison/allocation,
per-session/aggregate history/queues, threads and retained shared libraries.
Compare standalone clients doing equivalent work, not fewer active sessions.

Declare OS/architecture/hardware, configuration, workload/rate/duration and
sampling method. Report processor use, total process resident memory and
attributable components, daemon memory separately, cold/warm startup/readiness,
input/selector/attention/exit-initiation latency distributions and backlogs.
Tracked allocations/reservations are not a whole-process resident-memory cap.
If defaults cannot admit a scenario, record it and repeat with disclosed
sufficient configuration where supported; counts are evidence points, not caps
or arbitrary-hardware promises.

Preserve #69's supported-load targets of at most 100 ms at the 99th percentile
for input dispatch, selector/global actions and attention presentation.
A measured stall of 250 ms or more requires diagnosis. Missed targets require
diagnosis and an explicitly agreed disposition before release, not silent
success. These are declared-environment targets, not hard real-time guarantees.
Any additional processor/resident-memory regression threshold needs measurements
and separate agreement; this specification invents none.

## Documentation and evidence records

Sources: [protocol reference](protocol/README.md),
[native header](protocol/monitor-client/client-v1.h),
[examples](protocol/monitor-client/examples.c),
[declaration and link checks](protocol/monitor-client/check.py),
[runtime artifacts](runtime-artifacts.md) and [crate relationships](crate-relationships.md).

| ID | Setup and action | Required observable result | Layers |
|---|---|---|---|
| DOC-01 | Run existing reference/link/example/layout checks and compare every published operation with its implemented public interface. | Reachable per-message pages and valid links/examples; fields/tags/ownership/ordering/errors/limits match frozen schemas, on both declared native targets. Documentation compilation never substitutes for runtime compatibility. | Docs, Contract |
| DOC-02 | Review shipped artifacts, configuration/help/examples, diagrams and feature status against the candidate implementation and evidence manifest. | Implemented/planned/deferred claims are consistent; runtime/crate diagrams show actual consumption distinctly from plans and retain README links. No unexecuted monitor/library/plugin test is claimed as passing. | Docs |

For each scenario/case record: source revision, test selector, required layers,
artifact identity/provenance and hashes, architecture/environment/configuration,
setup/workload, expected/observed result, command and result, timing/diagnostic
evidence, and any substitute/manual limitation. Use explicit **not implemented**,
**not run**, **passed**, **failed** or **blocked** states. A skipped/unavailable
required fixture/architecture is blocked evidence, not a pass. A failed/blocked
mandatory scenario prevents unqualified conformance; any agreed disposition
must remain visible. Performance dispositions do not waive functional contracts.
Keep test transcripts synthetic and scoped to owned fixtures; production
diagnostics must not log session content.

This document is the specification, not the result manifest. No row is currently
claimed passed. Store execution records with candidate test/qualification
artifacts rather than editing a plan to imply tests ran.

## Existing fixtures and missing capabilities

The following are reusable starting points, not monitor acceptance evidence:

| Existing source | Capability and boundary |
|---|---|
| [Terminal harness](../crates/climon2-test-term/src/terminal.rs), especially modes at lines 73-116 and screen/history/query handling at 119-186 | Native PTY, emulated host, screen/history and mode/query assertions; useful for driving the actual monitor. Character-only screen text does not by itself prove combining-cell/rendition/cursor fidelity. |
| [Isolation](../crates/climon2-test-term/src/isolation.rs), lines 40-110 | Per-test home/environment, owned processes and leak checks. Ordinary cleanup expectations must be adapted to the deliberately surviving headless/detached sessions owned by a test. |
| [TUI fixture tests](../crates/climon2-fixtures/tests/tui.rs), lines 4-59 | Alternate screen, cursor queries, mouse, paste and resize reports through a PTY. They currently test the fixture, not the monitor. |
| [Packaged tests](../crates/climon2-e2e/tests/packaged.rs), lines 10-42 and 68-145 | Real installer/launcher/CLI/daemon/client path and application-observed runtime/resize/log-privacy checks. This route currently has no public client-library or monitor qualification. |
| [Native documentation checks](protocol/monitor-client/check.py), lines 14-94 | C declaration/example compilation for both target layouts, reference reachability and relative links. No linked production library or runtime call evidence. |

Genuinely missing release evidence includes the actual monitor and packaged
public-library test path; independently packaged mixed-release/private-dependency
fixtures; ABI lifetime/descriptor/allocator and fault/stall instrumentation;
monitor-integrated continuous-output/overlay/readiness/recovery/routing scenarios;
rich cell/cursor/host-output oracles where the current text harness is insufficient;
deterministic clock/scheduling and numerical exhaustion cases; and reproducible
monitor resource/startup/display qualification. Reuse existing fixture capabilities
where suitable, but name necessary extensions before treating scenarios as runnable.
Throwaway selector/rendering prototypes are design sources only.

## Deferred scope

No current gate requires plugin implementation/configuration/interfaces/examples
or plugin test endpoints. Core title/status/activity/attention and monitor
notifications are required. Progress metadata is required where specified,
not a progress-bar interface. Remote forwarding/security, other-platform monitor
support, user naming/renaming, persisted final archives, old-version cleanup and
selector creation remain deferred. Confirmed Delete and explicit CLI headless
creation are in scope. No new session-survival or native-crash isolation guarantee
is introduced.
