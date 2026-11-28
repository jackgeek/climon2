# Monitor link, link revision 1

What a [hub monitor](../../../../GLOSSARY.md#monitor-links) and a
[relay](../../../../GLOSSARY.md#monitor-links) send each other over a
[monitor link](../../../../GLOSSARY.md#monitor-links): the opening exchange,
pairing, the pinned-key handshake and the records of
[link revision](../../../../GLOSSARY.md#monitor-links) 1.

**Status: developer-agreed specification in
[#480](https://github.com/jackgeek/climon2/issues/480). The opening exchange,
pairing, the pinned-key handshake, Hello, Close and the framing of units are
implemented, in the `climon2-link` crate and in no released program.** No
climon2 release contains a relay or speaks any part of this protocol. The
[acceptance criteria](../../behaviour/monitor-link/acceptance.md) say what
automated evidence shows the protocol is met.

**Builds on:** [monitor-client](../monitor-client/README.md). A
[session stream](../../../../GLOSSARY.md#monitor-links) carries that
interface's requests and events, so the pages here link straight to the call
or record each one carries.

**Audience:** Integrators and Contributors.

## Shared terms

Use the [overview](../README.md), the
[domain glossary](../../../../GLOSSARY.md) and the
[technical terms](../TERMS.md) alongside every page. The words below are used
on every page of this folder.

- The **dialler** is the end that opens the [TCP](../TERMS.md#tcpip)
  connection. It is always the hub monitor. The relay listens and never dials.
- The **opening exchange** is the first message each way on every connection.
  It picks the link revision before anything is authenticated.
- A **pairing connection** exchanges and pins two public keys using a
  [link invitation](../../../../GLOSSARY.md#monitor-links). A **link
  connection** uses those pinned keys and carries everything else.
- A **static key** is a long-term key pair that identifies one end. The relay
  has one. The hub monitor has one for each relay it has paired with.
- A **unit** is one authenticated, encrypted message on a link connection.
- A **stream** is a numbered lane inside a link connection. Stream 0 carries
  control and advertisement. Every other stream is a session stream.
- A **record** is one message with a tag and a body. A record travels as one
  or more units of its stream.
- Text shown in backticks as a wire value, such as a marker or an info
  string, is [ASCII](../TERMS.md#ascii).
- [SHA-256 and SHA-512](../TERMS.md#sha-256-and-sha-512) are the hash
  functions used throughout.
- **Credit** is the number of [bytes](../TERMS.md#byte) of events the hub
  monitor is ready to receive on one session stream.

## Contents

Every connection starts with the opening exchange, frozen across all link
revisions:

| Message | Direction | Purpose |
|---|---|---|
| [Opening request](opening-request.md) | Hub monitor -> relay | State the purpose and the supported link revisions. |
| [Opening reply](opening-reply.md) | Relay -> hub monitor | Choose a revision, or refuse. |

A pairing connection then carries four messages and closes:

| Message | Direction | Purpose |
|---|---|---|
| [Pairing share](pairing-share.md) | Hub monitor -> relay, then relay -> hub monitor | Each end's public share of the PIN key exchange. |
| [Pairing confirmation](pairing-confirmation.md) | Hub monitor -> relay, then relay -> hub monitor | Prove knowledge of the PIN and hand over a static public key and a name. |

A link connection carries the handshake, then units:

| Message | Direction | Purpose |
|---|---|---|
| [Handshake initiation](handshake-initiation.md) | Hub monitor -> relay | Begin the pinned-key handshake. |
| [Handshake reply](handshake-reply.md) | Relay -> hub monitor | Complete the pinned-key handshake. |
| [Keepalive](keepalive.md) | Both | Show the sender is alive when it has nothing else to send. |

Records on stream 0:

| Record | Tag | Direction | Purpose |
|---|---|---|---|
| [Hello](hello.md) | `0x01` | Both | State release, name, capabilities and the largest record accepted. |
| [Close](close.md) | `0x02` | Both | End or refuse the link with a reason. |
| [Session](session.md) | `0x10` | Relay -> hub monitor | Advertise one session, new or changed. |
| [Session withdrawn](session-withdrawn.md) | `0x11` | Relay -> hub monitor | Withdraw a session proven stale. |
| [Snapshot complete](snapshot-complete.md) | `0x12` | Relay -> hub monitor | Mark the end of the first advertisement pass. |

Records on a session stream:

| Record | Tag | Direction | Purpose |
|---|---|---|---|
| [Open stream](stream-open.md) | `0x20` | Hub monitor -> relay | Ask the relay to start a client for one session. |
| [Stream opened](stream-opened.md) | `0x21` | Relay -> hub monitor | Report the client started, with its capabilities. |
| [Stream refused](stream-refused.md) | `0x22` | Relay -> hub monitor | Report the client could not start. |
| [Close stream](stream-close.md) | `0x23` | Hub monitor -> relay | Ask the relay to stop the client. |
| [Stream ended](stream-ended.md) | `0x24` | Relay -> hub monitor | Report the client's end. Always the stream's last record. |
| [Credit](credit.md) | `0x25` | Hub monitor -> relay | Grant more event bytes. |
| [Request refused](request-refused.md) | `0x26` | Relay -> hub monitor | Report a request the client library did not accept. |
| [Request](request.md) | `0x41` to `0x4b` | Hub monitor -> relay | One client interface request. |
| [Event](event.md) | `0x81` to `0x8e` | Relay -> hub monitor | One client interface event. |

## Connection lifecycle

1. The hub monitor connects. A connect that has not completed in 10 seconds
   is a failed dial.
2. The two ends run the opening exchange.
3. On a pairing connection they exchange two shares and two confirmations.
   The connection then closes. The hub monitor at once dials a link
   connection, and reports "paired" only when that link is established.
4. On a link connection they exchange the two handshake messages. Everything
   after that is units.
5. The hub monitor sends [Hello](hello.md). The relay answers with its own
   Hello, or refuses with [Close](close.md) alone.
6. The relay advertises its sessions. The hub monitor opens session streams.
7. Either end sends Close to end the link deliberately. A connection that
   ends without Close is a lost link.

The relay allows 30 seconds from accepting a connection to the end of a
pairing run, or to receiving the hub monitor's Hello. A connection that
passes the deadline is closed with no reply.

The relay allows at most 8 connections that have not yet completed pairing or
the handshake. It closes any further connection immediately.

## Cryptography

Link revision 1 fixes these choices. Changing any of them needs a new link
revision.

| Job | Choice |
|---|---|
| PIN key exchange | [CPace](../TERMS.md#cpace) on ristretto255 with SHA-512, as [`draft-irtf-cfrg-cpace-21`](https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-cpace-21) specifies it |
| Pairing key derivation and confirmation | [HKDF](../TERMS.md#hkdf) and [HMAC](../TERMS.md#hmac) with SHA-512 |
| Pinned-key handshake | [Noise](../TERMS.md#noise) pattern `IK`, named `Noise_IK_25519_ChaChaPoly_SHA256` |
| Units | Noise transport messages, ChaCha20-Poly1305 [AEAD](../TERMS.md#aead) |

The **transcript of the opening exchange** is the 26 bytes of the opening
request followed by the 20 bytes of the opening reply, exactly as sent. Both
key exchanges take it as input, so an attacker who edits either message makes
the key exchange fail and cannot force an older revision.

Neither end sends a public key, a name or a release version before the peer
is authenticated.

The `climon2-link` crate builds the handshake and the units on the `snow`
crate, pinned to one exact version. `snow` has had no formal audit; using it
is an accepted risk, recorded with the known-answer tests that guard it in
the [crate's README](../../../../crates/climon2-link/README.md#cryptography).

## Integers and byte strings

Every integer is unsigned and [big-endian](../TERMS.md#big-endian). `u8`,
`u16`, `u32` and `u64` name integers of 1, 2, 4 and 8 bytes.

Inside records:

- `bytes` is a `u32` length followed by that many bytes.
- `text` is `bytes` holding valid [UTF-8](../TERMS.md#unicode-and-utf-8).
- `tokens` is a `u32` count followed by that many `text` values.
- `error` is a `u32` origin, a `text` code and a `text` message, the three
  meaningful fields of the client interface's `cm1_error`. An absent error is
  origin 0 with both texts empty.

There is no padding, no alignment and no optional trailing field. A record
whose body is shorter or longer than its layout is a protocol violation.

## Units

After the handshake, each direction is a sequence of units. A unit on the
wire is:

| Field | Size | Meaning |
|---|---|---|
| Length | `u16` | Size of the ciphertext that follows, 21 to 16,384. |
| Ciphertext | Length bytes | One Noise transport message. |

The plaintext of a unit is:

| Field | Size | Meaning |
|---|---|---|
| Stream | `u32` | 0 for control and advertisement, otherwise a session stream. |
| Flags | `u8` | Bit 0 set means more of this record follows. Every other bit is zero. |
| Fragment | 0 to 16,363 bytes | Part of one record. |

Rules:

- A record is the fragments of its stream's units, in order, up to and
  including the first unit with bit 0 clear.
- Fragments of one record are never mixed with another record of the same
  stream. Units of other streams may be sent between them. This lets control
  and advertisement overtake a large event.
- A fragment is empty only in a [keepalive](keepalive.md).
- Each direction uses the Noise cipher state's own counter. A unit that does
  not decrypt ends the connection as a lost link. There is no rekeying.
- Receiving any unit that decrypts resets the receiver's 15-second silence
  timer. Bytes that have not yet formed a whole unit do not.

A reassembled record is:

| Field | Size | Meaning |
|---|---|---|
| Tag | `u8` | The record type, from the tables above. |
| Body | The rest | The layout on the record's page. |

## Streams

The hub monitor numbers session streams. The first is 1, each later one is
the previous number plus one, and a number is never used twice on one link
connection. A reconnect starts again from 1.

A session stream begins with [Open stream](stream-open.md) and ends with
[Stream ended](stream-ended.md) or [Stream refused](stream-refused.md). A
record on a stream that has not begun, or has ended, is ignored when it is
from the hub monitor and could have crossed the ending on the wire. Anything
else on such a stream is a protocol violation.

When the link ends, every stream ends with it. No Stream ended is sent.

## Flow control

[Events](event.md) are the only records that use
[credit](credit.md). An Event uses as many bytes of credit as its record is
long, tag included. The relay sends an Event only when the stream holds
enough credit for all of it, and never buffers more for a stream than the
credit it holds.

Every other record is small and is always sendable. Stream 0 never waits for
a session stream.

## Limits

| Limit | Value | When broken |
|---|---|---|
| Ciphertext of one unit | 16,384 bytes | Protocol violation. |
| Hello record | 4,096 bytes | Protocol violation. |
| Any other record | The receiver's Hello ceiling | Protocol violation. |
| Advertised sessions per relay | 128 | Protocol violation. |
| Queued stream-0 records | 1,048,576 bytes and 1,024 records | Protocol violation. |
| Credit held by one stream | 4,294,967,295 bytes | That stream ends. |
| Name | 64 bytes | Protocol violation. |
| Release version | 64 bytes | Protocol violation. |
| Capability tokens | 64 tokens of at most 64 bytes | Protocol violation. |

The stream-0 queue holds records an end has received and not yet processed.
An honest relay stays far below its bound, because it keeps at most one
pending advertisement change for each session.

These are release constants except the Hello ceiling, which each end takes
from its own configuration. The
[operational limits](../../behaviour/monitor/operational-limits.md) own the
configured values.

## Protocol violations

Each end treats everything it receives as hostile. An unknown tag, a
malformed body, a broken limit or a record sent in the wrong direction is a
protocol violation. It is never skipped.

- A violation that concerns one session stream's own records ends that
  stream. The relay sends Stream ended with the reason "stream violation".
  The hub monitor sends Close stream.
- Any other violation ends the link. The end that detects it sends
  [Close](close.md) with the reason "protocol violation" and closes the
  connection.

## Timings

These are part of link revision 1 and are the same in every release.

| Timing | Value |
|---|---|
| Opening request, from accept | 10 seconds |
| Opening reply, from sending the request | 10 seconds |
| Pairing run or handshake through Hello, from accept | 30 seconds |
| Keepalive interval | 5 seconds without sending |
| Silence threshold | 15 seconds without receiving a unit |

## Capabilities

Each Hello lists the sender's link capability tokens. A capability is in use
when both ends list it. Unknown tokens are ignored. Capabilities are the only
way to add a record without a new link revision. Link revision 1 publishes
none.

Link capabilities are separate from the client library's capabilities, which
[Stream opened](stream-opened.md) passes through for each session.

## Owning decisions

| Decision | Owner |
|---|---|
| The relay is a client of its own daemons; the link carries client interface records | [ADR 0009](../../../adr/0009-relay-terminates-daemon-wire.md) |
| Frozen link revisions and the opening exchange | [ADR 0010](../../../adr/0010-frozen-link-revisions-and-opening-exchange.md) |
| Threat model | [#192](https://github.com/jackgeek/climon2/issues/192) |
| Pairing ceremony | [#193](https://github.com/jackgeek/climon2/issues/193) |
| Advertisement and session streams | [#195](https://github.com/jackgeek/climon2/issues/195) |
| Refusals, cut and hold | [#197](https://github.com/jackgeek/climon2/issues/197) |
| What a lost link means for sessions | [#198](https://github.com/jackgeek/climon2/issues/198) |
| Stored pairings and forgetting | [#407](https://github.com/jackgeek/climon2/issues/407) |
| Limits | [#412](https://github.com/jackgeek/climon2/issues/412) |
| Keepalive and redial | [#477](https://github.com/jackgeek/climon2/issues/477) |
| Bytes, records and layouts | [#480](https://github.com/jackgeek/climon2/issues/480) |
