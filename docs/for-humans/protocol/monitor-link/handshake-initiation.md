# Handshake initiation

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The [hub monitor](../../../../GLOSSARY.md#monitor-links) begins the
pinned-key handshake on a link connection. It is the first message of the
[Noise](../TERMS.md#noise) pattern `IK`. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Layout

| Field | Size | Meaning |
|---|---|---|
| Length | `u16` | Always 96. |
| Ephemeral key | 32 [bytes](../TERMS.md#byte) | A fresh X25519 public key, in the clear. |
| Static key | 48 bytes | The hub monitor's static public key for this [relay](../../../../GLOSSARY.md#monitor-links), encrypted to the relay's pinned key. |
| Payload | 16 bytes | An empty payload's authentication tag. |

## Noise parameters

| Parameter | Value |
|---|---|
| Protocol name | `Noise_IK_25519_ChaChaPoly_SHA256` |
| Initiator | The hub monitor, with its static key for this relay. |
| Responder | The relay, with its identity key. |
| Prologue | The [opening transcript](README.md#cryptography). |
| Payloads | Empty in both handshake messages. |

## Rules

- The message carries nothing in the clear that identifies the hub monitor.
  The relay decrypts the static key and looks it up among its remembered hub
  monitors.
- A Length other than 96, or a message that does not decrypt, ends the
  connection with no reply. This is what a hub monitor meets when the relay's
  identity key has changed.
- The relay sends a [handshake reply](handshake-reply.md) to any message that
  decrypts, whether or not it remembers the key. The decision to accept or
  refuse comes later, inside the encrypted link, in answer to
  [Hello](hello.md).
- This message can be replayed by anyone who recorded it. The relay therefore
  treats nothing as linked, and replaces no existing link, until it has
  received the hub monitor's Hello. A replayer cannot produce that.

Owned by [#480](https://github.com/jackgeek/climon2/issues/480) and the
threat model in [#192](https://github.com/jackgeek/climon2/issues/192).
