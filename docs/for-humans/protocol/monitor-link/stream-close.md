# Close stream — `0x23`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout and its order; no released program speaks it yet, and a broken stream rule still ends the whole link rather than the one stream.**
The [hub monitor](../../../../GLOSSARY.md#monitor-links) asks the
[relay](../../../../GLOSSARY.md#monitor-links) to stop a stream's client. See
the [overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

Empty. The record is the single tag [byte](../TERMS.md#byte):

```text
23
```

## Rules

- The relay [shuts the client instance down](../monitor-client/request-shutdown.md),
  which attempts a deliberate Leave, and answers with
  [Stream ended](stream-ended.md) carrying the run report.
- The hub monitor may send it at any time after
  [Open stream](stream-open.md), including before the relay has answered.
- After sending it, the hub monitor sends nothing more on the stream. It
  still receives records the relay sent before the Stream ended, and discards
  them.
- A stream that was refused, or has already ended, ignores a Close stream
  that crossed the ending on the wire.

Owned by [#195](https://github.com/jackgeek/climon2/issues/195).
