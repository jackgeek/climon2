# Request refused — `0x26`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout and its order; no released program speaks it yet, and a broken stream rule still ends the whole link rather than the one stream.**
The [relay](../../../../GLOSSARY.md#monitor-links) reports that the client
library did not accept a [request](request.md) when the relay submitted it.
See the [overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Request ID | `u64` | The refused request's [request ID](../TERMS.md#request-id). |
| Code | `u32` | The call result the library returned, such as full or closed. |
| Error | `error` | The call's error, when there is one. |

## Example

Request 9 refused because the library's request queue was full (code 5):

```text
26 00 00 00 00 00 00 00 09 00 00 00 05 00 00 00
00 00 00 00 00 00 00 00 00
```

## Rules

- A local monitor learns this from the return value of the library's submit
  call. Across a link that value has to travel as a record.
- A refused request was never admitted. It has no effect and gets no
  [Response](event.md).
- An accepted request gets no record here. Its outcome arrives as a Response
  event.
- It needs no credit and is always sendable.
- The stream carries on.

Owned by [#480](https://github.com/jackgeek/climon2/issues/480).
