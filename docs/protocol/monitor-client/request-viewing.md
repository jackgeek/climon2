# Report viewing

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) reports
[viewing](../../../GLOSSARY.md#status-and-attention) through its
[client](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_VIEWING` uses `args.flag`: current [generation](../TERMS.md#connection-generation)/
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), 0/1
`value`, zero reserved bytes. See [header](client-v1.h) and
[mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_VIEWING;
request.request_id = 4;
request.args.flag.authority = (cm1_authority){generation, period, restoration_id};
request.args.flag.value = 1;
return api->submit(communication, &request);
```

True requires selected unobstructed [ready](../../../GLOSSARY.md#protocol-and-plugin-concepts)
live presentation, coherent usable canvas and [host](../../../GLOSSARY.md#sessions)
focus when reportable. False can clear during [restoration](../../../GLOSSARY.md#restoration-and-presentation).
Menus/coverage/focus loss clear viewing without releasing control; nonmodal
coverage need not block ready keyboard input. Visibility/focus-in never
acknowledges [attention](../../../GLOSSARY.md#status-and-attention).
Only this monitor's viewing suppresses its local [toasts](../TERMS.md#toast).
See [terminal rules](../terminal-integration.md) and [Acknowledge](request-acknowledge.md).

Local admission is not daemon acceptance; execution rechecks authority.
Success `CM1_VIEWING_RESULT` carries changed/value and zero accepted_bytes.
Stale unexecuted work rejects explicitly; uncertain effects are indeterminate,
never replayed. No aggregate viewing event is invented.
See [wire Viewing](../daemon-client/request-viewing.md) and
[ownership/errors](core-v1.md#function-calls-and-ownership).
[#67](https://github.com/jackgeek/climon2/issues/67) owns viewing/acknowledgement;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
