# Acknowledge attention

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) explicitly acknowledges
[attention](../../../GLOSSARY.md#status-and-attention) through its
[client](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_ACKNOWLEDGE` uses `args.authority`: current [generation](../TERMS.md#connection-generation)/
[period](../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts).
No observed [episode](../../../GLOSSARY.md#status-and-attention) field. See [header](client-v1.h),
[values](core-v1.md#shared-values) and [mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_ACKNOWLEDGE;
request.request_id = 5;
request.args.authority = (cm1_authority){generation, period, restoration_id};
return api->submit(communication, &request);
```

Require current [controller](../../../GLOSSARY.md#attachment-and-control),
applied [Ready](../../../GLOSSARY.md#protocol-and-plugin-concepts) and latest daemon
[viewing](../../../GLOSSARY.md#status-and-attention) true.
Explicit ordinary attach/selection or menu-close/cancel may acknowledge current
pending attention; opening/browsing, visibility or host focus-in never does.
Cancel deferred intent if local target context changes before submission.
No pending episode gives explicit no-change success.

Local admission queues only; execution rechecks authority/readiness/viewing.
Success `CM1_ACKNOWLEDGEMENT` carries changed. Stale unexecuted work rejects;
uncertain effects remain indeterminate with no replay. Acknowledgement keeps
episode identity; accepted real Input clears it separately. Focus never clears it.
See [wire Acknowledge](../daemon-client/request-acknowledge.md),
[status](event-session-status.md), [terminal rules](../terminal-integration.md)
and [ownership/errors](core-v1.md#function-calls-and-ownership).
[#67](https://github.com/jackgeek/climon2/issues/67) owns attention;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
