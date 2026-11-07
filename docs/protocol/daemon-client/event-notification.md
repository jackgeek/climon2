# Notification — `0x44`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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
No [request ID](../TERMS.md#request-id)/defaults/unknown members. Invalid records/families follow
[validation](core-v1.md#framing-and-validation); required records must be
preserved or the affected connection explicitly failed under #69.
