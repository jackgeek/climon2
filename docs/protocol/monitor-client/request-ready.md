# Report readiness

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) reports applied
[restoration](../../../GLOSSARY.md#restoration-and-presentation) through its
[client](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_READY` uses `args.authority`: nonzero current
[generation](../TERMS.md#connection-generation)/
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts).
No paint/selection boolean is sent. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_READY;
request.request_id = 3;
request.args.authority = (cm1_authority){generation, period, restoration_id};
return api->submit(communication, &request);
```

Submit after matching restoration completion and transactional whole-model/
history application, including background grants without paint or selection.
Completion receipt alone is insufficient. New begin/revocation invalidates
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Selected Input additionally waits for coherent painted current target;
Ready never establishes [viewing](../../../GLOSSARY.md#status-and-attention) or
[attention](../../../GLOSSARY.md#status-and-attention) acknowledgement.

Local admission is queuing only. Execution rechecks authority; stale unexecuted
work rejects and uncertain work is indeterminate without replay.
Success `CM1_READY_RESULT` carries changed and matching period/restoration.
See [wire Ready](../daemon-client/request-ready.md),
[terminal barriers](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness),
[ownership/errors](core-v1.md#function-calls-and-ownership) and
[Restoration completion](event-restoration-complete.md).
[#63](https://github.com/jackgeek/climon2/issues/63) owns application;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
