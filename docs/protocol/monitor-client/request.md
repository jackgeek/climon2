# Request

**Status:** planned semantics; in-process representation unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Core [requests](../TERMS.md#request) originate at the [monitor](../../../GLOSSARY.md).
Negotiated [extension](../../../GLOSSARY.md) requests may originate at either
endpoint through a [client](../../../GLOSSARY.md).

Each accepted request needs one explicit [response](response.md) while
communication remains healthy. Injected [plugin](../../../GLOSSARY.md) work participates in the same
correlation and outcome rules. Failure leaves unfinished outcomes
[indeterminate](../TERMS.md#indeterminate-outcome); retry is not implied.
Malformed or impermissible work must fail explicitly, not fabricate success.

[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns request dispatch, ordering, validation, compatibility, and correlation.
[Complete monitor-client interface schemas](https://github.com/jackgeek/climon2/issues/71)
must assign identifiers, argument types, limits, and valid examples. No
serialized [envelope](../TERMS.md#envelope), function signature, [ID](../TERMS.md#request-id) encoding, or error spelling is
assigned here.
