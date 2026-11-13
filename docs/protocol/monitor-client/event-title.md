# Title state and changes

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) relays `title-v1` for
[passive monitor connections](../../../GLOSSARY.md#attachment-and-control)
([#93](https://github.com/jackgeek/climon2/issues/93)).**
The [session daemon](../../../GLOSSARY.md#sessions) supplies title metadata
through its [client](../../../GLOSSARY.md#sessions) to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

Initial `cm1_metadata.title` uses unavailable/unset/value. Later `CM1_TITLE` /
`data.title` uses supported unset/value only, with length-delimited inert
[UTF-8](../TERMS.md#unicode-and-utf-8); a supported value can be empty.
See [header](client-v1.h), [values](core-v1.md#shared-values) and
[mapping](core-v1.md#events-and-ordering).

```c
static const uint8_t title[] = "Build ready";
cm1_record example = {0};
example.tag = CM1_TITLE;
example.generation = 1;
example.data.title = (cm1_title){CM1_VALUE, 0, {title, sizeof title - 1}};
(void)example;
return (cm1_call){0};
```

Require selected `title-v1` through library/daemon; missing title support leaves
control usable with command/arguments fallback. Atomic initial state and gap-free
changes flow while passive/background/revoked, without display scraping or plugins.
Empty/unset selects fallback; unavailable is explicit uncertainty, never a
fabricated supported-empty value. Cached title after loss is stale.
Render title literally, not as terminal instructions; changing/duplicate labels
never retarget session identity. Oversized required titles fail explicitly,
not healthy truncation.

See [Initial metadata](event-initial-metadata.md),
[wire Title](../daemon-client/event-title-changed.md),
[limits](../../monitor-operational-limits.md#payloads-and-parser-state)
and [ownership/order/errors](core-v1.md#function-calls-and-ownership).
[#67](https://github.com/jackgeek/climon2/issues/67) owns title support;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
