# Stream opened — `0x21`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements its layout and its order; no released program speaks it yet, and a broken stream rule still ends the whole link rather than the one stream.**
The [relay](../../../../GLOSSARY.md#monitor-links) reports that the client
asked for by [Open stream](stream-open.md) has started. See the
[overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Capabilities | `tokens` | The capability list from the client library's description, unchanged. |

## Example

A library that offers `status-v1`:

```text
21 00 00 00 01 00 00 00 09 73 74 61 74 75 73 2d
76 31
```

## Rules

- The [hub monitor](../../../../GLOSSARY.md#monitor-links) chooses required
  and optional [capabilities](../../../../GLOSSARY.md#sessions) from this
  list for its Open [request](request.md), as it does for a local session.
- These are the client library's capabilities for this session. They are
  separate from the [link capabilities](README.md#capabilities) in Hello.
- Stream opened proves the library loaded and the instance is running. It
  does not prove the session daemon was reached.
- After it, the hub monitor may send requests and the relay may send
  [Events](event.md) within the stream's credit.

Owned by [#196](https://github.com/jackgeek/climon2/issues/196).
