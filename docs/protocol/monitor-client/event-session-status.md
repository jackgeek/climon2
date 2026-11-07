# Session status

**Status:** agreed semantics and wire mapping, not implemented; native
[schemas](../TERMS.md#schema) unresolved. See the [overview](../README.md),
[domain glossary](../../../GLOSSARY.md), and
[technical terms](../TERMS.md).

The [session daemon](../../../GLOSSARY.md#sessions) owns
[activity state, attention state, and effective session status](../../../GLOSSARY.md#status-and-attention).
The [client](../../../GLOSSARY.md#sessions) reports current status and its stored
[UTC](../TERMS.md#utc) change timestamp to the
[monitor](../../../GLOSSARY.md#sessions) on connection, followed by ordered
changes without a subscription gap. This is core data, not a
[plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) extension.
Plugin implementation is deferred.

Reports work before control/input [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) and while background or
[revoked](../../../GLOSSARY.md#attachment-and-control), independently of
menu visibility. They neither [grant](../../../GLOSSARY.md#attachment-and-control)
control nor permit screen mirroring,
application input, or resize.

Use the [canonical mechanics and diagrams](../../session-status.md) for all
transitions, effective-state timestamp semantics, status/time ordering,
initial selection, and notification behavior. Internal `agent_working` and
`user_working` both project to `running` when attention is `cleared`; changes
between them or between detection modes alone do not update the effective
timestamp. Root terminal-status reports, heuristic inactivity, and recognized
notifications can request attention, but child program-status records cannot.
Structured [notification](event-notification.md) and
[progress](event-progress.md) metadata are separate interactions, not raw
terminal sequences embedded in status reports.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) require explicit
initial core metadata synchronization boundaries per
[connection generation](../TERMS.md#connection-generation), then gap-free
changes. Report supported current status and its stored change time, or
explicit unsupported/unknown state. Keep reliable attention-episode and final
outcome identities separate from request, control,
[restoration](../../../GLOSSARY.md#restoration-and-presentation), and generation
identities. Episode deduplication never relies on UTC uniqueness.
Status transitions and attention episodes cannot be dropped, overwritten,
reordered, or lossily coalesced under [backpressure](../TERMS.md#backpressure). Metadata synchronization
does not make the retained terminal current.

Do not assign a connection-time
timestamp to unchanged cached daemon state, silently map unsupported status to
`cleared`, or treat a timestamp as an implicit unique attention-episode ID.

Disconnection without a confirmed command result is a separate monitor-owned
fact, not a daemon `failed` result. Preserve definitively reported final
statuses after closure. Reports do not prove application consumption, accepted
detach, or completed thread/resource cleanup.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67) assigns
`status-v1`, [Initial metadata](../daemon-client/event-initial-metadata.md),
[Status changed](../daemon-client/event-status-changed.md), and
[Command final](../daemon-client/event-command-final.md).
Final carries the terminal transition with no duplicate status-change event.
Wire Status has required state, exact UTC time and nullable episode counter;
pending coalesces, acknowledgement preserves identity, clear removes it.
Final deduplication uses session [GUID](../TERMS.md#guid) plus fixed category `final outcome`,
not exit/signal variant. Terminal status/result must agree: completed is normal
exit zero; failed is nonzero exit or signal.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) still owns
native representation and executable examples, preserving
[#66's agreed communication rules](https://github.com/jackgeek/climon2/issues/66).
Unknown core tags are invalid, not implicitly negotiated extensions or
silent successes.
