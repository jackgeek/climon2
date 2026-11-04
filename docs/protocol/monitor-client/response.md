# Response

**Status:** planned semantics; in-process representation unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

A [response](../TERMS.md#response) travels opposite its [request](request.md)
and reports one explicit success or failure, including for
[plugin](../../../GLOSSARY.md)-injected work. Success and failure are distinct;
diagnostic prose is not a substitute for a machine-readable outcome.

Input acceptance is not application consumption; takeover acceptance is not
[readiness](../../../GLOSSARY.md); [detach](../../../GLOSSARY.md) acceptance is
not completed client cleanup. Unknown or already-completed correlation must
not create success. Communication failure leaves outstanding outcomes
[indeterminate](../TERMS.md#indeterminate-outcome).

[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns correlation and validation.
[Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71)
must assign result/error types, ownership, and valid examples; no [envelope](../TERMS.md#envelope) or
error-code catalog is currently frozen.
