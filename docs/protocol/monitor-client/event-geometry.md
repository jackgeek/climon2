# Geometry

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the monitor applies it ([#108](https://github.com/jackgeek/climon2/issues/108); [monitor README](../../../crates/climon2-monitor/README.md#selection-control-and-painting-108)).**
The [client](../../../GLOSSARY.md#sessions) reports effective daemon
[geometry](../TERMS.md#geometry) to the [monitor](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_GEOMETRY` / `data.geometry` has nonzero
[period](../../../GLOSSARY.md#attachment-and-control) and positive columns/rows,
scoped by record [generation](../TERMS.md#connection-generation).
No [restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts) field. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_GEOMETRY;
example.generation = 1;
example.data.geometry = (cm1_geometry_event){2, {100, 30}};
(void)example;
return (cm1_call){0};
```

Core support; actual geometry is distinct from Resize acceptance and
restoration/application/paint. Immediate causal changes precede their success;
deferred resize need not have a geometry event yet.
Canonical monitor display/restoration uses effective geometry even for passive
promotion/background grants. Only selected control receives monitor resize.
Host layout remains independent of daemon acknowledgement, with blank margins;
never fake PTY reflow. Geometry alone establishes no Ready/viewing/acknowledgement.

See [wire Geometry](../daemon-client/event-geometry.md),
[Resize](request-resize.md), [terminal sizing](../terminal-integration.md#host-canvas-geometry-and-coherent-readiness)
and [ownership/errors](core-v1.md#function-calls-and-ownership).
[#63](https://github.com/jackgeek/climon2/issues/63) owns sizing;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
