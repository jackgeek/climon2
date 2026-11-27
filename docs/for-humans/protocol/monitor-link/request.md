# Request — `0x41` to `0x4b`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [hub monitor](../../../../GLOSSARY.md#monitor-links) sends one client
interface [request](../TERMS.md#request) on a
[session stream](../../../../GLOSSARY.md#monitor-links). The
[relay](../../../../GLOSSARY.md#monitor-links) submits it to the stream's
client instance unchanged. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

This page owns only the encoding. What each request means, when it is
allowed and what its response proves belong to the
[client interface](../monitor-client/README.md) page linked in each row.

## Encoding rule

Every request record is built from its C structure in the client interface
[header](../monitor-client/client-v1.h) by one rule:

1. The tag is `0x40` plus the request's `CM1_` number.
2. The body starts with the request ID as a `u64`.
3. The fields of the request's arguments follow, in the header's order.
4. A reserved field is left out. A pointer with a length becomes `bytes`. A
   token list becomes `tokens`.

Two shared groups appear below. **Authority** is three `u64` values:
generation, control period and restoration ID. **Geometry** is two `u32`
values: columns and rows.

## Layouts

| Tag | Request | Body after the request ID |
|---|---|---|
| `0x41` | [Open](../monitor-client/request-open.md) | Mode `u32`, Geometry, required `tokens`, optional `tokens`. |
| `0x42` | [Input](../monitor-client/request-input.md) | Authority, input `bytes`. |
| `0x43` | [Resize](../monitor-client/request-resize.md) | Authority, Geometry. |
| `0x44` | [Take control](../monitor-client/request-take-control.md) | Generation `u64`. |
| `0x45` | [Ready](../monitor-client/request-ready.md) | Authority. |
| `0x46` | [Set viewing](../monitor-client/request-viewing.md) | Authority, value `u8`. |
| `0x47` | [Acknowledge](../monitor-client/request-acknowledge.md) | Authority. |
| `0x48` | [Leave](../monitor-client/request-detach.md) | Generation `u64`. |
| `0x49` | [Kill](../monitor-client/request-kill.md) | Required `tokens`. |
| `0x4a` | [Focus](../monitor-client/request-focus.md) | Authority, value `u8`. |
| `0x4b` | [Break](../monitor-client/request-break.md) | Authority. |

## Example

Input, request 9, generation 3, control period 2, restoration 1, the three
[bytes](../TERMS.md#byte) `ls` and a carriage return:

```text
42 00 00 00 00 00 00 00 09 00 00 00 00 00 00 00
03 00 00 00 00 00 00 00 02 00 00 00 00 00 00 00
01 00 00 00 03 6c 73 0d
```

## Rules

- Requests are allowed after [Stream opened](stream-opened.md) and before the
  hub monitor's [Close stream](stream-close.md).
- The relay submits requests in the order they arrive. It never invents,
  repeats, reorders or drops one.
- A request the library accepts is answered by a Response
  [event](event.md). One it does not accept is answered by
  [Request refused](request-refused.md).
- Requests use no [credit](credit.md). The library's own request queue bounds
  them: when it is full the request is refused.
- Input typed while a link is down is discarded by the hub monitor. It is
  never queued or replayed.
- Delete on the hub monitor is a Kill request followed by a dismissal the hub
  monitor keeps for itself. The relay never deletes anything.
- A tag from `0x41` to `0x4b` with a malformed body is a
  [stream violation](README.md#protocol-violations).

Owned by [#195](https://github.com/jackgeek/climon2/issues/195) for what is
carried and [#480](https://github.com/jackgeek/climon2/issues/480) for the
encoding.
