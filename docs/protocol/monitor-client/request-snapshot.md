# Resource and progress snapshot

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) implements snapshot
([#85](https://github.com/jackgeek/climon2/issues/85)): tracked instance
allocation, queued and delivered records, deliveries, workers and descriptors.
It tracks no shared-release allocation, and request, operation, input and
display measurements stay zero until [requests](../TERMS.md#request) exist
([#93](https://github.com/jackgeek/climon2/issues/93)). The caller is not
implemented.**
The [monitor](../../../GLOSSARY.md#sessions) queries its
[client](../../../GLOSSARY.md#sessions)'s local resource/progress measurements.
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`snapshot(cm1_communication *, cm1_metrics *)` returns `cm1_call`.
The [header](client-v1.h) fixes current/peak instance/shared-release allocations,
queue bytes and record/delivery/operation/worker/descriptor counts, plus accepted
input, delivered display, drained-record and released-delivery counters.
See [measurement semantics](core-v1.md#startup-limits-and-resources).

```c
cm1_metrics metrics = {0};
return api->snapshot(communication, &metrics);
```

Thread-safe bounded local work, no I/O/wait/join, available until communication
handle release. All retained allocation capacity and unreleased deliveries count.
Shared release allocations are deduplicated by release; stacks/reservations follow
qualified requirements. Overflow fails explicitly, never wraps.
This is not daemon acceptance, application consumption, Ready, cleanup or join.
Monitor model/history/render/whole-process measurements remain caller-owned.

Invalid handle use remains caller misuse; ordinary reported failures are explicit
and output is cleared. See [ownership/errors](core-v1.md#function-calls-and-ownership)
and [operational evidence](../../monitor-operational-limits.md#required-implementation-evidence).
[#69](https://github.com/jackgeek/climon2/issues/69) owns measurement requirements;
[#71](https://github.com/jackgeek/climon2/issues/71) owns the native fields.
