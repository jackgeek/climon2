# Session status

**Status:** agreed semantics, not implemented; exact declarations and wire
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
menu visibility. They neither grant control nor permit screen mirroring,
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

The [daemon contract](https://github.com/jackgeek/climon2/issues/67) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
exact names/types, synchronization [event](../TERMS.md#event) forms, timestamp encoding, episode
and final-outcome identities, capability tokens, daemon mapping, error codes,
and executable examples, preserving
[#66's agreed communication rules](https://github.com/jackgeek/climon2/issues/66).
No exact function, event name, capability token, or wire example is assigned
here. Unknown core tags are invalid, not implicitly negotiated extensions or
silent successes.
