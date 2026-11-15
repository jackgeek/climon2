# Diagnostic

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) implements its own
content-free diagnostic for a contained panic
([#85](https://github.com/jackgeek/climon2/issues/85)) and the diagnostic of
a failed connection ([#93](https://github.com/jackgeek/climon2/issues/93)):
[daemon](../../../GLOSSARY.md#sessions) errors with daemon origin, and
transport and library failures with theirs.**
The [client](../../../GLOSSARY.md#sessions) supplies explicit inert error
information to the [monitor](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_DIAGNOSTIC` / `data.diagnostic` contains origin, reserved zero, nonempty
stable [UTF-8](../TERMS.md#unicode-and-utf-8) code and diagnostic message,
scoped by record [generation](../TERMS.md#connection-generation).
See [header](client-v1.h), [event mapping](core-v1.md#events-and-ordering)
and [error catalog](core-v1.md#errors-terminal-evidence-and-cleanup).

```c
cm1_record example = {0};
example.tag = CM1_DIAGNOSTIC;
example.generation = 1;
example.data.diagnostic = (cm1_error){CM1_ORIGIN_LIBRARY, 0,
    {(const uint8_t *)"timeout", 7},
    {(const uint8_t *)"Operation deadline exceeded", 27}};
(void)example;
return (cm1_call){0};
```

Diagnostics are content-free metadata, not terminal bytes or a success outcome.
Unknown codes stay explicit errors, never imply retry. Asynchronous daemon
operational/fatal codes are preserved with origin; native/transport errors have
their distinct origin. Failure retry classification is separately reported in
terminal evidence. A queued fatal diagnostic is best effort; persistent terminal
reason remains observable outside full ordinary queues after earlier records.

Same-library immutable delivery ownership applies; never free text with caller
allocators or log session content. Synchronous call diagnostics have a different
borrow-until-next-call lifetime. See [wire Fatal](../daemon-client/event-fatal-error.md),
[client outcome](event-outcome.md) and [ownership](core-v1.md#function-calls-and-ownership).
[#66](https://github.com/jackgeek/climon2/issues/66) owns failure delivery;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native errors.
