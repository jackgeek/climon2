# Acquire — `0x33`

**Status: agreed; implemented for [attachments](../../../GLOSSARY.md#attachment-and-control) ([#87](https://github.com/jackgeek/climon2/issues/87)); refused on passive monitor connections until [#89](https://github.com/jackgeek/climon2/issues/89) serves them**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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

## Implemented behavior

On an interactive attachment the
[session daemon](../../../GLOSSARY.md#sessions) serves Acquire. A
non-controlling attachment takes control at once. The previous
[controller](../../../GLOSSARY.md#attachment-and-control), of either protocol, first
receives the [release](../../../GLOSSARY.md#restoration-and-presentation) of its
[presentation](../../../GLOSSARY.md#restoration-and-presentation), then a
[revocation](../../../GLOSSARY.md#attachment-and-control): a revision-1 one
gets [Control revoked](event-control-revoked.md) with reason `takeover`.
The requester receives [Control granted](event-control-granted.md) with
reason `acquire` and the [Restoration begin](event-restoration-begin.md) of
its first restoration, then the success with `changed:true` and that
[control period](../../../GLOSSARY.md#attachment-and-control) and
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts). The restoration's
[Display](event-display.md) chunks and
[Restoration complete](event-restoration-complete.md) follow the success, as
the connection drains. The controller's own Acquire is an explicit no-op:
`changed:false` with its current period and restoration ID, and nothing
else. During committed termination Acquire is `session_terminating`.

On a [passive monitor connection](../../../GLOSSARY.md#attachment-and-control)
Acquire is rejected without effects with `internal_error`, the interim code
until #89 converts a registration into an attachment: no control is
acquired and the registration stays. Before any Open there is nothing to
convert, so Acquire is `invalid_state`, like an early
[Leave](request-leave.md). A rejection never ends the stream.
