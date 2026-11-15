# Leave / detach

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) implements Leave of a
[passive monitor connection](../../../GLOSSARY.md#attachment-and-control) and
shutdown's own Leave ([#93](https://github.com/jackgeek/climon2/issues/93)), as
its [implemented behavior](core-v1.md#implemented-behavior) details; detaching
an attachment arrives with interactive roles
([#94](https://github.com/jackgeek/climon2/issues/94)). The caller is not
implemented.**
The [monitor](../../../GLOSSARY.md#sessions) asks its
[client](../../../GLOSSARY.md#sessions) to leave a daemon connection, deliberately
[detaching](../../../GLOSSARY.md#attachment-and-control) any attachment.
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_LEAVE` uses `args.generation`, the current opened
[generation](../TERMS.md#connection-generation); no stale
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration](../../../GLOSSARY.md#restoration-and-presentation) requirement.
See [header](client-v1.h) and
[mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_LEAVE;
request.request_id = 7;
request.args.generation = generation;
return api->submit(communication, &request);
```

Atomic Leave removes passive registration and any attachment before succession,
including promotion racing with shutdown. Switching away retains control and
is not Leave. Local admission proves only queuing. Success `CM1_LEFT` reports
attachment_detached 0 for passive deregistration, 1 for confirmed attachment
detach. Response loss remains indeterminate; closure/final/worker cleanup/join
never retroactively confirms it.

Independent shutdown stays serviceable with full queues and records internal
Leave evidence without a public ID. Join an existing departure attempt, never
replay an uncertain Leave. Drain/release deliveries, stop polling/calls, release
handles, owner-thread destroy and caller join remain separate obligations.
See [wire Leave](../daemon-client/request-leave.md),
[cleanup](core-v1.md#errors-terminal-evidence-and-cleanup) and
[lifecycle](../../monitor-lifecycle.md#coordinated-shutdown).
[#61](https://github.com/jackgeek/climon2/issues/61) owns shutdown;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
