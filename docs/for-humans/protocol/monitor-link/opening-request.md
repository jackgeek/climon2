# Opening request

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [hub monitor](../../../../GLOSSARY.md#monitor-links) sends this as the
first [bytes](../TERMS.md#byte) of every connection to a
[relay](../../../../GLOSSARY.md#monitor-links). It states why the hub monitor
is connecting and which [link revisions](../../../../GLOSSARY.md#monitor-links)
it supports. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

This message is frozen forever. No link revision can change it, and it has no
room for extension.

## Layout

Exactly 26 bytes, in the clear:

| Field | Size | Meaning |
|---|---|---|
| Marker | 8 bytes | `63 32 6c 69 6e 6b 0d 0a`, the ASCII text `c2link` and a carriage return and line feed. |
| Purpose | `u8` | 1 to pair, 2 to link. |
| Count | `u8` | How many revisions follow, 1 to 8. |
| Revisions | 8 × `u16` | The supported revisions in the first Count slots, ascending, with no zero and no repeat. Every unused slot is zero. |

It carries nothing else: no release version, no name and no key.

## Example

A hub monitor that wants to link and supports only link revision 1:

```text
63 32 6c 69 6e 6b 0d 0a 02 01 00 01 00 00 00 00
00 00 00 00 00 00 00 00 00 00
```

## Rules

- All 26 bytes must reach the relay within 10 seconds of the relay accepting
  the connection.
- The relay closes the connection without sending a byte when the message is
  late or breaks the layout in any way. Someone probing the port learns only
  that it is open.
- A valid request gets an [opening reply](opening-reply.md).
- The request proves nothing about the sender. It is bound into the key
  exchange that follows as part of the
  [opening transcript](README.md#cryptography).

Owned by [ADR 0010](../../../adr/0010-frozen-link-revisions-and-opening-exchange.md).
