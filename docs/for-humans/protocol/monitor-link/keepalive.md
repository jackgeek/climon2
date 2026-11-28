# Keepalive

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements it and the silence threshold; no released program speaks it yet.**
Either end of a [monitor link](../../../../GLOSSARY.md#monitor-links) sends a
keepalive to show it is alive when it has nothing else to send. See the
[overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Layout

A keepalive is a [unit](README.md#units), not a record. It has no tag. Its
plaintext is five zero [bytes](../TERMS.md#byte): stream 0, no flags and an
empty fragment.

```text
00 00 00 00 00
```

On the wire it is 23 bytes: the length `00 15` and 21 bytes of ciphertext.

## Rules

- An end sends a keepalive whenever it has sent no unit for 5 seconds.
- There is no reply. Nothing measures a round trip.
- An end that has received no unit for 15 seconds declares the link lost. Any
  unit that decrypts counts, not only a keepalive.
- A keepalive may arrive between the fragments of a record on a session
  stream. An end never sends one between the fragments of a stream-0 record.
  It has no need to, because it is already sending.
- Both timings are fixed by the link revision. There is no setting and no
  negotiation.

Owned by [#477](https://github.com/jackgeek/climon2/issues/477).
