# Resize

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) requests
[geometry](../TERMS.md#geometry) through its [client](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_RESIZE` uses `args.resize`: current [generation](../TERMS.md#connection-generation)/
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts) and
positive columns/rows. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_RESIZE;
request.request_id = 11;
request.args.resize = (cm1_resize){
    {generation, period, restoration_id}, {100, 30}};
return api->submit(communication, &request);
```

Only the selected [controller](../../../GLOSSARY.md#attachment-and-control)
receives monitor resize, including before [restoration](../../../GLOSSARY.md#restoration-and-presentation)
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
[Host](../../../GLOSSARY.md#sessions) desire excludes the top row; effective daemon dimensions remain
authoritative. Background grants/menu opening never imply resize; a real host
resize still updates selected control under overlays. Too-small space sends no
invalid geometry. See [terminal geometry](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness).

Local admission copies/queues only; recheck current authority at execution.
Known stale work rejects explicitly; uncertain effects are indeterminate, never
replayed. `CM1_RESIZE_ACCEPTED` has no payload and proves neither final geometry
nor applied/painted restoration. Observe [Geometry](event-geometry.md) and
restoration separately. See [wire Resize](../daemon-client/request-resize.md),
[ownership/errors](core-v1.md#function-calls-and-ownership) and
[limits](../../monitor-operational-limits.md).
[#63](https://github.com/jackgeek/climon2/issues/63) owns geometry/application;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
