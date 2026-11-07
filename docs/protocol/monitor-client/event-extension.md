# Extension event

**Status:** future relay requirements only;
[plugin](../../../GLOSSARY.md#protocol-and-plugin-concepts) [APIs](../TERMS.md#api), configuration,
implementation, and examples are deferred, not release prerequisites.
Interface [schemas](../TERMS.md#schema) are unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Either endpoint may emit an [event](../TERMS.md#event) for a negotiated
[extension](../../../GLOSSARY.md). The [client](../../../GLOSSARY.md) relays
[plugin](../../../GLOSSARY.md)-owned [payloads](../TERMS.md#payload) without
interpreting them as core commands.

Plugin data must reach the [monitor](../../../GLOSSARY.md) even when the client
is passive, in the background, or [revoked](../../../GLOSSARY.md). An
[attention](../../../GLOSSARY.md#status-and-attention) notification must not
depend on control. This does not authorize core screen mirroring, application
input, or resize. Unnegotiated traffic must fail explicitly; optional events may
be filtered only when their extension permits it.

The agreed [communication contract](../README.md#shared-in-process-communication-contract)
and its [core semantic examples](../README.md#semantic-examples), owned by
[#66](https://github.com/jackgeek/climon2/issues/66), preserve these future
guarantees without assigning a plugin API. Unknown core tags are invalid, not
implicit extensions; negotiate the explicit name/revision intersection including
relay support. Preserve arbitrary binary data and truthful outcomes for any
injected, removed, or replaced work.
[The daemon contract](https://github.com/jackgeek/climon2/issues/67) owns wire
support, and [interface schemas](https://github.com/jackgeek/climon2/issues/71)
owns exact core declarations. Plugin schemas, API examples, and implementation
remain deferred; no serialized [envelope](../TERMS.md#envelope) is used by this
in-process contract. Plugin payload schemas belong to their extensions.
See [extension requests](request-extension.md) and
[operational limits](https://github.com/jackgeek/climon2/issues/69).
