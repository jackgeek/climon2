# Progress metadata

**Status: agreed revision-1 specification, not implemented.**
The [session daemon](../../../GLOSSARY.md#sessions) sends normalized progress
through its [client](../../../GLOSSARY.md#sessions) to the
[monitor](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

Initial metadata carries available progress; `CM1_PROGRESS` / `data.progress`
has state none/normal/indeterminate/error/paused and optional percent.
Normal requires 0..100; none/indeterminate forbid percent; error/paused permit it.
Absent percent is zero. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#events-and-ordering).

```c
cm1_record example = {0};
example.tag = CM1_PROGRESS;
example.generation = 1;
example.data.progress = (cm1_progress){CM1_PROGRESS_NORMAL, 1, 50, 0};
(void)example;
return (cm1_call){0};
```

Require selected `progress-v1`; gap-free updates follow initial retained current
value while passive/background/revoked. Unsupported is unavailable, never
fabricated no-progress. Unchanged normalized values emit no event; 100 is not
completion. Error progress is not final failed status, and progress alone does
not request attention or grant input/screen access.

The daemon owns application-source parsing/last-number cache; invalid application
sequences leave the indicator unchanged, invalid core records fail explicitly.
No raw host sequence forwarding or plugin prerequisite.
See [wire Progress](../daemon-client/event-progress-changed.md),
[normalization](../daemon-client/core-v1.md#source-normalization) and
[ownership/errors/limits](core-v1.md#function-calls-and-ownership).
[#67](https://github.com/jackgeek/climon2/issues/67) owns progress;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
