# Request

**Status:** communication design agreed, not implemented; exact declarations unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Core [requests](../TERMS.md#request) originate at the [monitor](../../../GLOSSARY.md).
Negotiated [extension](../../../GLOSSARY.md) requests may originate at either
endpoint through a [client](../../../GLOSSARY.md).

The [shared contract](../README.md#shared-in-process-communication-contract)
specifies nonblocking submit/drain, tagged [C-compatible](../TERMS.md#c-abi) records, ownership,
ordering, and validation; its [semantic examples](../README.md#semantic-examples)
apply here. Local acceptance proves validation and queuing only. Submit copies
accepted arguments before returning; rejection retains no ownership.

Each accepted request needs one explicit [response](../TERMS.md#response) while
communication remains healthy. Injected [plugin](../../../GLOSSARY.md) work participates in the same
correlation and outcome rules. Failure leaves unfinished outcomes
[indeterminate](../TERMS.md#indeterminate-outcome); retry is not implied.
Malformed or impermissible work must fail explicitly, not fabricate success.

Accepted [request IDs](../TERMS.md#request-id) strictly increase over the instance
lifetime without wrap. Reject IDs at or below the last accepted ID; locally
rejected submissions do not consume them. The monitor serializes per-instance
submissions when allocating IDs. Exhaustion requires a new instance.
Malformed records, invalid length/null-buffer descriptions, unsupported tags,
full queues, and closed handles are synchronous rejections, with no later outcome.
Invalid native pointers are not made safe by length validation.

Execution follows accepted submission order except independent shutdown.
Recheck authority at execution: stale connection, generation, control, or
[restoration](../../../GLOSSARY.md#restoration-and-presentation) state requires explicit asynchronous rejection if known unexecuted,
or an indeterminate outcome if execution is unknown. Never move stale work to a
new [connection generation](../TERMS.md#connection-generation) or silently drop it.
IDs do not supply a retry/idempotency mechanism.

[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns these agreed rules. See the [response page](response.md) for outcomes.
[Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71)
must assign exact identifier widths, argument types, result codes, and executable
examples; [operational limits](https://github.com/jackgeek/climon2/issues/69)
owns numerical bounds. No serialized [envelope](../TERMS.md#envelope) is used.
Executable C call examples cannot be validly published before #71.
