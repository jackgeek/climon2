# Monitor lifecycle, exit and failure

**Status: agreed specification, not implemented behavior.** This resolves
[Define monitor exit and failure behavior](https://github.com/jackgeek/climon2/issues/61)
within [the monitor planning map](https://github.com/jackgeek/climon2/issues/57).
It amends the earlier selector/history and session-management scope decisions.
Numerical deadlines, retry intervals and retention budgets are specified in
[operational limits](monitor-operational-limits.md); complete native
declarations are agreed in [revision 1](protocol/monitor-client/core-v1.md).

Read the [domain glossary](../GLOSSARY.md), [status mechanics](session-status.md),
[selector](monitor-selector.md), [terminal contract](protocol/terminal-integration.md),
[embedding](protocol/library-embedding.md) and
[daemon wire](protocol/daemon-client/core-v1.md) together with this document.
No monitor, new wire, native interface or headless implementation is delivered.

## Monitor and selection lifetime

The monitor stays open when initial discovery finds no sessions, when all clients
are unavailable, and when every known command finishes. It shows an explicit
empty state or retained session rows and continues discovery until exit.

Connection loss, completion, control revocation and failed acquisition preserve
selected-session identity. Reordering preserves selection and selector highlight
identity. Never automatically acquire a different session or redirect queued
input because the chosen one fails. Partial eager-startup failure leaves the
affected row unavailable; successful clients remain usable.

Keep known completed/failed outcomes after closure. Otherwise show disconnected
with stale cached metadata, not a fabricated final outcome. Keep session identity
and metadata for this monitor run unless explicitly removed. Terminal models and
local scrollback are bounded by the operational-limits decision. Memory pressure
shortens captured history to its newest page, never completely erases that page
or silently evicts a live client. Sessions with no received cache say so
truthfully. Resource-blocked live rows remain visible and retry admission;
failure to retain required identities is an explicit coordinated monitor failure.
No persisted startup archive is implied.

## History and passive recovery

Disconnected sessions expose their retained terminal/history read-only. Overlay
exactly `Reconnecting...` while an automatic passive attempt is scheduled or
running. If permanent rejection stops recovery, show an explicit disconnected/
error overlay instead. Missing cache shows a truthful placeholder. Completed and
failed sessions allow retained history without retry.

A connected session controlled elsewhere shows the normal placeholder, not its
revoked cached terminal. There is no separate revoked-cache inspection route.
This removes the earlier planned requirement for such a route. No prefix+h,
prefix+r or additional history/retry binding is added.

Selector Enter retains its agreed behavior: acquire a connected nonfinal session,
or explicitly open disconnected/final retained history read-only. Read-only
inspection sends no application input, acquisition or acknowledgement.

Use automatic exponential backoff for uncertain transport loss, recoverable
startup failures (including a missing library), and safely contained worker
panics. A repaired installation can recover automatically. Confirmed permanent
protocol/interface rejection and invalid configuration remain explicit errors
and stop automatic retries; definitive completion also stops retry. Numerical
initial delay, cap and any jitter belong to operational limits.

Each fresh client attempt waits for the preceding instance's deliveries, handles,
workers and client thread to finish cleanup and join. Never overlap replacement
instances while old code may still run. Cleanup that cannot be proven complete
uses the fail-stop policy below, not another retry. Every successfully opened
library handle remains loaded until process exit, including later-rejected ones.

Recovery is passive: refresh metadata and reconcile retained state, but never
automatically request takeover or replay indeterminate requests/input. Remove the
reconnection overlay after reconnect; a selected non-controller shows its
connected placeholder until explicit acquisition. Daemon-initiated passive
succession remains valid under the agreed wire rules, not a monitor takeover
request. Control, current restoration and readiness still gate live input.

Stop initiating retry during shutdown or a confirmed pending termination/removal.
Recovery needed to learn an indeterminate Kill's outcome remains passive and
must never replay Kill.

## Selector Delete: terminate and remove

Delete in the session chooser captures the highlighted session's immutable GUID
and opens a modal confirmation showing its title and state. This explicitly
reopens the planning map's earlier exclusion of selector session killing/removal;
session creation in the selector remains out of scope.

Default to **Cancel**. Offer explicit **Terminate and remove**, or **Remove** for
an already-final session. Cancel/Escape makes no request and changes nothing.
Rows reordering never retargets the dialog; dialog input never leaks to a session.
If the captured session becomes final while the dialog is open, confirmation may
skip Kill, but it still concerns the same GUID. Unknown/disconnected is not proof
of termination. Repeated Delete never creates duplicate pending Kill operations.

After confirmation, display termination pending while unrelated sessions remain
usable. Use the existing Kill semantics: termination is committed, succession
does not override it, and concurrent kill requests join the same termination
without restarting grace. A successful Kill confirms actual command exit and
daemon cleanup, not zero command status or completion of client-thread cleanup.

Remove only after an authoritative final outcome or successful Kill establishes
termination, followed by release of deliveries/handles and cleanup/join of the
owned client instance. An already-final row skips Kill but not required local
cleanup. A stalled cleanup uses the process-level fail-stop policy.

On rejection or failure, keep the row and report the error. A lost response is
unconfirmed: keep the row and recover passively without automatically replaying
Kill. Later definitive completion may satisfy the already-confirmed removal
intent. Disappearance from discovery alone is not definitive completion.

Removal dismisses this GUID and discards its retained monitor cache for this
monitor run only. Discovery must not immediately re-add it in the same run.
Do not delete diagnostics, installation files or metadata beyond normal daemon
cleanup, and do not persist a cross-startup dismissal/archive.

When removing the selected row, select the remaining row at its old index, or the
previous row if it was last. If none remains, show the empty state. This fallback
changes highlight/selection only: no acquisition or application input is implied.
The user must explicitly Enter to acquire another session. Removing another row
preserves the selected session's identity.

## Failure ownership

Missing/incompatible libraries, per-client startup/communication failures and
containable worker panics are local to the affected row where ownership and
cleanup can be proven safe. Show an actionable diagnostic; unrelated sessions
remain usable. Distinguish recoverable unavailability from permanent rejection.

Failure of rendering, host output or shared monitor infrastructure is fatal to
interactive operation and initiates coordinated shutdown. A partially started
instance remains owned and accounted for during cleanup; neither a startup error
nor a final event erases that obligation.

Trusted native code may fault or abort the entire monitor and every hosted client.
There is no thread-level native-crash containment, forced library unloading or
universal session-survival promise. Ordinary non-headless sessions still use the
established leave policy and agreed passive succession.

## Coordinated shutdown

Prefix+x and catchable SIGINT/SIGTERM/SIGHUP enter one shutdown path:

1. Stop new application input, acquisition, discovery and retry scheduling.
2. Request shutdown for every owned instance concurrently through its thread-safe,
   nonblocking shutdown control. Include passive, attached, partially started and
   failing instances. One slow instance must not serialize unrelated cleanup.
3. Keep draining outcomes and releasing outstanding deliveries. Each client
   attempts atomic Leave where required, then closes its connection and cleans
   up its workers on its owner thread. Leave removes passive successor
   registration and any attachment, including a promotion racing with shutdown.
4. Finish handle release after concurrent use ends and join every owned client
   thread. Never successfully destroy an instance with outstanding deliveries.
5. Restore the host terminal and emit the specified root attention `clear` before
   final shell diagnostics. Restoration failure must be reported, not described
   as success.

Record separate evidence for daemon-accepted deliberate detach/passive
deregistration, connection closure, final client outcome, worker cleanup and
thread join. A Leave response proves only its stated daemon action. A lost
response stays failed/unconfirmed/indeterminate; neither closure nor a final
event proves successful Leave or local shutdown. Never replay indeterminate work.

Repeated exit requests or catchable signals neither restart the shutdown deadline
nor skip cleanup. Keyboard Ctrl+C remains application input according to the
input contract; it is not a monitor exit binding.

If cleanup stalls beyond the eventual budget, attempt host restoration and root
attention clear, report the affected instances and unconfirmed Leave/cleanup,
then terminate the entire monitor process with failure. Never kill an individual
thread, unload running code or claim a stalled thread joined. Whole-process
termination is the bounded fail-stop boundary, not a clean detach.

When output is broken, restoration/attention clear and diagnostics are
best-effort through available channels. Return failure and never promise the host
was restored. Native crash, abort and uncatchable signals have no cleanup promise.

## Exit status

Explicit prefix+x succeeds only after required current Leave outcomes, local
cleanup/joins and host restoration succeed. Session command failures or safely
resolved session-local client errors do not by themselves poison a later clean
monitor exit. Never inherit the selected command's exit status.

A prior uncertain transport loss remains recorded as unconfirmed; it does not
permanently force exit failure after its connection/registration is gone and
local cleanup is complete. Do not retroactively describe it as accepted detach.
A live or possibly active connection closed during this shutdown without a
confirmed required Leave makes this exit fail.

Monitor-wide fatal error, required Leave failure/uncertainty, incomplete cleanup
or failed host restoration uses the existing climon2 error code. Catchable
signal-initiated exit uses signal-style status after successful cleanup; cleanup
failure instead uses the climon2 error code.

## Future headless creation and existing CLI kill

Reserve future `c2 exec --headless <command> [args...]`. It starts no
interactive client and returns success only after daemon-confirmed successful
session creation. A later command outcome is the session's outcome, not the
already-returned CLI's status. Creation failure must be reported truthfully.

Headless is an explicit durable per-session creation policy: zero clients are
allowed from startup and throughout that session's lifetime, including after the
last client unexpectedly disappears. The session ends on command completion or
explicit Kill. Normal sessions retain their existing leave policy; deliberate
detach retains its established behavior. This narrowly scoped headless lifetime
exception does not make every session persistent or add native-crash isolation.

`c2 kill <session>` already exists. Preserve it and require it to terminate
future headless sessions using the same authoritative Kill semantics. Selector
Delete adds confirmation and run-local dismissal, not a different wire Kill.
CLI kill does not dismiss rows in an independently running monitor.

## Required automated implementation evidence

Later implementation must demonstrate the following with automated tests, not
manual screenshots or assertions about this documentation:

- Empty startup/all-final lifetime, selection identity through reordering/loss/
  acquisition failure, and independent partial eager-startup failures.
- Passive/attached/partially-started shutdown; promotion racing with atomic Leave;
  lost Leave response, closure, final event and join as distinct evidence.
- Concurrent cleanup, outstanding deliveries/handles, worker/thread stalls and
  bounded whole-process failure without false join/detach claims.
- Safe local panic versus whole-monitor native crash/abort exposure, without
  assuming affected ordinary sessions survive.
- Exponential passive recovery after transport/startup/panic failure, permanent
  rejection and final stopping, no overlapping instances, takeover or work replay.
- Disconnected read-only cache with exact reconnecting/error overlay, cache loss,
  recovery to non-controller placeholder and prohibited connected revoked history.
- Terminal modes/termios and root attention clear on orderly exit; broken output,
  shared failure, signals and repeated shutdown with truthful exit outcomes.
- Identity-safe default-Cancel Delete, completion racing with confirmation,
  duplicate suppression, kill rejection/indeterminate outcome, cleanup before
  dismissal, no same-run rediscovery and non-acquiring selection fallback.
- Headless creation acknowledgement and zero-client survival before/after
  attachment loss; command completion and existing CLI kill of headless sessions.

These obligations supplement [final monitor acceptance](https://github.com/jackgeek/climon2/issues/62).
Exact budgets are in [operational limits](monitor-operational-limits.md);
complete native schemas are published in [revision 1](protocol/monitor-client/core-v1.md). No
production test conformance is claimed by this specification.
