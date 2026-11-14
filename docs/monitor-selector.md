# Monitor selector and attention presentation

**Agreed planning specification from [Shape the overlaid session selector](https://github.com/jackgeek/climon2/issues/60).**
The [monitor](../GLOSSARY.md#sessions) ships only as the empty walking skeleton of
[#74](https://github.com/jackgeek/climon2/issues/74), whose packaged tests show
that typed prefix+x exits it, with the
[key introduction and help](#key-introduction-and-help) of
[#105](https://github.com/jackgeek/climon2/issues/105) over its empty state.
That dialog closes on a bare Escape when Escape is not the prefix
([#109](https://github.com/jackgeek/climon2/issues/109)); it does not yet open
over a selector. The basic [selector](../GLOSSARY.md#attachment-and-control) of
[#106](https://github.com/jackgeek/climon2/issues/106) is implemented: prefix+s
overlays the captionless double border with status and title columns and a
reversed highlight, Up and Down move it without wrapping, and Escape closes
it, without resizing an application or acquiring anything. Rows list newest
created first, connected rows show Unavailable without `status-v1` and no
status otherwise, and not-connected rows show Starting, Unavailable, This
monitor or Disconnected; a row never seen connected is dropped once its
session is proven gone. Its keys come from the input router of
[#109](https://github.com/jackgeek/climon2/issues/109) (INP-02), which uses no timer (owner decision on #109): a lone Escape is
classified by the byte after it, so a cursor key split across host reads,
even right after its Escape, never closes the selector, and Escape closes it
when the next key arrives, which is then routed as itself (Escape followed by
a key in one read behaves the same). The same holds for the key
introduction. Bytes bound for the selected session, a lone Escape included,
go to it at once. Enter selects the highlighted row
([#108](https://github.com/jackgeek/climon2/issues/108)): a connected row not
known to be final is acquired, a Starting row or an Unavailable row without a
connected client is selected read-only, and on This monitor the selection
stays while the selector explains why on its bottom border
([#110](https://github.com/jackgeek/climon2/issues/110)); disconnected and
final rows wait for [#116](https://github.com/jackgeek/climon2/issues/116). Nothing else this
document specifies is implemented: no search or separator, status order,
key-hint footer, Delete, toasts or bell.
The [throwaway prototype](https://github.com/jackgeek/climon2/pull/72) is a
primary source, not monitor implementation or real daemon/client acceptance.
Use the [terminal contract](protocol/terminal-integration.md) and
[session-status mechanics](session-status.md) for existing authority and attention rules.
The later [lifecycle specification](monitor-lifecycle.md) amends history,
recovery, exit and selector termination/removal. Apart from the empty monitor's
exits and key introduction, all remain unimplemented.

## Key introduction and help

On monitor startup, automatically open a modal key introduction only when
`monitor.show_key_hints` is enabled and the user has not saved **Don't show
again**. Its initial content lists these actions using the actual configured
prefix:

| Binding | Action |
|---|---|
| prefix+x | Exit the monitor |
| prefix+s | Open the session list |
| prefix+? | Show this dialog |

Provide an initially unchecked **Don't show again** checkbox. Persist the
selected preference separately from the user's configuration, per climon2 home
across monitor restarts and releases. Closing without checking it keeps automatic
startup display enabled. Report save failures explicitly; never claim the choice
was saved when it was not. Reopening the dialog shows the saved choice and lets
the user reverse it. This preference does not change `monitor.show_key_hints`.

Prefix+? opens this dialog even when key hints are disabled or automatic display
is suppressed. Repeating it while the dialog is open is a no-op.
Provide an explicit Continue/Close action usable with Enter. Escape closes only
when the configured prefix does not consume it; prefix+x still exits.
The dialog owns application input, retains selection/control and leaves
background clients and terminal models updating. It does not resize the
application. Opening over the selector preserves its search/highlight and
returns to it on close. Showing or closing help does not itself acquire a
session or acknowledge attention. Existing modal focus/viewing and stale-input
rules apply.

## Selector

The selector overlays the [host](../GLOSSARY.md#sessions) without resizing the
[session](../GLOSSARY.md#sessions)'s terminal. Its outer box is double-lined,
without captions. Search occupies the first interior line; a single horizontal
separator joins the edges with U+255F and U+2562. Below it are two borderless
columns: effective status on the left and literal title/command fallback on the
right. There is no details pane/view, working-directory display, control-elsewhere
suffix, current-session marker or buttons. A distinct highlight
identifies the candidate, not a newly selected session.

When `monitor.show_key_hints` is enabled, show a key-hint footer at the bottom
inside the selector, below the session list. Keep it visible while the list
scrolls or search has no matches. Show the available selector actions using
their actual bindings; configured-prefix actions use the configured prefix.
Do not advertise Escape as cancellation when the prefix consumes it.
Clip hints safely to the available display cells without wrapping into the list
or changing application geometry. When hints are disabled, omit the footer
and its reserved space; bindings and selector behavior remain unchanged.

Use the latest nonempty daemon title, otherwise command plus arguments.
No menu [GUIDs](protocol/TERMS.md#guid) are displayed. Future user-assigned names must appear alongside
titles and participate in search when naming ships; this release adds neither
naming nor persistence. Duplicate labels are deliberately not distinguished by
working-directory details.

Rows follow the existing effective-status/time/internal-GUID order.
Opening starts with empty search and highlights the selected session if present,
otherwise the first row. Opening an already-open selector is a no-op.
Search is always focused; Tab/Shift+Tab do nothing. Typing, Backspace and
Left/Right/Home/End edit it. Up/Down and PageUp/PageDown navigate the highlight,
without wrapping, and keep it visible. Enter confirms the highlight from search.
Clicking a row only highlights it; wheel scrolls the list without selecting.
Delete opens the termination/removal confirmation below, not search editing.

Search is case-insensitive ordered-subsequence matching against one string:
displayed status (including failure details), one space, then the displayed
title/command fallback. Hidden command arguments are not searched separately.
All query characters must occur in order across that text; nonmatching
characters are skipped. For example, `bld` matches `Build`, `blr` matches
`Build Replica`, and `Fo Br` matches `foo bar`. Matches may span the status/title
boundary: `rgb` matches a `Running` session titled `Build`. Spaces in the query
participate in matching.
Exact substrings also match. Query characters cannot be reordered, substituted
or absent.
Use `fuzzy-matcher`'s `SkimMatcherV2` for this ordered-subsequence matching, not
trigram similarity or edit-distance typo matching. There is no similarity
cutoff or special short-query rule. Match scores never change the
status/time/internal-GUID order.
Status matching uses the effective display state (for example Disconnected),
not a stale underlying activity status. Search changes retain a still-matching
highlight, otherwise highlight the first match. Clearing search does not resurrect an old
highlight. Bracketed paste inserts search data only: line breaks/tabs become
spaces, other controls are discarded, and pasted commands never execute.
Session-provided text remains inert, with
[display-cell](protocol/TERMS.md#display-cell)-safe clipping.

Status/title/discovery changes apply while the selector is open. Reordering
preserves highlighted identity. Except for confirmed Delete's fallback below,
if an asynchronous update removes that row or its search match, clear the
highlight and require explicit navigation/click
before Enter can select another row. Retained disconnected rows stay listed.
Menu input never reaches the application, acquires control or acknowledges
attention merely through highlighting.

Ordinary Escape cancels without changing selection or held control. Configured
prefix handling runs first. When Ctrl+[ is the prefix, bare Escape waits and
cannot cancel: the developer accepted this limitation, with no alternate
selector-cancel action. Prefix+x remains exit, prefix+j remains dynamic attention
navigation, and prefix+? opens the key introduction/help dialog.
Keep the existing literal-prefix, paste, fragmented-input and stale-input rules.

## Confirmed selection and history

Enter captures the exact highlighted identity. A connected nonfinal session
is explicitly selected/acquired as needed, then restored and painted under
the existing [readiness](../GLOSSARY.md#protocol-and-plugin-concepts) barriers.
Attention acknowledgement still waits for ready, unobstructed presentation.
Control retained by other monitor clients is not released merely by switching.

Enter on a disconnected or definitive final session explicitly opens its retained
history read-only, without acquisition. Chrome says Disconnected, Succeeded or
Failed, including the actual known exit code or signal for failure.
Succeeded is a display label for existing `completed`, not a new daemon status.
Missing cache is explicit; never invent terminal content or a final result.
The agreed [operational limits](monitor-operational-limits.md) shorten already-
captured history to its newest page under pressure, never erase that page
entirely or silently evict live clients. Resource-blocked sessions stay visible.
History inspection cannot accept application input or acknowledge attention.
Disconnected history overlays exactly `Reconnecting...` while passive recovery
is scheduled or active; permanent rejection instead shows a disconnected/error
overlay. On reconnect, remove that overlay and show the non-controller
[placeholder](../GLOSSARY.md#attachment-and-control) until explicit acquisition.
There is no connected revoked-cache inspection route or prefix+h/prefix+r binding.

An unavailable/disconnected target during switching never redirects acquisition
or buffered input to another session. Preserve selection through loss,
completion, revocation and failed acquisition. The monitor remains open through
empty startup, all-unavailable and all-final states, continuing discovery.
There are no selector create/rename actions.

## Delete: terminate and remove

The [lifecycle decision](monitor-lifecycle.md#selector-delete-terminate-and-remove)
explicitly reopens the planning map's earlier selector killing/removal exclusion.
Delete captures the highlighted session's immutable GUID and opens a modal
showing its title and state. Default to **Cancel**; offer **Terminate and remove**,
or **Remove** when already final. Cancel/Escape changes nothing and sends nothing.
Reordering cannot retarget the dialog, and modal input never reaches a session.
Completion while the dialog is open may skip Kill, never change its target.

After confirmation, keep a pending row while unrelated sessions remain usable.
Suppress duplicate pending Kill operations. Use the existing
[Kill](../GLOSSARY.md#attachment-and-control) semantics; concurrent kills join
termination without restarting grace. Remove only after authoritative final
outcome or successful Kill establishes termination, then release deliveries/
handles and complete local instance cleanup/thread join. Final rows skip Kill,
not cleanup. Stalled cleanup follows bounded whole-process fail-stop.

Rejection/failure keeps the row with an error. A lost response is unconfirmed:
recover passively without replaying Kill; later definitive completion may
satisfy confirmed removal intent. Discovery disappearance is not completion.
Dismiss the GUID and discard its retained cache for this run only; discovery
must not re-add it in the same run. Do not delete diagnostics/installations or
persist dismissal across starts.

Removing the selected row selects the row at its old index, or the previous row
if last; none remaining shows the empty state. This fallback changes highlight/
selection only, never acquires control or routes input. Explicit Enter is needed
to acquire another session. Removing another row preserves selected identity.

## Toasts and bell

Use the existing once-per-unviewed-episode/final-outcome notification rules.
The selected terminal continues updating under the modal selector, while
[viewing](../GLOSSARY.md#status-and-attention) and application keyboard focus
remain gated. Nonmodal toasts retain the existing viewing/gesture semantics.
Highlighting, opening/cancelling, dismissing a toast and host focus do not
acknowledge attention.

A toast appears bottom-right outside selector hit targets, in a stable lane.
Its yellow border draws clockwise from the top-left across top/right/bottom/left;
the title appears after the border completes. Main content is just the session
label. Width is at most 40% of host width, with one-line display-cell-safe
ellipsis on overflow. Height is three rows including border, or four rows when
`monitor.show_key_hints` adds a separate configured-prefix+j hint row.
The hint describes dynamic newest attention, not the toast's fixed session.

On disappearance, title and optional hint fade to the toast background,
then the border erases in the same clockwise order. Clicking starts dismissal
without selection, acquisition or acknowledgement. During early dismissal,
erase only the border already drawn. Distinct eligible episodes queue and
repeated requests within a pending episode do not duplicate notifications.
Animation never blocks session switching, selector input, resize or exit.

Accepted prototype timings are 400 ms drawing, 2 seconds title hold,
300 ms text fade, then 400 ms erasure. These are visual trials;
final operational/rendering/queue budgets are specified in
[operational limits](monitor-operational-limits.md#ui-sampling-and-responsiveness).
Too-small screens remain explicit, retain clients/control/models and global
commands, and never send invalid application geometry or buffer input.

Planned `monitor.bell` is boolean, default true. It controls one
[host bell](protocol/monitor-host/bell.md) when each toast first begins drawing,
including initial and queued presentations, not every frame, hint change,
dismissal or coalesced request. Missing file/object/field uses the default.
Read one [climon2-home](../GLOSSARY.md#installation)
[JSONC](protocol/TERMS.md#jsonc) snapshot before terminal setup; wrong types/null,
duplicate known fields or unreadable/malformed input fail explicitly.
Unknown/unrelated settings are ignored. There is no live reload.
Disabling bell changes neither visual toasts nor host program-status reports.
Host settings determine whether a bell is audible.

## Monitor-to-host attention

The monitor generates its own [OSC](protocol/TERMS.md#osc) 7501
[reports](protocol/README.md#monitor-to-host-status-and-bell),
not raw session-output forwarding. Advertise pending attention while any
currently connected session reports `needs_attention`, regardless of toast
visibility/dismissal. Exclude acknowledged, disconnected and final reports.
Use root `blocked`/`kind=question` while pending, `working` while monitoring
without pending attention, and root `clear` on orderly exit.
Report initial state and changes only; no heartbeat or capability query.
These reports neither acknowledge sessions nor amend the enclosing daemon
[FSM](protocol/TERMS.md#fsm).

## Automated acceptance obligations

Later implementation must test real monitor input/terminal behavior, not just
the prototype's synthetic state: exact borders/columns and search cursor,
bottom key hints enabled/disabled (including reclaimed list space, scrolling,
empty search results, configured bindings and narrow-screen clipping),
literal labels and Unicode clipping, exact and fuzzy filtering (including
Unicode, short queries and unchanged ordering), editing/paste isolation,
identity-preserving reorder/removal, explicit confirmation and switch races,
readiness-gated acknowledgement, disconnected/final read-only history,
truthful result/no-cache display, resize/too-small/global progress,
toast ordering/fade/dismissal/coalescing, once-per-presentation bell and startup
configuration validation/snapshot, and transition-only OSC reports/cleanup.
Include default-Cancel immutable-GUID Delete, completion/reordering races,
duplicate suppression, rejection/lost response without Kill replay, definitive
termination and local cleanup/join before dismissal, no same-run rediscovery,
non-acquiring fallback, and prohibited connected revoked-cache inspection.
Include startup help with hints enabled/disabled and suppression saved/unsaved,
the three actual-prefix bindings, reversible suppression across restarts and
releases, explicit save errors, and prefix+? despite either suppression.
Cover repeated help, Enter/Escape/prefix handling, underlying selector
search/highlight preservation, background progress, and modal input isolation
without implicit acquisition or attention acknowledgement.
The agreed [acceptance matrix](monitor-acceptance.md) assigns final scenarios
and evidence layers for #62; executable monitor conformance remains future work.
