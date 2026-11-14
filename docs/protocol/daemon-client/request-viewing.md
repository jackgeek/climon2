# Set viewing — `0x35`

**Status: agreed, not implemented; rejected until served**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) reports per-[attachment](../../../GLOSSARY.md#attachment-and-control)
[viewing](../../../GLOSSARY.md#status-and-attention) to the
[session daemon](../../../GLOSSARY.md#sessions), as current
[controller](../../../GLOSSARY.md#attachment-and-control).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`request_id`, `period`, `restoration_id` are
[Counters](core-v1.md#shared-types); `viewing` is boolean.

```json
{"request_id":"6","period":"1","restoration_id":"1","viewing":true}
```

## Acceptance, order and failures

Core support is required. Validate current period/
[restoration](../../../GLOSSARY.md#restoration-and-presentation).
True additionally requires [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts);
false may clear viewing during restoration.
Success [Response](response.md) result is
`{"kind":"viewing","changed":<bool>,"viewing":<bool>}`.
This is a fact report, not [Acknowledge](request-acknowledge.md), takeover,
input, resize or consumption.

Menus clear this [monitor](../../../GLOSSARY.md#sessions)'s viewing without releasing control/restoration;
switching away, disconnect and
[revocation](../../../GLOSSARY.md#attachment-and-control) also clear it.
The [terminal contract](../terminal-integration.md) adds live viewport,
reportable host focus, and unobstructed painted usable space. Scrolled history,
too-small/unavailable canvas, stale inspection, or notification coverage clears
viewing. Nonmodal toasts and scrollback retain keyboard focus; modal menus do not.
Host focus loss retains control/readiness; focus-in or hiding a notification
never automatically acknowledges. Without reports, host focus is best-effort
assumed, not detected. Application [Focus](request-focus.md) is separate.
No aggregate viewing field/broadcast exists. Another monitor's viewing never
suppresses this monitor's notifications. See
[local notification rules](core-v1.md#viewing-acknowledgement-and-local-notifications).

Stale IDs, missing attachment/control/readiness, termination and operational
failures use [explicit errors](core-v1.md#errors-and-bounded-delivery).
No optional fields/defaults/unknown members; malformed records/IDs are fatal.
Frame bounds remain #69. Related: [Ready](request-ready.md).

## Implemented behavior

Until [#91](https://github.com/jackgeek/climon2/issues/91) serves Set viewing, the
[session daemon](../../../GLOSSARY.md#sessions) rejects it as impermissible
with `invalid_state`, before the connection has a role, on a
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) and
on an interactive [attachment](../../../GLOSSARY.md#attachment-and-control)
alike.
No viewing is reported; the stream goes on and any role stays.
