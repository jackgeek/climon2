# Request admission

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) submits a
[request](../TERMS.md#request) through the [client library](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

[`cm1_request`](client-v1.h) has a native tag, reserved zero, positive
`uint64_t` [request ID](../TERMS.md#request-id) and the tag's argument union member. The
[complete request mapping](core-v1.md#requests-and-result-mapping) fixes every
variant, required field, capability and result. No extension variant is published.

```c
cm1_request request = {0};
request.tag = CM1_ACQUIRE;
request.request_id = 10;
request.args.generation = generation;
return api->submit(communication, &request);
```

IDs increase strictly per instance's accepted submissions. Local rejection
does not consume an ID, retain buffers or produce a later response. Acceptance
copies all data and proves only queuing. Full/closed/malformed/unsupported/ID
errors are synchronous; see [errors](core-v1.md#errors-terminal-evidence-and-cleanup).
Accepted work executes [FIFO](../TERMS.md#fifo) except independent shutdown;
stale unexecuted work is explicitly rejected, uncertain effects are
[indeterminate](../TERMS.md#indeterminate-outcome) and never replayed.
Public IDs are mapped separately from daemon IDs. See [ownership](core-v1.md#function-calls-and-ownership)
and [Response](response.md). [#66](https://github.com/jackgeek/climon2/issues/66)
owns admission/order; [#71](https://github.com/jackgeek/climon2/issues/71) owns these declarations.
