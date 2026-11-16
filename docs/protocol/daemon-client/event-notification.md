# Notification — `0x44`

**Status: agreed and implemented** ([#98](https://github.com/jackgeek/climon2/issues/98)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) supplies normalized
notification metadata to a subscribed [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One bounded [JSON](../TERMS.md#json) [frame](../TERMS.md#frame).
All fields required:

| Field | Type/value |
|---|---|
| `source` | `osc9`, `osc99`, or `osc777`. |
| `source_id` | String or null; source identity, not [attention](../../../GLOSSARY.md#status-and-attention) identity. |
| `title` | String or null. |
| `body` | String or null. |
| `episode_id` | Counter or null. |

```json
{"source":"osc777","source_id":null,"title":"","body":"Build ready; inspect results","episode_id":"2"}
```

## Source, order and guarantees

Requires selected `notification-v1` **and** `status-v1`.
Follow [initial synchronization](event-initial-metadata.md); delivery remains
independent of control/[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
including revoked subscribers.
Recognized logical notifications request
[attention](../../../GLOSSARY.md#status-and-attention), but repeated requests
while pending coalesce; metadata is not another toast entitlement.
No payload archive/replay or full desktop-notification protocol is promised.

Apply the exact [source normalization and primary references](core-v1.md#source-normalization):
[OSC](../TERMS.md#osc) 9 text, completed OSC 99 title/body chunks, and the climon2 OSC 777
first-delimiter subset. Title/body are inert [Unicode](../TERMS.md#unicode-and-utf-8) with chosen replacement
at the text boundary, never raw OSC, executable actions or terminal encoding.
Unsupported source controls/queries do not invent notifications.

Episode identity is separate from source ID and [UTC](../TERMS.md#utc) time.
[Local toast rules](core-v1.md#viewing-acknowledgement-and-local-notifications)
deduplicate per [monitor](../../../GLOSSARY.md#sessions) lifetime,
including reconnects; no screen mirroring,
takeover or consumption follows delivery.
The [#63 terminal rules](../terminal-integration.md#mouse-gestures-and-notifications)
allow nonmodal toast coverage without stealing ready keyboard input or an existing
application drag; new toast clicks dismiss without takeover. Coverage clears
viewing, disappearance never auto-acknowledges. Needs-attention hints use the
actual prefix plus lowercase j for a dynamic most-recent jump, not a fixed toast
target; planned show_key_hints hides all implicit monitor hints.
No [request ID](../TERMS.md#request-id)/defaults/unknown members. Invalid records/families follow
[validation](core-v1.md#framing-and-validation); required records must be
preserved or the affected connection explicitly failed under #69.

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) recognizes notifications
in the command's output before filtering, and none reaches
[host](../../../GLOSSARY.md#sessions) output:

- [OSC](../TERMS.md#osc) 9 text, `ESC ] 9 ; text ST`: a notification with a
  null title and the whole text, later semicolons included, as its body. A
  first parameter of decimal digits (`9;4;…` progress, `9;1;…` and the other
  ConEmu control operations) is no notification.
- OSC 777 `notify`, `ESC ] 777 ; notify ; title [; body] ST`: the title up to
  the first `;` and the rest, if any, as the body; a missing body is null.
  Other OSC 777 commands are no notification.
- OSC 99, `ESC ] 99 ; metadata ; payload ST`: metadata keys `i` (identifier,
  `[a-zA-Z0-9-_+.]`, the record's `source_id`), `d` (`0` incomplete, `1`
  complete, the default), `p` (`title`, the default, `body`, or auxiliary
  `icon` and `buttons`) and `e` (`1` for a base64 payload). Chunks with the
  same identifier, or none, concatenate their title and body bytes until a
  completing chunk; auxiliary data completes a notification that has text but
  never makes one. Queries (`p=?`), `close` and `alive` are ignored, as are
  chunks with an invalid value, identifier or base64 payload, an unknown
  payload type or no payload; unknown keys are ignored.

Text is made [UTF-8](../TERMS.md#unicode-and-utf-8), invalid bytes replaced
by U+FFFD. A notification whose title and body are both missing or empty is
ignored. So is one whose title exceeds 4,096 UTF-8 bytes, whose body exceeds
8,192, or whose record, measured with the longest episode identifier, would
exceed the 16,384-byte metadata record bound: it is never truncated. These
bounds are release constants.

Incomplete OSC 99 notifications are bounded by the session's notification
assembly limits, which `exec` passes to the daemon from its validated
configuration and which stay fixed for the session's lifetime:
`payload.assembly_records` incomplete notifications at once, holding at most
`payload.assembly_bytes` of text together, each for `deadlines.assembly_ms`
from its first chunk (see the
[operational limits](../../monitor-operational-limits.md#payloads-and-parser-state)).
A chunk that would open one notification too many is dropped; a chunk that
would pass a text bound, or that an incomplete notification cannot hold within
the byte budget, discards that notification, whose later chunks complete
nothing; at the end of its lifetime an incomplete notification expires. Each
logs a diagnostic, and none completes a notification. A completing chunk is
not held, so only the text bounds apply to it.

Each recognized notification requests
[attention](../../../GLOSSARY.md#status-and-attention) first. When that
changes the effective status, its [Status changed](event-status-changed.md)
comes before the Notification, and `episode_id` is the episode then pending:
a new one, or the pending one it joined. It is null when no attention is
pending, after the command's final result for example. The Notification then
goes, in the order the program wrote it, to every connection that selected
`notification-v1`, whatever its control: passive, controlling in the
background or revoked. An Open selects `notification-v1` only with
`status-v1`: optional without it, it is not selected; required without it,
the Open is rejected with `unsupported_capability`.
