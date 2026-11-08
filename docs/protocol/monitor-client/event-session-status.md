# Session status

**Status: agreed revision-1 specification, not implemented.**
The [session daemon](../../../GLOSSARY.md#sessions) supplies effective
[session status](../../../GLOSSARY.md#status-and-attention) through its
[client](../../../GLOSSARY.md#sessions) to the [monitor](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

Initial `cm1_metadata.status` has explicit availability.
`CM1_STATUS` / `data.status` contains state, episode presence/ID and exact
24-byte millisecond [UTC](../TERMS.md#utc) change time, scoped by
[generation](../TERMS.md#connection-generation).
See [header](client-v1.h), [values](core-v1.md#shared-values) and
[mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_STATUS;
example.generation = 1;
example.data.status.state = CM1_NEEDS_ATTENTION;
example.data.status.episode_present = 1;
example.data.status.episode_id = 2;
memcpy(example.data.status.changed_at.bytes, "2026-11-08T10:00:00.000Z", 24);
(void)example;
return (cm1_call){0};
```

Require selected `status-v1`. States map to idle/running/needs_attention/
acknowledged/completed/failed; final transition belongs to Final, not a duplicated
Status change. Only pending/acknowledged attention has a nonzero episode.
Episode identity survives acknowledgement/reconnect and coalesces while pending.
Time is daemon-stored effective-change time, not receipt/reconnect time or unique ID.
Hidden detector changes do not replace unchanged effective-state time.

Initial state/changes are gap-free and independent of control/readiness.
Unknown/unsupported status remains unavailable with explicit capability
information; invalid core values fail, never normalize to cleared/idle.
Visibility/Focus does not acknowledge; explicit Acknowledge and accepted real
Input have distinct effects. Loss preserves stale status, not fabricated final.
See [wire Status](../daemon-client/event-status-changed.md),
[mechanics](../../session-status.md), [Final](event-session-ended.md)
and [ownership/errors](core-v1.md#function-calls-and-ownership).
[#67](https://github.com/jackgeek/climon2/issues/67) owns status;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
