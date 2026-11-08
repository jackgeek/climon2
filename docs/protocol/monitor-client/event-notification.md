# Notification metadata

**Status: agreed revision-1 specification, not implemented.**
The [session daemon](../../../GLOSSARY.md#sessions) sends normalized notification
metadata through its [client](../../../GLOSSARY.md#sessions) to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_NOTIFICATION` / `data.notification` carries source [OSC](../TERMS.md#osc)
9/99/777, optional source_id/title/body and optional nonzero
[episode](../../../GLOSSARY.md#status-and-attention), scoped by
[generation](../TERMS.md#connection-generation).
Title/body cannot both be absent/empty. Text is inert length-delimited
[UTF-8](../TERMS.md#unicode-and-utf-8).
See [header](client-v1.h), [values](core-v1.md#shared-values) and
[mapping](core-v1.md#events-and-ordering).

```c
static const uint8_t body[] = "Build ready; inspect results";
cm1_record example = {0};
example.tag = CM1_NOTIFICATION;
example.generation = 1;
example.data.notification.source = CM1_OSC777;
example.data.notification.episode_present = 1;
example.data.notification.episode_id = 2;
example.data.notification.body = (cm1_optional_text){
    1, 0, {body, sizeof body - 1}};
(void)example;
return (cm1_call){0};
```

Require selected notification-v1 and status-v1; deliver after Initial metadata,
including passive/background/revoked. Source identity is not attention identity.
Repeated notifications while pending coalesce into one episode; no payload
archive/replay or new toast entitlement. Follow daemon normalization, not raw
host OSC forwarding or full desktop-notification emulation.
Notifications do not grant screen/input access.

Local toast dedup persists per episode across reconnect; other clients' viewing
never suppresses this monitor. Nonmodal coverage clears viewing but retains ready
keyboard input; disappearance never acknowledges. Required oversized metadata
fails explicitly. See [wire Notification](../daemon-client/event-notification.md),
[source normalization](../daemon-client/core-v1.md#source-normalization),
[terminal notifications](../terminal-integration.md#mouse-gestures-and-notifications)
and [ownership/limits](core-v1.md#function-calls-and-ownership).
[#67](https://github.com/jackgeek/climon2/issues/67) owns normalization;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
