# Restoration start

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [client](../../../GLOSSARY.md#sessions) begins a
[restoration](../../../GLOSSARY.md#restoration-and-presentation) for the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_RESTORATION_BEGIN` / `data.restoration` contains nonzero
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts) and positive
effective [geometry](../TERMS.md#geometry), scoped by record
[generation](../TERMS.md#connection-generation).
See [header](client-v1.h), [values](core-v1.md#shared-values) and
[mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_RESTORATION_BEGIN;
example.generation = 1;
example.data.restoration = (cm1_restoration){2, 3, {80, 24}};
(void)example;
return (cm1_call){0};
```

Core support; grant precedes first begin, later begin may occur in the same
period. New begin invalidates prior readiness/incomplete model. Canonical
monitor bytes use effective geometry, never host desire, including background
grants. Stage a separate whole model/history, apply transactionally after
matching completion; no merging/dedup of old history.
Zero or more matching Display chunks follow, then complete. Superseded
restoration gets no fabricated completion. Metadata may interleave without
changing terminal-byte order. Begin is not input readiness or viewing.

See [wire Begin](../daemon-client/event-restoration-begin.md),
[Display](event-display.md), [completion](event-restoration-complete.md),
[terminal application](../terminal-integration.md) and
[ownership/errors](core-v1.md#function-calls-and-ownership).
[#63](https://github.com/jackgeek/climon2/issues/63) owns application;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
