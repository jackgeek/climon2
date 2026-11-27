# Credit — `0x25`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [hub monitor](../../../../GLOSSARY.md#monitor-links) grants a
[session stream](../../../../GLOSSARY.md#monitor-links) more
[bytes](../TERMS.md#byte) of events. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Amount | `u32` | Bytes to add to the stream's credit. At least 1. |

## Example

16,384 more bytes:

```text
25 00 00 40 00
```

## Rules

- A stream's credit starts at the Credit field of its
  [Open stream](stream-open.md). Each Credit record adds to it. Each
  [Event](event.md) the [relay](../../../../GLOSSARY.md#monitor-links) sends
  takes away the length of that Event record, tag included.
- The hub monitor grants credit as it drains the session's event queue. A
  stream's credit is the free space in that queue, so there is no separate
  window setting.
- The relay sends an Event only when the stream's credit covers all of it. It
  stops draining that client's library while it waits.
- Credit above 4,294,967,295 bytes, or an Amount of zero, ends the stream as
  a [stream violation](stream-ended.md).
- An Event that exceeds the credit the hub monitor granted is a violation by
  the relay. The hub monitor closes the stream.

Owned by [#412](https://github.com/jackgeek/climon2/issues/412).
