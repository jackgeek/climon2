# Hello — `0x01`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
The first record each way on stream 0 of a link connection. Each end states
its release, its name, its link [capabilities](README.md#capabilities) and
the largest record it accepts. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Release | `text`, 1 to 64 [bytes](../TERMS.md#byte) | The sender's climon2 release version. |
| Name | `text`, 0 to 64 bytes | The sender's suggested display name, its hostname by default. |
| Capabilities | `tokens`, at most 64 of at most 64 bytes | The sender's link capability tokens. Empty in link revision 1. |
| Largest record | `u32`, at least 4,096 | The longest record, tag included, that the sender accepts. |

A Hello record is at most 4,096 bytes.

## Example

Release `2.4.0`, name `atlas`, no capabilities, records up to 1,048,576
bytes:

```text
01 00 00 00 05 32 2e 34 2e 30 00 00 00 05 61 74
6c 61 73 00 00 00 00 00 10 00 00
```

## Order

1. The [hub monitor](../../../../GLOSSARY.md#monitor-links) sends Hello as
   its first record, straight after the
   [handshake reply](handshake-reply.md).
2. The [relay](../../../../GLOSSARY.md#monitor-links) checks that the Hello
   is valid and decides whether to accept the link.
3. To accept, the relay sends its own Hello. The link is established. A link
   this hub monitor already had on another connection is closed with the
   reason "superseded by your own new connection".
4. To refuse, the relay sends [Close](close.md) and no Hello, so a refused
   caller learns no name and no release.

The hub monitor's Hello is what defeats a replayed handshake initiation.
Until the relay has received it, the relay changes nothing.

## Rules

- No behaviour branches on Release. It is for display and diagnostics.
- The relay stores the hub monitor's Name as that hub monitor's suggested
  name, replacing the one it held. The hub monitor ignores the relay's Name
  once its user has named the relay.
- Each end treats the other's Release and Name as hostile text.
- Largest record is a receive ceiling taken from the sender's own
  `payload.wire_frame_bytes`. It does not limit what the sender itself may
  send. A record longer than the receiver's ceiling is a
  [protocol violation](README.md#protocol-violations), so an honest sender
  never sends one.
- Any record before Hello, or a second Hello, is a protocol violation.

Owned by [#480](https://github.com/jackgeek/climon2/issues/480). Release
exchange is owned by [#196](https://github.com/jackgeek/climon2/issues/196).
The receive ceiling sharpens "neither end sends limits to the other" in
[#412](https://github.com/jackgeek/climon2/issues/412): each machine still
governs its own resources.
