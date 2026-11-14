# Initial metadata

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) delivers this barrier for
[passive monitor connections](../../../GLOSSARY.md#attachment-and-control)
([#93](https://github.com/jackgeek/climon2/issues/93)): the title by
selection, status and progress unavailable until their families are relayed,
and the command result.**
The [session daemon](../../../GLOSSARY.md#sessions), through the
[client](../../../GLOSSARY.md#sessions), supplies the
[monitor](../../../GLOSSARY.md#sessions) an atomic initial report.
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_INITIAL_METADATA` uses `cm1_record.data.metadata`: title, available status,
available progress and optional command result in the [header](client-v1.h).
See [value constraints](core-v1.md#shared-values) and [event mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_INITIAL_METADATA;
example.generation = 1;
example.data.metadata.title.state = CM1_UNSET;
/* Status/progress are explicitly unavailable; no final command is known. */
(void)example;
return (cm1_call){0};
```

This example requires selected title support, but no status/progress support.
Unavailable is never supported-empty, idle, cleared or no-progress. Available
terminal status and command result must agree. Open success precedes this
single barrier; subsequent selected-family changes have no subscription gap.
Interactive grant/restoration follows metadata. Passive/background/revoked
delivery requires no control or readiness and never refreshes cached terminal
content or acknowledges attention.

See [wire Initial metadata](../daemon-client/event-initial-metadata.md),
[ownership](core-v1.md#function-calls-and-ownership) and
[title](event-title.md)/[status](event-session-status.md)/[progress](event-progress.md).
[#67](https://github.com/jackgeek/climon2/issues/67) owns the barrier;
[#71](https://github.com/jackgeek/climon2/issues/71) owns its native declaration.
