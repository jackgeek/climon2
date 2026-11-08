# Request shutdown

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) requests shutdown through its
[client](../../../GLOSSARY.md#sessions)'s independent control handle.
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`shutdown(cm1_control *)` returns `cm1_call`; no request ID, geometry,
generation argument or ordinary queue record. Handle identifies the instance.
See [header](client-v1.h) and [call rules](core-v1.md#function-calls-and-ownership).

```c
/* The control handle came from this library's create call. */
extern cm1_control *example_control;
return api->shutdown(example_control);
```

Thread-safe, nonblocking, idempotent and independent of full ordinary queues.
May precede run. Closes admission; known-unexecuted accepted work gets explicit
rejection, uncertain work stays indeterminate. Attempts required atomic Leave
once, joining any executing explicit departure, never replaying lost Leave/Kill.
Success confirms shutdown intent, not daemon acceptance, closure, cleanup or join.
Repeated signals/exit do not restart deadlines.

Drain terminal evidence separately; release deliveries/handles after concurrent
use/polling ends, then owner-thread destroy and caller join.
Cleanup stalls fail-stop the whole monitor process; no thread kill/unload or
session-survival promise. See [terminal evidence](event-outcome.md),
[Leave](request-detach.md), [native cleanup](core-v1.md#errors-terminal-evidence-and-cleanup)
and [lifecycle](../../monitor-lifecycle.md#coordinated-shutdown).
[#61](https://github.com/jackgeek/climon2/issues/61) owns shutdown policy;
[#71](https://github.com/jackgeek/climon2/issues/71) owns the call.
