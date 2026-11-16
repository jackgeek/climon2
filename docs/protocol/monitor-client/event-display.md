# Display

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [client](../../../GLOSSARY.md#sessions) delivers ordered terminal
[bytes](../TERMS.md#byte) to the [monitor](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_DISPLAY` / `data.display` carries [period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts) and nonempty
exact bytes, scoped by record [generation](../TERMS.md#connection-generation). This is not the wire binary header or
cross-release terminal cells. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#events-and-ordering).

```c
static const uint8_t bytes[] = {0x00, 0xff, 0x41};
cm1_record example = {0};
example.tag = CM1_DISPLAY;
example.generation = 1;
example.data.display = (cm1_display){{2, 3}, {bytes, sizeof bytes}};
(void)example;
return (cm1_call){0};
```

Core support; only controllers receive live bytes, including background
controllers. Passive/revoked metadata is not screen mirroring.
Restoration chunks occur between matching begin/complete; live chunks use current
authority afterward. No old-authority Display follows invalidation.
Preserve arbitrary bytes and order, including [NUL](../TERMS.md#nul)/invalid
[UTF-8](../TERMS.md#unicode-and-utf-8); bounded chunks
cannot silently alter/drop data.

Parse canonical monitor bytes at effective session dimensions into monitor-owned
[virtual terminal](../../../GLOSSARY.md#sessions), never write them directly to
[host](../../../GLOSSARY.md#sessions) or emit duplicate terminal-query replies.
The daemon is application-query authority. Stage/replace whole model/history
transactionally; apply before Ready and paint before selected Input.
Disconnected retained cache stays explicitly stale/read-only.
See [wire Display](../daemon-client/event-display.md),
[terminal contract](../terminal-integration.md),
[limits](../../monitor-operational-limits.md#queues-and-transfers) and
[immutable ownership](core-v1.md#function-calls-and-ownership).
[#63](https://github.com/jackgeek/climon2/issues/63) owns terminal application;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
