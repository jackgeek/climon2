# Application focus

**Status: agreed revision-1 specification, implemented by the
[client library](../../../GLOSSARY.md#sessions)
([#94](https://github.com/jackgeek/climon2/issues/94)) as its
[implemented behavior](core-v1.md#implemented-behavior) details; the caller is
not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) sends non-user application focus
through its [client](../../../GLOSSARY.md#sessions).
Use the [overview](../README.md), [glossary](../../../GLOSSARY.md) and
[terms](../TERMS.md).

## Schema and example

`CM1_FOCUS` uses `args.flag`: current [generation](../TERMS.md#connection-generation)/
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), 0/1
desired `value`, zero reserved bytes. Require selected `focus-v1` through both
library and daemon. See [header](client-v1.h) and [mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_FOCUS;
request.request_id = 6;
request.args.flag.authority = (cm1_authority){generation, period, restoration_id};
request.args.flag.value = 0;
return api->submit(communication, &request);
```

Require current [ready](../../../GLOSSARY.md#protocol-and-plugin-concepts) authority.
Selected ready [host](../../../GLOSSARY.md#sessions)-focused sessions without
modal [UI](../TERMS.md#ui) are keyboard-focused; retained
[controllers](../../../GLOSSARY.md#attachment-and-control) may send focus-out.
Menus/switching clear focus; scrollback/nonmodal [toasts](../TERMS.md#toast) do not.
During [restoration](../../../GLOSSARY.md#restoration-and-presentation)
retain latest desired local state, then synchronize only fresh ready authority.
Unsupported focus is a visible limitation, not an unusable session or plugin.
Focus is not Input, terminal reply, [viewing](../../../GLOSSARY.md#status-and-attention)
or [attention](../../../GLOSSARY.md#status-and-attention) acknowledgement.

Local admission queues only. Success `CM1_FOCUS_RESULT` reports changed/value
and 0 or 3 complete bytes accepted for forwarding, not command consumption.
State remains known with application reporting disabled; enabling it emits current
state once without duplicate transitions. Known stale work rejects explicitly;
uncertain effects are indeterminate, never replayed.
See [wire Focus](../daemon-client/request-focus.md),
[terminal focus](../terminal-integration.md#host-focus-and-application-focus)
and [ownership/errors](core-v1.md#function-calls-and-ownership).
[#63](https://github.com/jackgeek/climon2/issues/63) owns focus;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
