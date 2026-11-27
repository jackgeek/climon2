# Close — `0x02`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480), not implemented.**
Either end ends or refuses a [monitor link](../../../../GLOSSARY.md#monitor-links)
deliberately, giving the reason. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

## Body

| Field | Type | Meaning |
|---|---|---|
| Reason | `u16` | One of the codes below. |

There is no text. Each end logs its own detail.

## Reasons

Sent by the [relay](../../../../GLOSSARY.md#monitor-links):

| Code | Reason | The hub monitor then |
|---|---|---|
| 1 | Cut by the relay's user | Redials every 60 seconds. |
| 2 | Relay stopping | Redials with its ordinary backoff. |
| 3 | Relay restarting for an upgrade | Redials with its ordinary backoff. |
| 4 | Not paired | Stops redialling. The relay needs a new pairing. |
| 5 | On [hold](../../../../GLOSSARY.md#monitor-links) | Redials every 60 seconds. |
| 6 | Busy with another hub monitor | Redials every 60 seconds. |
| 7 | Replaced by a newly paired hub monitor | Redials every 60 seconds. |
| 9 | Superseded by your own new connection | Does nothing. Its newer connection is the link. |

Sent by the [hub monitor](../../../../GLOSSARY.md#monitor-links):

| Code | Reason | The relay then |
|---|---|---|
| 32 | Hub monitor disconnected by its user | Shows it and waits. |
| 33 | Forgotten by the hub monitor | Deletes its entry for that hub monitor. |
| 34 | Hub monitor stopped | Shows it and waits. |

Sent by either end:

| Code | Reason | Effect |
|---|---|---|
| 8 | Protocol violation | The hub monitor redials every 60 seconds. |

## Example

A relay on hold refuses a link:

```text
02 00 05
```

## Rules

- Close is the sender's last record. It closes the connection once the record
  is written.
- A link that ends without Close is a lost link. The hub monitor redials with
  its ordinary backoff.
- "Not paired" answers a hub monitor whose key the relay does not remember,
  and is also sent on a live link when the relay's user forgets that hub
  monitor. A hub monitor that was offline when it was forgotten learns it on
  its next dial.
- The relay sends a refusal (codes 4 to 7) only after the handshake and the
  hub monitor's [Hello](hello.md), in place of its own Hello. A probe that
  cannot complete the handshake learns nothing.
- A reason the receiver does not know, or one sent by the wrong end, is
  handled as code 8.
- However a link ends, the relay sends a deliberate
  [Leave](../monitor-client/request-detach.md) on every client it holds for
  the hub monitor, so no session ends because a link ended.

Owned by [#197](https://github.com/jackgeek/climon2/issues/197),
[#407](https://github.com/jackgeek/climon2/issues/407) and
[#477](https://github.com/jackgeek/climon2/issues/477). Codes 9 and 34 were
added in [#480](https://github.com/jackgeek/climon2/issues/480).
