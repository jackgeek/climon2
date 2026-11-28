# Stream refused — `0x22`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout, its causes and its order; no released program speaks it yet.**
The [relay](../../../../GLOSSARY.md#monitor-links) reports that the client
asked for by [Open stream](stream-open.md) could not start. It ends the
stream. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Cause | `u16` | One of the codes below. |
| Error | `error` | The library's or loader's error, when there is one. |

| Code | Cause | The hub monitor then |
|---|---|---|
| 1 | The session is not advertised | Treats the start as failed. |
| 2 | The session's version folder is missing | Reports a failed start naming the missing version. |
| 3 | The client library failed to load | Reports a failed start. |
| 4 | Unsupported client interface revision | Reports a failed start naming the cause. |
| 5 | Relay out of resources | Shows `Awaiting resources` and retries at its admission rate. |
| 6 | The client failed to start | Reports a failed start. |

## Example

A relay that cannot admit another client:

```text
22 00 05 00 00 00 00 00 00 00 00 00 00 00 00
```

## Rules

- The stream has ended. Its number is never used again on this connection.
- The link and every other stream carry on.
- The [hub monitor](../../../../GLOSSARY.md#monitor-links) treats the Error
  texts as hostile.
- A Cause the hub monitor does not know is a
  [protocol violation](README.md#protocol-violations).

Owned by [#196](https://github.com/jackgeek/climon2/issues/196) and
[#412](https://github.com/jackgeek/climon2/issues/412).
