# Control granted — `0x46`

**Status: agreed and implemented** for interactive attachments
([#87](https://github.com/jackgeek/climon2/issues/87)) and monitor-mode
connections, reason `successor` included
([#89](https://github.com/jackgeek/climon2/issues/89)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) sends a
[grant](../../../GLOSSARY.md#attachment-and-control) of control to the receiving
[client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`period: Counter`, `restoration_id: Counter`, `geometry: Geometry`,
`reason` one of `attach`, `acquire`, `hand_back`, `successor`.

```json
{"period":"2","restoration_id":"3","geometry":{"cols":80,"rows":24},"reason":"successor"}
```

## Order and guarantees

Core support is required; passive entry/succession also requires `monitor-v1`.
Each grant starts a fresh increasing
[control period](../../../GLOSSARY.md#attachment-and-control) and precedes matching
[Restoration begin](event-restoration-begin.md). Interactive Open first supplies
response/metadata; later immediate grant effects precede their success response.
Grant is not [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
[viewing](../../../GLOSSARY.md#status-and-attention) or
[attention](../../../GLOSSARY.md#status-and-attention) acknowledgement.

Existing most-recent-control
[attachments](../../../GLOSSARY.md#attachment-and-control) win hand-back before newest accepted
passive successor. [Promotion](../../../GLOSSARY.md#attachment-and-control) creates ordinary attachment membership, retained
through switching/[revocation](../../../GLOSSARY.md#attachment-and-control).
Inheritance does not select, resize, send input
or acknowledge. See [succession](core-v1.md#membership-and-succession).
For monitor-mode connections the supplied geometry also defines canonical
display/restoration serialization, even for passive successor/background grants.
Apply a complete replacement model/history before Ready, without selection or
paint; selected application input additionally requires coherent paint. See the
[terminal contract](../terminal-integration.md).

Old authority is invalid immediately; the old reader need not receive
[Control revoked](event-control-revoked.md) before this grant.
No old-authority Display may cross its invalidating boundary.
No [request ID](../TERMS.md#request-id)/defaults/unknown members; malformed records follow
[fatal validation](core-v1.md#framing-and-validation).
Essential grant records cannot silently disappear; #69 owns bounds/exhaustion,
#63 actual sizing/application.

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) sends Control granted to
an interactive [attachment](../../../GLOSSARY.md#attachment-and-control) at
once when an interactive [Open](request-open.md) (reason `attach`), its
[Acquire](request-acquire.md) (`acquire`) or another controller's departure
(`hand_back`) grants it control. A
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control)
receives it when its own Acquire converts it into an attachment (`acquire`),
or when a controller departs, by Leave or by loss, and no attachment remains
to hand control back to (`successor`): the newest registration is promoted,
and an older one waits for a later departure. From either grant on it is an
attachment, so a later departure hands control back to it (`hand_back`)
before any registration is promoted. Committed termination promotes nobody.
Each grant's [Restoration begin](event-restoration-begin.md) follows immediately.
The geometry is the session geometry the grant applies, under the same
policy as an ordinary attach; for a monitor-mode connection it is always the
session's current geometry, as its grants never resize the session. Periods
and restoration IDs are session-wide and only increase. Private-protocol
attachments share the same control, and their grants use periods and
restorations too, so the numbers one connection sees can skip.
