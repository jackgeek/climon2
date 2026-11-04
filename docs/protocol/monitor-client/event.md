# Event

**Status:** planned semantics; in-process representation unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Core [events](../TERMS.md#event) originate at the [client](../../../GLOSSARY.md).
Negotiated [extension](../../../GLOSSARY.md) events may travel either way.
Essential control and restoration changes must retain their ordering and cannot
silently disappear through plugin filtering.

[Plugin](../../../GLOSSARY.md) status must flow to the [monitor](../../../GLOSSARY.md) even for passive,
background, or [revoked](../../../GLOSSARY.md) clients. It does not grant screen
access, application input, or resize permission.

[Title state and changes](event-title.md) are core metadata, not plugin events
or screen mirroring. Supply current daemon title state and ordered updates
without a subscription gap, regardless of control, menu visibility, or
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Exact representation remains unresolved.

[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns delivery, validation, and unknown-event behavior.
[Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71)
must assign event names, types, lifetime rules, and examples. No event [envelope](../TERMS.md#envelope)
or callback signature is assigned here.
