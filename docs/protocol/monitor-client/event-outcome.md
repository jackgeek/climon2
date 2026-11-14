# Client outcome and cleanup evidence

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) implements terminal evidence
and the run report for instances that never open a connection
([#85](https://github.com/jackgeek/climon2/issues/85)): their
[Leave](request-detach.md) is not required and no connection remains open.
Opened connections arrive with [#93](https://github.com/jackgeek/climon2/issues/93).**
The [client](../../../GLOSSARY.md#sessions) reports instance termination to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

Drain's `CM1_TERMINAL` returns immutable `cm1_terminal`:
[generation](../TERMS.md#connection-generation), client
outcome, explicit retry classification, Leave evidence, attachment-detached/
connection-closed flags and error. It is persistent outside queue capacity,
after earlier queued records, not an ordinary event tag.
Run separately returns `cm1_run_report` with worker-cleanup state.
See [header](client-v1.h) and [terminal rules](core-v1.md#errors-terminal-evidence-and-cleanup).

```c
cm1_terminal example = {0};
example.generation = 1;
example.client_outcome = CM1_CLIENT_STOPPED;
example.retry = CM1_RETRY_NONE;
example.leave = CM1_LEAVE_CONFIRMED;
example.attachment_detached = 0;
example.connection_closed = 1;
(void)example;
return (cm1_call){0};
```

This is confirmed passive deregistration and closure, not worker cleanup/join.
Stopped has no error; failed requires origin/code/text. Retry none/passive/
permanent is explicit and never inferred from unknown error prose.
Leave not-required/confirmed/rejected/failed/indeterminate is distinct from
closure/final result. Lost acknowledgement never becomes confirmed.
Fatal wire errors map to explicit diagnostics plus persistent terminal reason;
unknown core values fail rather than inventing success.

After terminal release deliveries, end polling/calls and release communication/
control handles. Owner-thread run return proves only its cleanup report;
destroy stays busy with outstanding ownership. Successful destroy and caller
join are separately required before replacement. Stalls/native faults use
whole-process fail-stop, never forced unload/thread termination or false joins.
See [ownership](core-v1.md#function-calls-and-ownership),
[library lifetime](../library-embedding.md) and
[lifecycle](../../monitor-lifecycle.md#coordinated-shutdown).
[#61](https://github.com/jackgeek/climon2/issues/61) owns cleanup/recovery;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native reporting.
