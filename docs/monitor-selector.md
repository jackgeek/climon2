# Monitor selector and attention presentation

**Agreed planning specification from [Shape the overlaid session selector](https://github.com/jackgeek/climon2/issues/60).**
The [monitor](../GLOSSARY.md#sessions) is not shipped.
The [throwaway prototype](https://github.com/jackgeek/climon2/pull/72) is a
primary source, not monitor implementation or real daemon/client acceptance.
Use the [terminal contract](protocol/terminal-integration.md) and
[session-status mechanics](session-status.md) for existing authority and attention rules.

## Selector

The selector overlays the [host](../GLOSSARY.md#sessions) without resizing the
[session](../GLOSSARY.md#sessions)'s terminal. Its outer box is double-lined,
without captions. Search occupies the first interior line; a single horizontal
separator joins the edges with U+255F and U+2562. Below it are two borderless
columns: effective status on the left and literal title/command fallback on the
right. There is no details pane/view, working-directory display, control-elsewhere
suffix, current-session marker, buttons or bottom hints. A distinct highlight
identifies the candidate, not a newly selected session.

Use the latest nonempty daemon title, otherwise command plus arguments.
No menu [GUIDs](protocol/TERMS.md#guid) are displayed. Future user-assigned names must appear alongside
titles and participate in search when naming ships; this release adds neither
naming nor persistence. Duplicate labels are deliberately not distinguished by
working-directory details.

Rows follow the existing effective-status/time/internal-GUID order.
Opening starts with empty search and highlights the selected session if present,
otherwise the first row. Opening an already-open selector is a no-op.
Search is always focused; Tab/Shift+Tab do nothing. Typing, Backspace/Delete and
Left/Right/Home/End edit it. Up/Down and PageUp/PageDown navigate the highlight,
without wrapping, and keep it visible. Enter confirms the highlight from search.
Clicking a row only highlights it; wheel scrolls the list without selecting.

Search is case-insensitive against title, command arguments or displayed status,
including failure details. Exact substring matches always qualify. Queries of
at least three characters also qualify when `rust_fuzzy_search` reports trigram
similarity of at least 0.5 against any field. Shorter queries use exact
substrings only. Similarity never changes the status/time/internal-GUID order.
Query length counts Unicode characters before lowercasing.
Status matching uses the effective display state (for example Disconnected),
not a stale underlying activity status. Search changes retain a still-matching
highlight, otherwise highlight the first match. Clearing search does not resurrect an old
highlight. Bracketed paste inserts search data only: line breaks/tabs become
spaces, other controls are discarded, and pasted commands never execute.
Session-provided text remains inert, with
[display-cell](protocol/TERMS.md#display-cell)-safe clipping.

Status/title/discovery changes apply while the selector is open. Reordering
preserves highlighted identity. If an asynchronous update removes that row or
its search match, clear the highlight and require explicit navigation/click
before Enter can select another row. Retained disconnected rows stay listed.
Menu input never reaches the application, acquires control or acknowledges
attention merely through highlighting.

Ordinary Escape cancels without changing selection or held control. Configured
prefix handling runs first. When Ctrl+[ is the prefix, bare Escape waits and
cannot cancel: the developer accepted this limitation, with no alternate Cancel
action. Prefix+x remains exit and prefix+j remains dynamic attention navigation.
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
History inspection cannot accept application input or acknowledge attention.
A reconnect does not automatically turn inspection into a live presentation.

An unavailable/disconnected target during switching never redirects acquisition
or buffered input to another session. Monitor-wide fallback, exit/retry policy,
and a separate route to inspect a connected revoked cache remain with
[lifecycle](https://github.com/jackgeek/climon2/issues/61).
There are no selector create/kill/rename actions.

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
final operational/rendering/queue budgets belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
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
literal labels and Unicode clipping, exact and fuzzy filtering (including
Unicode, short queries and unchanged ordering), editing/paste isolation,
identity-preserving reorder/removal, explicit confirmation and switch races,
readiness-gated acknowledgement, disconnected/final read-only history,
truthful result/no-cache display, resize/too-small/global progress,
toast ordering/fade/dismissal/coalescing, once-per-presentation bell and startup
configuration validation/snapshot, and transition-only OSC reports/cleanup.
Final scenarios remain with
[automated acceptance criteria](https://github.com/jackgeek/climon2/issues/62).
