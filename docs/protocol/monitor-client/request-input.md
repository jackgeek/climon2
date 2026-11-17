# Input

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) sends real user application
[bytes](../TERMS.md#byte) through the [client](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_INPUT` uses `args.input`: current [generation](../TERMS.md#connection-generation)/
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration](../../../GLOSSARY.md#restoration-and-presentation) and
nonempty exact binary bytes. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [Input mapping](core-v1.md#requests-and-result-mapping).

```c
const uint8_t bytes[] = {0x00, 0xff, 0x41};
cm1_request request = {0};
request.tag = CM1_INPUT;
request.request_id = 10;
request.args.input = (cm1_input){
    {generation, period, restoration_id}, {bytes, sizeof bytes}};
return api->submit(communication, &request);
```

Submit only for the selected [controller](../../../GLOSSARY.md#attachment-and-control)
after current restoration application and coherent paint. Preserve arbitrary
bytes including [NUL](../TERMS.md#nul)/invalid [UTF-8](../TERMS.md#unicode-and-utf-8); no
second shortcut parser, [host](../../../GLOSSARY.md#sessions) replies or injected work.
Modal [UI](../TERMS.md#ui)/stale inspection/
too-small canvas blocks input; nonmodal coverage may retain ready keyboard input.
The [terminal contract](../terminal-integration.md#transparent-keyboard-and-exact-paste)
owns key/mouse/paste encoding and stale-gesture draining.

Local acceptance copies bytes but does not clear [attention](../../../GLOSSARY.md#status-and-attention).
Recheck authority at execution; explicit rejection for known stale work,
[indeterminate](../TERMS.md#indeterminate-outcome) for uncertain
effects, never replay/retarget. `CM1_INPUT_ACCEPTED` reports full accepted byte
count, not command consumption. Accepted real input clears daemon pending or
acknowledged attention; Focus/navigation never does.
See [wire Input](../daemon-client/request-input.md) and
[admission/errors/ownership](core-v1.md#function-calls-and-ownership).
[#63](https://github.com/jackgeek/climon2/issues/63) owns routing/application;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
