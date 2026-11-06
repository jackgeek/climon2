# Event

**Status:** communication design agreed, not implemented; exact declarations unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Core [events](../TERMS.md#event) originate at the [client](../../../GLOSSARY.md).
Negotiated [extension](../../../GLOSSARY.md) events may travel either way.
Essential control and [restoration](../../../GLOSSARY.md#restoration-and-presentation)
changes must retain their ordering and cannot silently disappear through
[plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) filtering.

[Plugin](../../../GLOSSARY.md) status must flow to the [monitor](../../../GLOSSARY.md) even for passive,
background, or [revoked](../../../GLOSSARY.md) clients. It does not grant screen
access, application input, or resize permission.

[Title state and changes](event-title.md) use [daemon](../../../GLOSSARY.md#sessions) core, not plugin traffic
or screen mirroring. When supported,
supply authoritative current daemon title state and ordered updates without a
subscription gap, regardless of control, menu visibility, or
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Exact fields and text encodings remain unresolved.

[Session status](event-session-status.md) is also daemon-owned core data, with
its stored change timestamp. Reports support passive startup ordering and
reconnection without taking control. Plugin implementation is deferred.

Events share one per-instance [FIFO](../TERMS.md#fifo) stream with
[responses](../TERMS.md#response). Drain transfers immutable library-owned
objects released through the same library. Bounds include unreleased objects.
Required status transitions, [attention](../../../GLOSSARY.md#status-and-attention) episodes, responses, and
control/restoration boundaries cannot be dropped or lossily coalesced.
Closure reason is observable outside ordinary queue capacity, with queued
records preserved ahead of the terminal indication.

Initial core metadata synchronization is explicitly delimited per
[connection generation](../TERMS.md#connection-generation), followed by gap-free
changes independently of control/readiness/selection. It does not refresh stale
terminal content. Unknown core tags are invalid, not implicit extensions.
The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) own delivery, ownership,
notification, ordering, and validation, agreed in
[the communication decision](https://github.com/jackgeek/climon2/issues/66).
[Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71)
must assign exact tags, types, release declarations, and executable examples.
No serialized [envelope](../TERMS.md#envelope) or monitor callback is used.
