# Control event

**Status:** agreed ownership behavior; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [client](../../../GLOSSARY.md) reports [grants](../../../GLOSSARY.md) and
[revocations](../../../GLOSSARY.md) to the [monitor](../../../GLOSSARY.md),
identifying the applicable [control period](../../../GLOSSARY.md).
A grant starts [restoration](../../../GLOSSARY.md), not input [readiness](../../../GLOSSARY.md).
Revocation invalidates readiness and pending restoration; show the
[placeholder](../../../GLOSSARY.md), not cached content as current.
There is no connected revoked-cache inspection route, implicit takeover or live
core mirroring. Disconnected cache is explicitly read-only, overlaying exactly
`Reconnecting...` while passive recovery is scheduled or active; permanent
rejection instead shows a disconnected/error overlay. Reconnect removes that
overlay but leaves the non-controller placeholder until explicit acquisition.
No prefix+h/prefix+r binding is added. These later
[lifecycle amendments](../../monitor-lifecycle.md#history-and-passive-recovery)
are agreed, unimplemented.

[Hand-back](../../../GLOSSARY.md) may restore background control without
changing selection. That client resumes screen maintenance but receives no
monitor application input or resize.
Grant supplies effective daemon geometry; canonical monitor display/restoration
uses it even for passive promotion or background hand-back. Apply the complete
replacement model/history without selection or paint, then report Ready.
Selected application input additionally waits for coherent paint. See the
[terminal contract](../terminal-integration.md).
Core title/status reports continue even without control;
[plugin](../../../GLOSSARY.md) implementation is deferred.

[Viewing](../../../GLOSSARY.md#status-and-attention) is separate from retained control.
See [viewing reports](request-viewing.md). A grant or
hand-back alone does not establish a selected, displayed presentation.
Revocation clears viewing. Explicit selection/takeover can acknowledge
[attention](../../../GLOSSARY.md#status-and-attention), but merely being visible must
not automatically acknowledge a new attention request.

The [ownership decision](https://github.com/jackgeek/climon2/issues/58) owns these
rules. The [communication decision](https://github.com/jackgeek/climon2/issues/66)
owns the agreed [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples). One per-instance
[FIFO](../TERMS.md#fifo) [response](../TERMS.md#response)/[event](../TERMS.md#event) stream preserves causal grant,
restoration, revocation, and result order. Deliveries cannot be dropped,
overwritten, or lossily coalesced under [backpressure](../TERMS.md#backpressure). Identify the current
[connection generation](../TERMS.md#connection-generation) separately from the
control period; old authority never transfers across reconnect.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must define
exact event tags, identity fields, error codes, and executable examples.
Essential control changes cannot silently disappear through plugin filtering.

[Wire grant](../daemon-client/event-control-granted.md) and
[revocation](../daemon-client/event-control-revoked.md) assign exact authority
fields and reasons, not native tags. Old authority is invalid immediately,
without waiting for revocation receipt before a new grant.
On [controller](../../../GLOSSARY.md#attachment-and-control) departure,
existing most-recent-control [attachment](../../../GLOSSARY.md#attachment-and-control) hand-back
precedes newest accepted passive successor promotion. Inheritance does not
select, resize, send input or acknowledge; promoted attachment membership
persists after switching/revocation. Committed termination overrides succession.
