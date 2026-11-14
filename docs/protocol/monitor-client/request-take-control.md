# Take control

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the monitor submits it on Enter ([#108](https://github.com/jackgeek/climon2/issues/108); [monitor README](../../../crates/climon2-monitor/README.md#selection-control-and-painting-108)) and on the take-control key over a revoked selection ([#110](https://github.com/jackgeek/climon2/issues/110); [monitor README](../../../crates/climon2-monitor/README.md#switching-revocation-and-taking-control-back-110)).**
The [monitor](../../../GLOSSARY.md#sessions) explicitly asks its
[client](../../../GLOSSARY.md#sessions) to [take control](../../../GLOSSARY.md#attachment-and-control).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_ACQUIRE` uses `args.generation`, a nonzero current opened
[generation](../TERMS.md#connection-generation).
There is no [geometry](../TERMS.md#geometry), old authority or observed
[episode](../../../GLOSSARY.md#status-and-attention) field.
See [header](client-v1.h) and [mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_ACQUIRE;
request.request_id = 2;
request.args.generation = generation;
return api->submit(communication, &request);
```

Initial selection/explicit selection/takeover may acquire; discovery, browsing,
passive recovery and stale inspection never do. Already controlling returns
no-change without new [restoration](../../../GLOSSARY.md#restoration-and-presentation).
Success is `CM1_ACQUIRED` with changed, [period](../../../GLOSSARY.md#attachment-and-control)
and [restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), not
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts), resize or acknowledgement.
Observe grant/restoration/application separately; selected input also waits for
coherent paint. Background succession grants can apply/Ready without selection.

Admission copies/queues and execution rechecks generation/support.
Known-unexecuted stale work rejects; uncertain effects remain indeterminate,
never replayed on a new connection. See [wire Acquire](../daemon-client/request-acquire.md),
[Control](event-control.md), [ownership/errors](core-v1.md#function-calls-and-ownership)
and [terminal contract](../terminal-integration.md).
[#58](https://github.com/jackgeek/climon2/issues/58) owns control;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
