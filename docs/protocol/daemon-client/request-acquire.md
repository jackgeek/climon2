# Acquire — `0x33`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) explicitly requests
[control](../../../GLOSSARY.md#attachment-and-control) from the
[session daemon](../../../GLOSSARY.md#sessions) after Open.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with the sole required
field `request_id: Counter`:

```json
{"request_id":"4"}
```

## Acceptance, order and failures

Core support is required. Acquisition converts a passive
[monitor](../../../GLOSSARY.md#sessions) to an ordinary
[attachment](../../../GLOSSARY.md#attachment-and-control), if not already one.
Last accepted takeover wins.
[Grant](event-control-granted.md) and
[restoration](../../../GLOSSARY.md#restoration-and-presentation) precede a changed success
[Response](response.md):
`{"kind":"acquired","changed":<bool>,"period":<Counter>,"restoration_id":<Counter>}`.
Already controlling means `changed:false`, retaining authority with no new
grant/restoration. No implicit geometry change or
[attention](../../../GLOSSARY.md#status-and-attention) acknowledgement is
included. Acceptance is not
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).

Discovery, menu browsing and passive reconnect do not originate Acquire.
See [succession](core-v1.md#membership-and-succession) for promotion without
selection, input, resize or acknowledgement.
Invalid state, terminating session and resource/I/O failures use
[explicit errors](core-v1.md#errors-and-bounded-delivery).
No optional fields/defaults or unknown members; malformed records/bad IDs are
fatal. Numeric bounds belong to #69.
Related: [Acknowledge](request-acknowledge.md), [Ready](request-ready.md).
