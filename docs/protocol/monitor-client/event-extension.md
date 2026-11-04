# Extension event

**Status:** required relay behavior; interface [schema](../TERMS.md#schema) unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Either endpoint may emit an [event](../TERMS.md#event) for a negotiated
[extension](../../../GLOSSARY.md). The [client](../../../GLOSSARY.md) relays
[plugin](../../../GLOSSARY.md)-owned [payloads](../TERMS.md#payload) without
interpreting them as core commands.

Plugin data must reach the [monitor](../../../GLOSSARY.md) even when the client
is passive, in the background, or [revoked](../../../GLOSSARY.md). An attention notification must not
depend on control. This does not authorize core screen mirroring, application
input, or resize. Unnegotiated traffic must fail explicitly; optional events may
be filtered only when their extension permits it.

[Communication](https://github.com/jackgeek/climon2/issues/66),
[the daemon contract](https://github.com/jackgeek/climon2/issues/67), and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must specify
names/revisions, negotiation, representation, errors, and examples. No attention
status vocabulary or serialized [envelope](../TERMS.md#envelope) is assigned. Plugin payload schemas
belong to their extensions.
See [extension requests](request-extension.md) and
[operational limits](https://github.com/jackgeek/climon2/issues/69).
