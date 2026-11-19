# Request admission

**Status: agreed revision-1 specification. The
[client library](../../../../GLOSSARY.md#sessions) admits monitor-mode
[Open](request-open.md) and [Leave](request-detach.md)
([#93](https://github.com/jackgeek/climon2/issues/93)), interactive Open,
[Acquire](request-take-control.md), [Input](request-input.md),
[Resize](request-resize.md), [Ready](request-ready.md) and
[Focus](request-focus.md) ([#94](https://github.com/jackgeek/climon2/issues/94)),
[Set viewing](request-viewing.md) and [Acknowledge](request-acknowledge.md)
([#95](https://github.com/jackgeek/climon2/issues/95)), and
[Kill](request-kill.md) ([#96](https://github.com/jackgeek/climon2/issues/96)).
See its
[implemented behavior](README.md#implemented-behavior). The monitor submits
these requests ([#106](https://github.com/jackgeek/climon2/issues/106) to [#117](https://github.com/jackgeek/climon2/issues/117)).**
The [monitor](../../../../GLOSSARY.md#sessions) submits a
[request](../TERMS.md#request) through the [client library](../../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

[`cm1_request`](client-v1.h) has a native tag, reserved zero, positive
`uint64_t` [request ID](../TERMS.md#request-id) and the tag's argument union member. The
[complete request mapping](README.md#requests-and-result-mapping) fixes every
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
errors are synchronous; see [errors](README.md#errors-terminal-evidence-and-cleanup).
Accepted work executes [FIFO](../TERMS.md#fifo) except independent shutdown;
stale unexecuted work is explicitly rejected, uncertain effects are
[indeterminate](../TERMS.md#indeterminate-outcome) and never replayed.
Public IDs are mapped separately from daemon IDs. See [ownership](README.md#function-calls-and-ownership)
and [Response](response.md). [#66](https://github.com/jackgeek/climon2/issues/66)
owns admission/order; [#71](https://github.com/jackgeek/climon2/issues/71) owns these declarations.
