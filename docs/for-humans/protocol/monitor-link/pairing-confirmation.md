# Pairing confirmation

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements it; no released program speaks it yet.**
The last two messages of a pairing connection. Each end proves it derived the
same key from the PIN, and hands over its static public key and a suggested
name. See the [overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md) and the [shared rules](README.md).

## Keys

Both ends hold the 64-[byte](../TERMS.md#byte) key that
[CPace](../TERMS.md#cpace) produced from the two
[pairing shares](pairing-share.md). Each derives four 32-byte keys from it
with [HKDF](../TERMS.md#hkdf) using SHA-512, an empty salt and these ASCII
info strings:

| Key | Info string |
|---|---|
| Hub confirmation key | `climon2 link pairing hub confirm` |
| Relay confirmation key | `climon2 link pairing relay confirm` |
| Hub box key | `climon2 link pairing hub box` |
| Relay box key | `climon2 link pairing relay box` |

## Layout

The [hub monitor](../../../../GLOSSARY.md#monitor-links) sends its
confirmation first. The [relay](../../../../GLOSSARY.md#monitor-links)
answers with its own. Both have the same layout:

| Field | Size | Meaning |
|---|---|---|
| Length | `u16` | Size of the rest, 113 to 177. |
| Confirmation | 64 bytes | [HMAC](../TERMS.md#hmac) with SHA-512 under the sender's confirmation key, over the [opening transcript](README.md#cryptography), then the hub monitor's share, then the relay's share. Each share is the 32-byte Share field of its [pairing share](pairing-share.md), without the Length before it. |
| Box | 49 to 113 bytes | The sender's key and name, encrypted. |

The box is ChaCha20-Poly1305 [AEAD](../TERMS.md#aead) under the sender's box
key, with a nonce of twelve zero bytes and the Confirmation field as
associated data. Each box key is used once. Its plaintext is:

| Field | Size | Meaning |
|---|---|---|
| Static public key | 32 bytes | The sender's X25519 public key for the [pinned-key handshake](handshake-initiation.md). |
| Name length | `u8` | 0 to 64. |
| Name | Name length bytes | A suggested display name, [UTF-8](../TERMS.md#unicode-and-utf-8), the hostname by default. |

The hub monitor sends a key pair it made for this pairing alone. The relay
sends its one identity key.

## Rules

- A Length outside 113 to 177 ends the connection as soon as it arrives,
  before the bytes behind it. At the relay the attempt counts.
- The relay checks the hub monitor's Confirmation and opens its box before it
  sends anything. If either fails, it closes the connection with no reply and
  the attempt counts. A dialler that does not know the PIN therefore never
  receives a message it can test a guess against.
- On success the relay pins the hub monitor's key, uses up the invitation and
  sends its own confirmation.
- The hub monitor checks the relay's Confirmation and opens its box. If
  either fails, pairing has failed and it keeps nothing.
- The connection closes after the relay's confirmation. No session traffic
  ever runs on a pairing connection. Bytes that arrive behind the last
  message an end reads are ignored, because that end then closes.
- The hub monitor then dials a link connection at once. It reports "paired"
  only when that link's [Hello](hello.md) exchange succeeds. If it never gets
  that far, the relay keeps an entry that its user can forget.
- A name is a label and never an identity. Each end treats the other's name
  as hostile text.

Owned by [#193](https://github.com/jackgeek/climon2/issues/193).
