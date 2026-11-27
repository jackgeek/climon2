# Session withdrawn — `0x11`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [relay](../../../../GLOSSARY.md#monitor-links) withdraws a
[session](../../../../GLOSSARY.md#sessions) it advertised, on stream 0. See
the [overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Session ID | 16 [bytes](../TERMS.md#byte) | The ID from the session's [Session](session.md) record. |

## Example

```text
11 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd
ef
```

## Rules

- The relay withdraws a session only when discovery proves it stale. A
  session that ended a moment ago and is still listed is not withdrawn.
- The [hub monitor](../../../../GLOSSARY.md#monitor-links) applies its local
  rule: a row that never connected is dropped, and one that was connected
  stays `Disconnected` with its history.
- Withdrawing does not end the session's stream. A stream ends with its own
  [Stream ended](stream-ended.md).
- A Session ID the relay has not advertised on this connection is a
  [protocol violation](README.md#protocol-violations).

Owned by [#195](https://github.com/jackgeek/climon2/issues/195).
