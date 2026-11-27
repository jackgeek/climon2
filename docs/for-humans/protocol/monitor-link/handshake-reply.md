# Handshake reply

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [relay](../../../../GLOSSARY.md#monitor-links) completes the pinned-key
handshake. It is the second and last message of the
[Noise](../TERMS.md#noise) pattern `IK` begun by the
[handshake initiation](handshake-initiation.md). See the
[overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Layout

| Field | Size | Meaning |
|---|---|---|
| Length | `u16` | Always 48. |
| Ephemeral key | 32 [bytes](../TERMS.md#byte) | A fresh X25519 public key, in the clear. |
| Payload | 16 bytes | An empty payload's authentication tag. |

## Rules

- A Length other than 48, or a message that does not decrypt, is a failed
  dial. The [hub monitor](../../../../GLOSSARY.md#monitor-links) redials with
  its ordinary backoff.
- A reply that decrypts proves the relay holds the identity key the hub
  monitor pinned.
- Both ends now split the Noise state into two cipher states, one for each
  direction. Everything that follows is [units](README.md#units).
- The hub monitor sends [Hello](hello.md) at once. The relay sends nothing
  further until it has received it.

Owned by [#480](https://github.com/jackgeek/climon2/issues/480).
