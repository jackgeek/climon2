# Event envelope

**Status:** agreed, not implemented; individual [event](../TERMS.md#event) [schemas](../TERMS.md#schema) may be pending.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Core events originate at the [helper](../../../GLOSSARY.md). Negotiated [extension](../../../GLOSSARY.md) events can travel in
either direction.

| Required field | Type | Meaning |
|---|---|---|
| `kind` | String, exactly `"event"` | [Envelope](../TERMS.md#envelope) [discriminator](../TERMS.md#discriminator). |
| `event` | String | Event name. |
| `data` | Object | Event-specific fields. |

Optional binary data follows the shared
[header/payload rules](../README.md#binary-payloads).
Fields identifying [control periods](../../../GLOSSARY.md) and [restorations](../../../GLOSSARY.md) must follow their event
schema; they are not [request IDs](../TERMS.md#request-id).

## Example

A complete example for a negotiated attention extension:

```json
{"kind":"event","event":"extension","data":{"name":"org.example.attention","revision":1,"event":"status_changed","payload":{"status":"waiting_for_user"}}}
```

This is [plugin](../../../GLOSSARY.md)-defined status information, not a core [readiness](../../../GLOSSARY.md) or control event.
Its meaning requires that exact extension/revision to be negotiated.

Core events preserve control/restoration ordering. An impossible transition,
unknown core event, or unnegotiated extension event fails the channel. Essential
core events cannot silently disappear through plugin filtering.

Some accepted event meanings have no assigned wire name or full `data` schema
yet. Their pages use descriptive filenames and state that gap explicitly.
These missing definitions are tracked by
[schema completion](https://github.com/jackgeek/climon2/issues/71).

**Source:** [helper resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
