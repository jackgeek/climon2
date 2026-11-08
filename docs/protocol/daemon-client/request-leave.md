# Leave — `0x37`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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
deliberate or lost, can first hand back to the
most recently controlling attachment, otherwise promote the newest accepted
passive registration, before final-participant leave policy.
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
final acceptance #62 remains open. Related: [Kill](request-kill.md).
