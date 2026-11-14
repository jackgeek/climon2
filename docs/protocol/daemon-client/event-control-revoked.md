# Control revoked — `0x47`

**Status: agreed and implemented for interactive attachments**
([#87](https://github.com/jackgeek/climon2/issues/87)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) reports the end of the
receiving [client](../../../GLOSSARY.md#sessions)'s
[control](../../../GLOSSARY.md#attachment-and-control).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame), all fields required:
`period: Counter`, `restoration_id: Counter`, `reason` either
`takeover` or `session_ending`.

```json
{"period":"1","restoration_id":"1","reason":"takeover"}
```

## Order and guarantees

Core support is required. [Revocation](../../../GLOSSARY.md#attachment-and-control)
invalidates [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
[viewing](../../../GLOSSARY.md#status-and-attention) and any incomplete
[restoration](../../../GLOSSARY.md#restoration-and-presentation) immediately. Old-authority work is rejected regardless
of when the old reader receives this event; its receipt is not a prerequisite
for [a new grant](event-control-granted.md). No old-authority
[Display](event-display.md) follows this invalidating boundary.

[Attachment](../../../GLOSSARY.md#attachment-and-control) membership persists;
revocation is not detach or completion.
A [monitor](../../../GLOSSARY.md#sessions) shows a placeholder, not live read-only mirroring. Core metadata
continues independently, with no automatic takeover or acknowledgement.
Committed termination overrides [succession](core-v1.md#membership-and-succession).

No [request ID](../TERMS.md#request-id)/defaults/unknown members. Invalid records follow
[fatal validation](core-v1.md#framing-and-validation).
Required control records must be preserved or the connection explicitly failed,
not lossily coalesced. Numeric bounds/deadlines/policy remain #69.
Related: [Ready](request-ready.md), [Leave](request-leave.md).

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) sends Control revoked to
an interactive [attachment](../../../GLOSSARY.md#attachment-and-control)
whose control ends while the attachment stays: with reason `takeover` when
another attachment, of either protocol, attaches or takes control, and with
reason `session_ending` when the session ends while it still controls, just
before its stream closes. Each time, the
[release](../../../GLOSSARY.md#restoration-and-presentation) of its
[presentation](../../../GLOSSARY.md#restoration-and-presentation) reaches it first as
[Display](event-display.md), and Control revoked carries the
[control period](../../../GLOSSARY.md#attachment-and-control) and
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts) that Display used; no
Display follows it. The [Command final](event-command-final.md#implemented-behavior)
of a command whose result is final reaches the attachment after the Display
of the command's last output and before the release and its
`session_ending` revocation.
