# Focus — `0x39`

**Status: agreed, unimplemented #63 amendment to wire revision 1; rejected until served.**
The [client](../../../GLOSSARY.md#sessions) reports desired application focus to
the [session daemon](../../../GLOSSARY.md#sessions), as the current ready
[controller](../../../GLOSSARY.md#attachment-and-control).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), inherited [wire rules](core-v1.md), and
[terminal contract](../terminal-integration.md#host-focus-and-application-focus).
This is a dedicated non-user core operation, not a
[plugin extension](../../../GLOSSARY.md#protocol-and-plugin-concepts) or Input.

## Body, support, and example

One bounded [UTF-8](../TERMS.md#unicode-and-utf-8)
[JSON](../TERMS.md#json) [frame](../TERMS.md#frame).
All fields are required; no defaults, optional fields, or unknown members:

| Field | Type/value |
|---|---|
| `request_id` | [Counter](core-v1.md#shared-types), strictly increasing on this connection. |
| `period` | Counter for the current [control period](../../../GLOSSARY.md#attachment-and-control). |
| `restoration_id` | Counter for the current [restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts). |
| `focused` | Boolean desired application keyboard focus. |

```json
{"request_id":"8","period":"2","restoration_id":"3","focused":false}
```

`0x39` is the next unused [request](../TERMS.md#request) code after `0x38` Kill;
it does not change
existing codes. Require selected optional core
[capability](../../../GLOSSARY.md#sessions) `focus-v1`, advertised
through the existing additive [hello](../../../GLOSSARY.md#sessions) capability
array and selected by [Open](request-open.md).
There is no new hello field, release-number inference, or extension negotiation.
Absent library/daemon support is an explicit focus limitation, not a reason to
make otherwise usable sessions unavailable. Bounds/deadlines remain
[#69](https://github.com/jackgeek/climon2/issues/69).

## State, ordering, and truthful response

Validate current [attachment](../../../GLOSSARY.md#attachment-and-control),
controller, period, [restoration](../../../GLOSSARY.md#restoration-and-presentation), and
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) at execution.
A retained background controller may report `focused:false` when switching away;
this is not permission for background application input or resize.
Neither selection nor [viewing](../../../GLOSSARY.md#status-and-attention) true is required for focus-out.
The [monitor](../../../GLOSSARY.md#sessions) sends focus-in only for its selected
ready [host](../../../GLOSSARY.md#sessions)-focused keyboard target without modal
[UI](../TERMS.md#ui).

Remember desired state for the current attachment even when application
focus-reporting mode is disabled. When mode is enabled and state is known, queue
exactly `ESC [ I` for true or `ESC [ O` for false.
Repeated state does not duplicate [bytes](../TERMS.md#byte). Enabling reporting later queues current
known state once; no unknown state is invented. New authority does not inherit
or replay old requests. Restoration retains latest local desired focus; a newly
ready monitor submits its current state with current authority.

Success [Response](response.md) result is:
`{"kind":"focus","changed":<bool>,"focused":<bool>,"accepted_bytes":<u32>}`.
`changed` means desired state changed (unknown to known counts as changed).
`focused` is the accepted state. `accepted_bytes` is 0 when no sequence was
needed or reporting was disabled, or 3 when the complete matching sequence was
accepted for forwarding. These bytes are non-user traffic. The response proves
state acceptance and the stated forwarding acceptance, **not command consumption
or a [PTY](../TERMS.md#pseudo-terminal-pty) read**. It never changes
[attention](../../../GLOSSARY.md#status-and-attention) or reports viewing.
Automatic emission on later mode enable is not a second response to this request.

```json
{"request_id":"8","outcome":"ok","result":{"kind":"focus","changed":true,"focused":false,"accepted_bytes":0},"error":null}
```

This example accepts focus-out with reporting disabled.
With reporting enabled, a changed focus-out instead accepts the three bytes
`1b 5b 4f`; repeated unchanged focus-out accepts zero bytes.
No binary payload is supplied by the caller.

An identifiable Focus request without selected support is explicitly rejected:

```json
{"request_id":"8","outcome":"rejected","result":null,"error":{"code":"unsupported_capability","message":"focus-v1 was not selected"}}
```

## Rejection and failure

Use existing operational codes: `unsupported_capability`, `not_attached`,
`not_controller`, `stale_period`, `stale_restoration`, `not_ready`,
`invalid_state`, `session_terminating`, `resource_limit`, `io_error`, or
`internal_error`, as applicable. No new error code is needed.
Wrong types/members, malformed framing/IDs and wrong direction follow
[fatal validation](core-v1.md#framing-and-validation), not silent fallback.
Rejected work has no execution effects; failed work must expose known effects
truthfully under the shared Response rules, never pretend a required sequence
was forwarded successfully. Complete forwarding acceptance is all-or-none;
an unsuccessful required enqueue cannot be reported as successful zero bytes.

Well-formed rejected requests consume wire IDs. Each receives one correlated
outcome while healthy; known unexecuted stale work rejects, lost/unknown execution
is [indeterminate](../TERMS.md#indeterminate-outcome), never replayed after
[revocation](../../../GLOSSARY.md#attachment-and-control), restoration replacement,
or [connection-generation](../TERMS.md#connection-generation) change.
Ordinary unsupported clients must not send this type to an unadvertising daemon.
Related: [Input](request-input.md), [Ready](request-ready.md),
[Set viewing](request-viewing.md), [Acknowledge](request-acknowledge.md), and
[native Focus semantics](../monitor-client/request-focus.md).

## Implemented behavior

No revision-1 connection is an attachment yet, so until
[#87](https://github.com/jackgeek/climon2/issues/87) serves Focus the
[session daemon](../../../GLOSSARY.md#sessions) rejects it as impermissible
with `invalid_state`, whether it is a connection's first request or arrives on
a [passive monitor connection](../../../GLOSSARY.md#attachment-and-control).
No focus sequence reaches the command; the stream goes on and a registration
stays. This interim rejection comes before the `focus-v1` check shown above.
