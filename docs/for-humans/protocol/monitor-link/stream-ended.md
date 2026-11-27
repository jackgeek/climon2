# Stream ended — `0x24`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [relay](../../../../GLOSSARY.md#monitor-links) reports that a stream's
client has ended. It is always the stream's last record. See the
[overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Reason | `u8` | 1 the client ended, 2 the stream fell behind, 3 stream violation. |
| Workers cleanup | `u32` | From the client's run report. |
| Generation | `u64` | The run report's connection generation. |
| Client outcome | `u32` | Stopped or failed. |
| Retry | `u32` | None, passive or permanent. |
| Leave | `u32` | What became of the deliberate Leave. |
| Attachment detached | `u8` | 1 when an attachment was detached. |
| Connection closed | `u8` | 1 when the daemon connection closed. |
| Error | `error` | The client's final error, when there is one. |

The fields after Reason are the client interface's run report, with the
values the [client interface](../monitor-client/README.md) defines.

## Reasons

- **1, the client ended.** By itself, or after
  [Close stream](stream-close.md).
- **2, the stream fell behind.** The stream had no [credit](credit.md) for
  three quarters of the relay's delivery stall deadline. The relay sent a
  deliberate Leave so that a slow link never ends a busy session. The
  [hub monitor](../../../../GLOSSARY.md#monitor-links)'s ordinary passive
  retry opens a new stream.
- **3, stream violation.** The hub monitor broke a rule of this stream. See
  [protocol violations](README.md#protocol-violations).

## Example

A client that stopped after Close stream, with its Leave confirmed:

```text
24 01 00 00 00 01 00 00 00 00 00 00 00 03 00 00
00 01 00 00 00 01 00 00 00 02 00 01 00 00 00 00
00 00 00 00 00 00 00 00
```

## Rules

- It needs no credit and is always sendable.
- [Events](event.md) the relay could not send for lack of credit are dropped
  when the stream ends.
- The hub monitor treats the run report as it treats a local client's.
- When the link itself ends, no Stream ended is sent. The hub monitor ends
  every stream of that link as a lost connection.

Owned by [#195](https://github.com/jackgeek/climon2/issues/195) and
[#198](https://github.com/jackgeek/climon2/issues/198).
