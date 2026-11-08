# Final command result

**Status: agreed revision-1 specification, not implemented.**
The [session daemon](../../../GLOSSARY.md#sessions) reports an authoritative final
command result through its [client](../../../GLOSSARY.md#sessions) to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_FINAL` / `data.final` has command result and terminal status availability,
scoped by [generation](../TERMS.md#connection-generation).
Command kind is exit/raw u32 code or signal/positive
source-host number and optional name. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_FINAL;
example.generation = 1;
example.data.final.command_result.kind = CM1_SIGNAL;
example.data.final.command_result.value = 15;
(void)example;
return (cm1_call){0};
```

This example has no selected status support and no signal name.
Core support supplies final result regardless of optional status. When status
is selected it must be terminal and agree: completed is exit zero; failed is
nonzero exit/signal. Exit 143 differs from signal 15.
Final transition is not duplicated as Status change. Final identity is session
GUID plus fixed final-outcome category, not result kind or another counter.
Inconsistent repeats fail and never authorize another toast.

Final stops passive recovery and remains retained after closure;
[EOF](../TERMS.md#eof)/discovery
loss is not final. First learned viewed stays toast-suppressed forever here;
first learned unviewed notifies once. Final does not prove Leave, Kill daemon
cleanup, local worker cleanup, destruction or thread join. Confirmed removal
still waits for local cleanup/join.
See [wire Final](../daemon-client/event-command-final.md),
[Kill](request-kill.md), [client outcome](event-outcome.md),
[ownership/errors](core-v1.md#function-calls-and-ownership) and
[lifecycle](../../monitor-lifecycle.md).
[#67](https://github.com/jackgeek/climon2/issues/67) owns final results;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
