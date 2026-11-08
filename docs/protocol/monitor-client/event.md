# Event delivery

**Status: agreed revision-1 specification, not implemented.**
The [client](../../../GLOSSARY.md#sessions) delivers an
[event](../TERMS.md#event) to the [monitor](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

[`cm1_record`](client-v1.h) contains a native tag, reserved zero, startup
generation and one active payload. The [complete event mapping](core-v1.md#events-and-ordering)
fixes all tags, fields, capabilities and daemon sources. Unknown tags are invalid,
not plugin traffic. No plugin records are published in revision 1.

```c
cm1_drained drained = {0};
return api->drain(communication, &drained);
```

Drain returns an immutable bounded ordered batch, empty, or persistent terminal
evidence after earlier queued records. Release each delivery through the same
library; all payload pointers expire at release. Unreleased objects count against
limits and block destruction. See [drain/ownership](core-v1.md#function-calls-and-ownership).

Initial metadata is atomic and gap-free, independent of control/readiness.
Control/restoration/display boundaries retain causal order. Metadata is not
screen mirroring; final command results are not connection loss or client cleanup.
Required records cannot be dropped or lossily coalesced under pressure.
[#66](https://github.com/jackgeek/climon2/issues/66) owns delivery semantics;
[#71](https://github.com/jackgeek/climon2/issues/71) owns declarations.
