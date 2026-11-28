# Opening reply

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements it; no released program speaks it yet.**
The [relay](../../../../GLOSSARY.md#monitor-links) answers a valid
[opening request](opening-request.md) with the
[link revision](../../../../GLOSSARY.md#monitor-links) to use, or with a
refusal. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

This message is frozen forever, as the opening request is.

## Layout

Exactly 20 [bytes](../TERMS.md#byte), in the clear:

| Field | Size | Meaning |
|---|---|---|
| Result | `u8` | 1 chosen, 2 no shared revision, 3 pairing unavailable. |
| Chosen | `u16` | The revision to use when Result is 1. Zero otherwise. |
| Count | `u8` | How many revisions follow, 1 to 8. |
| Revisions | 8 × `u16` | The relay's supported revisions, in the same form as the request's. |

## Results

- **1, chosen.** Chosen is the highest revision in both lists. The connection
  continues in that revision: [pairing](pairing-share.md) for purpose 1, the
  [handshake](handshake-initiation.md) for purpose 2.
- **2, no shared revision.** The relay closes the connection. Both ends now
  hold both lists, and each tells its user which machine must upgrade. The
  [hub monitor](../../../../GLOSSARY.md#monitor-links) shows the link as
  incompatible and redials every 60 seconds. A pairing is kept.
- **3, pairing unavailable.** Sent only for purpose 1, when the relay has no
  live [link invitation](../../../../GLOSSARY.md#monitor-links) or another
  pairing run is in flight. The relay closes the connection. No attempt is
  counted.

## Examples

Revision 1 chosen:

```text
01 00 01 01 00 01 00 00 00 00 00 00 00 00 00 00
00 00 00 00
```

Refused by a relay that supports only revisions 2 and 3:

```text
02 00 00 02 00 02 00 03 00 00 00 00 00 00 00 00
00 00 00 00
```

## Rules

- The hub monitor waits 10 seconds for all 20 bytes, then treats the dial as
  failed.
- A reply that breaks the layout, or names a revision the hub monitor did not
  offer, is a failed dial.
- The relay sends nothing after the reply until the hub monitor sends its
  next message, whatever the purpose: the relay's
  [pairing share](pairing-share.md) answers the hub monitor's, and the
  [handshake reply](handshake-reply.md) answers the initiation. Bytes that
  arrive with the reply, behind its 20th byte, break the layout, and the dial
  fails before the hub monitor sends anything more.
- The reply is not authenticated, so someone on the path can forge a refusal.
  That gains them only the denial of service they already have. A forged
  "chosen" makes the following key exchange fail, because both messages are
  part of the [opening transcript](README.md#cryptography).
- A refusal during pairing does not use up the invitation or one of its three
  attempts.

Owned by [ADR 0010](../../../adr/0010-frozen-link-revisions-and-opening-exchange.md)
and [#193](https://github.com/jackgeek/climon2/issues/193) for the pairing
results.
