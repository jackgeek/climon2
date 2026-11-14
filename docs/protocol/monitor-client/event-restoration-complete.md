# Restoration completion

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [client](../../../GLOSSARY.md#sessions) reports complete
[restoration](../../../GLOSSARY.md#restoration-and-presentation) delivery to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_RESTORATION_COMPLETE` / `data.barrier` has nonzero
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), scoped by
record [generation](../TERMS.md#connection-generation).
No [geometry](../TERMS.md#geometry) or applied flag.
See [header](client-v1.h), [values](core-v1.md#shared-values) and
[mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_RESTORATION_COMPLETE;
example.generation = 1;
example.data.barrier = (cm1_barrier){2, 3};
(void)example;
return (cm1_call){0};
```

Core support; follows matching begin and all restoration bytes, including empty
restoration. Superseded/revoked restoration gets no fabricated complete.
Receipt proves delivery, not monitor application, coherent paint or input Ready.
Replace whole staged model/history transactionally, then submit
[Ready](request-ready.md). Background application needs no selection/paint;
selected Input additionally waits for coherent painted current target.
No viewing/acknowledgement follows automatically.

See [wire Complete](../daemon-client/event-restoration-complete.md),
[terminal barriers](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness),
[ownership/order/errors](core-v1.md#function-calls-and-ownership) and
[limits](../../monitor-operational-limits.md).
[#63](https://github.com/jackgeek/climon2/issues/63) owns application;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
