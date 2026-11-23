# Break

**Status: agreed and implemented
([#326](https://github.com/jackgeek/climon2/issues/326)). The
[client library](../../../../GLOSSARY.md#sessions) performs it with `break-v1`
selected, and the [client executable](../../../../GLOSSARY.md#sessions)'s
ordinary attach sends it on Windows when its console reports Ctrl+Break. The
monitor does not send it.**
A [client](../../../../GLOSSARY.md#sessions) asks for Ctrl+Break on the
command's console. Use the [overview](../README.md),
[glossary](../../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_BREAK` uses `args.authority`: current
[generation](../TERMS.md#connection-generation)/
[period](../../../../GLOSSARY.md#attachment-and-control)/
[restoration ID](../../../../GLOSSARY.md#protocol-and-plugin-concepts). See
[header](client-v1.h), [values](README.md#shared-values) and
[mapping](README.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_BREAK;
request.request_id = 6;
request.args.authority = (cm1_authority){generation, period, restoration_id};
return api->submit(communication, &request);
```

The Open must have selected `break-v1`, which only a session daemon on Windows
advertises; otherwise `submit` refuses the request with
`unsupported_capability` and sends nothing. Local admission queues only;
the daemon rechecks the authority and readiness. Success `CM1_BREAK_SENT` has
no active result member and proves only that the daemon started delivering
Ctrl+Break, not that a program received it. Break is never user Input.
Stale unexecuted work rejects; uncertain effects remain indeterminate with no
replay. See [wire Break](../daemon-client/request-break.md) and
[ownership/errors](README.md#function-calls-and-ownership).
