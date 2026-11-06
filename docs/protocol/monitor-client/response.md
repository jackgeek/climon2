# Response

**Status:** communication design agreed, not implemented; exact declarations unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

A [response](../TERMS.md#response) travels opposite its [request](../TERMS.md#request)
and reports one explicit success or failure, including for
[plugin](../../../GLOSSARY.md)-injected work. Success and failure are distinct;
diagnostic prose is not a substitute for a machine-readable outcome.

Local submission acceptance is only validated queuing, not this eventual
operation outcome or [daemon](../../../GLOSSARY.md#sessions) acceptance. Local rejection produces no later
response. Each accepted request receives one correlated outcome while healthy.
Known unexecuted stale work is explicitly rejected; uncertain execution remains
[indeterminate](../TERMS.md#indeterminate-outcome) and must not be automatically retried.

Input acceptance is not application consumption; takeover acceptance is not
[readiness](../../../GLOSSARY.md); [detach](../../../GLOSSARY.md) acceptance is
not completed [client](../../../GLOSSARY.md#sessions) cleanup. Unknown or already-completed correlation must
not create success. Communication failure leaves outstanding outcomes
[indeterminate](../TERMS.md#indeterminate-outcome).

Responses share one per-instance [FIFO](../TERMS.md#fifo) stream with
[events](../TERMS.md#event), preserving causal control/[restoration](../../../GLOSSARY.md#restoration-and-presentation)/result order.
Draining transfers immutable library-owned objects released through the same
library; outstanding deliveries prevent successful instance destruction.
The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) specify admission,
ownership, and bounded progress.

[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns these agreed rules. See [request admission](request.md).
[Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71)
must assign exact result/error types and executable examples; no error-code
catalog is currently frozen. There is no serialized
[envelope](../TERMS.md#envelope).
