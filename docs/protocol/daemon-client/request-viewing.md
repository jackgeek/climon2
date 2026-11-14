# Set viewing — `0x35`

**Status: agreed and implemented** ([#91](https://github.com/jackgeek/climon2/issues/91)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
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

Core support is required, and the session daemon also requires selected
`status-v1` (owner decision on [#86](https://github.com/jackgeek/climon2/issues/86); see
[implemented behavior](#implemented-behavior)). Validate current period/
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

The [session daemon](../../../GLOSSARY.md#sessions) serves Set viewing with
the [implemented authority checks](core-v1.md#implemented-authority-checks).
By owner decision on [#86](https://github.com/jackgeek/climon2/issues/86)
(2026-11-13), the connection's Open must have selected `status-v1`, checked
first: otherwise `unsupported_capability`, before any Open too. Viewing true
needs readiness; viewing false needs only control, so it is accepted during
restoration. `changed` says whether the latest reported viewing of the
current [control period](../../../GLOSSARY.md#attachment-and-control)
changed. Every new period, by grant, takeover or hand-back, starts not
viewing, so revocation clears it. Viewing never acknowledges or clears
[attention](../../../GLOSSARY.md#status-and-attention) and sends no
[Status changed](event-status-changed.md).
