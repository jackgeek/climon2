# Leave — `0x37`

**Status: agreed and implemented** for
[passive monitor connections](../../../GLOSSARY.md#attachment-and-control)
([#86](https://github.com/jackgeek/climon2/issues/86)) and
[attachments](../../../GLOSSARY.md#attachment-and-control)
([#87](https://github.com/jackgeek/climon2/issues/87)), owned by [#67](https://github.com/jackgeek/climon2/issues/67);
promotion races arrive with [#89](https://github.com/jackgeek/climon2/issues/89).
The [client](../../../GLOSSARY.md#sessions) requests accepted departure from the
[session daemon](../../../GLOSSARY.md#sessions) after Open.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with sole required
field `request_id: Counter`:

```json
{"request_id":"8"}
```

## Acceptance, succession and failures

Core support is required; Leave remains allowed during committed termination.
Remove the departing connection's registration before succession, including
promotion races. If attached, remove its
[attachment](../../../GLOSSARY.md#attachment-and-control) membership.
Success [Response](response.md) result is
`{"kind":"left","attachment_detached":<bool>}`; close the stream after response.
False distinguishes passive departure from an accepted
[detach](../../../GLOSSARY.md#attachment-and-control).
Local closure or a lost response does not prove deliberate detach.
Neither success nor closure proves client cleanup or a joined thread.

Every [controller](../../../GLOSSARY.md#attachment-and-control) departure,
deliberate or lost, can first [hand back](../../../GLOSSARY.md#attachment-and-control) to the
most recently controlling attachment, otherwise promote the newest accepted
passive registration, before final-participant
[leave policy](../../../GLOSSARY.md#attachment-and-control).
Committed termination overrides [succession](core-v1.md#membership-and-succession).
Switching [monitor](../../../GLOSSARY.md#sessions) selection is not Leave;
inherited control does not select,
resize, send input or acknowledge.

Invalid state and resource/I/O/internal failures use the shared
[error catalog](core-v1.md#errors-and-bounded-delivery).
No optional fields/defaults/unknown members; malformed records or IDs are fatal.
The [agreed, unimplemented lifecycle](../../monitor-lifecycle.md#coordinated-shutdown)
uses atomic Leave during concurrent shutdown of passive, attached, partially
started and failing owned clients. Promotion racing with shutdown cannot leave
membership registered. Record accepted passive deregistration/deliberate detach
separately from closure, final client outcome, worker cleanup and thread join.
A lost response stays unconfirmed/indeterminate and is not replayed.
Stalled cleanup uses bounded whole-process failure, not false Leave/join success.
[Numerical limits/deadlines](../../monitor-operational-limits.md) and
[native declarations](../monitor-client/core-v1.md) are agreed, unimplemented;
the [acceptance matrix](../../monitor-acceptance.md) is agreed in #62,
not executed conformance evidence. Related: [Kill](request-kill.md).

## Implemented behavior

On a [passive monitor connection](../../../GLOSSARY.md#attachment-and-control)
the [session daemon](../../../GLOSSARY.md#sessions) removes the registration
in the same step that queues the success Response
`{"kind":"left","attachment_detached":false}`, then closes the stream once that
Response is written; requests sent after Leave are not served, and no core
metadata follows it. Leave on a connection that opened nothing, before any
Open or on a standalone [Kill](request-kill.md#implemented-behavior)'s
connection, is `invalid_state` and the stream goes on. A peer may half-close
after sending Leave and still read its Response.

An interactive attachment's Leave is an accepted detach. A controller first
receives the [release](../../../GLOSSARY.md#restoration-and-presentation) of its
[presentation](../../../GLOSSARY.md#restoration-and-presentation) as
[Display](event-display.md), then control is
[handed back](../../../GLOSSARY.md#attachment-and-control) to the most recently
controlling remaining attachment, of either protocol, with reason
`hand_back`. The success reports `attachment_detached:true`, and the stream
then closes. The [leave policy](../../../GLOSSARY.md#attachment-and-control) is
the private protocol's: a final deliberate Leave keeps the session running,
detached, while a final unexpected departure, closing without Leave, ends
it. During committed termination Leave still removes the attachment, after
releasing a presentation it still has, and hands nothing back.
