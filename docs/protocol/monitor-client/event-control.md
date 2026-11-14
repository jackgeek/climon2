# Control grant and revocation

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [client](../../../GLOSSARY.md#sessions) reports daemon
[grants](../../../GLOSSARY.md#attachment-and-control) and
[revocations](../../../GLOSSARY.md#attachment-and-control) to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schemas and examples

`CM1_GRANTED` / `data.granted` has [period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), effective
[geometry](../TERMS.md#geometry) and
reason attach/take-control/hand-back/successor. `CM1_REVOKED` / `data.revoked`
has period/restoration and takeover/session-ending reason. All IDs are nonzero;
record [generation](../TERMS.md#connection-generation) scopes them. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_GRANTED;
example.generation = 1;
example.data.granted = (cm1_granted){2, 3, {80, 24}, CM1_SUCCESSOR, 0};
(void)example;
return (cm1_call){0};
```

```c
cm1_record example = {0};
example.tag = CM1_REVOKED;
example.generation = 1;
example.data.revoked = (cm1_revoked){2, 3, CM1_TAKEOVER, 0};
(void)example;
return (cm1_call){0};
```

Core control [capability](../../../GLOSSARY.md#sessions) is required.
Grant precedes matching [restoration](../../../GLOSSARY.md#restoration-and-presentation)
and is not [Ready](../../../GLOSSARY.md#protocol-and-plugin-concepts), selection,
resize, [viewing](../../../GLOSSARY.md#status-and-attention) or acknowledgement. Passive promotion/
background hand-back uses canonical effective geometry and may apply/Ready
without paint. Revocation invalidates readiness/viewing/incomplete restoration;
membership remains, no live mirroring or connected revoked-cache inspection.
Old authority is invalid regardless of when the old reader receives revocation.

Required events cannot be lost/coalesced; same-library delivery release owns
buffers. See [ownership/order](core-v1.md#function-calls-and-ownership),
[wire Grant](../daemon-client/event-control-granted.md),
[wire Revocation](../daemon-client/event-control-revoked.md) and
[terminal/lifecycle rules](../../monitor-lifecycle.md).
[#67](https://github.com/jackgeek/climon2/issues/67) owns succession;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native schemas.
