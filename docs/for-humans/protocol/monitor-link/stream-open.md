# Open stream — `0x20`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout and the numbering rule; no released program speaks it yet, and no relay yet starts a client.**
The [hub monitor](../../../../GLOSSARY.md#monitor-links) asks the
[relay](../../../../GLOSSARY.md#monitor-links) to start a client for one
advertised session. It is the first record of a new
[session stream](../../../../GLOSSARY.md#monitor-links). See the
[overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Session ID | 16 [bytes](../TERMS.md#byte) | The ID from the session's [Session](session.md) record. |
| Generation | `u64` | The hub monitor's [connection generation](../TERMS.md#connection-generation) for this client. Every [Event](event.md) on the stream repeats it. |
| Interface revision | `u32` | The client interface revision the stream carries. Link revision 1 allows only 1. |
| Credit | `u32` | The stream's first [credit](credit.md), in bytes. |

## Example

Session `0123…cdef`, generation 3, interface revision 1, 1,048,576 bytes of
credit, sent on a stream number not used before:

```text
20 01 23 45 67 89 ab cd ef 01 23 45 67 89 ab cd
ef 00 00 00 00 00 00 00 03 00 00 00 01 00 10 00
00
```

## What the relay does

The relay loads the client library of the session's own release, as a monitor
does, and [creates](../monitor-client/request-create.md) and runs one client
instance. It fills the instance's startup values from its own configuration.
It answers with [Stream opened](stream-opened.md) or
[Stream refused](stream-refused.md).

Opening a stream does not open a connection to the session daemon. The hub
monitor does that next, with an Open [request](request.md).

## Rules

- The hub monitor decides when to open a stream, by its own scheduling,
  admission and retry rules. The relay never opens one and never retries.
- One stream serves one client instance. A reconnect of that client is a new
  stream with a new Generation.
- The stream number follows the [numbering rule](README.md#streams). A number
  that breaks it is a [protocol violation](README.md#protocol-violations)
  that ends the link.
- Until the relay answers, the hub monitor sends nothing else on the stream
  except [Close stream](stream-close.md).

Owned by [#195](https://github.com/jackgeek/climon2/issues/195) and
[#196](https://github.com/jackgeek/climon2/issues/196).
