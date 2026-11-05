# Session status

**Status:** agreed semantics, not implemented; exact declarations and wire
schemas unresolved. See the [overview](../README.md),
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

Reports work before control/input readiness and while background or
[revoked](../../../GLOSSARY.md#attachment-and-control), independently of
menu visibility. They neither grant control nor permit screen mirroring,
application input, or resize.

Use the [canonical mechanics and diagrams](../../session-status.md) for all
transitions, effective-state timestamp semantics, status/time ordering,
initial selection, and notification behavior. Do not assign a connection-time
timestamp to unchanged cached daemon state, silently map unsupported status to
`cleared`, or treat a timestamp as an implicit unique attention-episode ID.

Disconnection without a confirmed command result is a separate monitor-owned
fact, not a daemon `failed` result. Preserve definitively reported final
statuses after closure. Reports do not prove application consumption, accepted
detach, or completed thread/resource cleanup.

The [daemon contract](https://github.com/jackgeek/climon2/issues/67),
[communication decision](https://github.com/jackgeek/climon2/issues/66), and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
names/types, initial snapshot/change ordering, timestamp encoding, episode
identity, compatibility, validation, rejection, ownership, and valid examples.
No exact function, event name, capability token, or wire example is assigned
here. Unknown traffic must follow those completed contracts, not an invented
silent-success rule.
