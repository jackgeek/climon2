# Kill

**Status: agreed revision-1 specification, not implemented.**
The [monitor](../../../GLOSSARY.md#sessions) requests
[Kill](../../../GLOSSARY.md#attachment-and-control) through its
[client](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_KILL` uses `args.required`, a unique list of required
[capability](../../../GLOSSARY.md#sessions) tokens.
Target is this instance's immutable session ID; no alternate target or authority.
See [header](client-v1.h) and [mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_KILL;
request.request_id = 8;
return api->submit(communication, &request);
```

Use a separately hello-validated first-role connection, never repurpose an opened
passive/attached stream. A kill-only instance needs no Open. Verify exact daemon
identity/release/revision/required support before effects. At most one pending
Kill per immutable target; repeated daemon Kill joins termination without
restarting grace. Committed termination overrides succession.

Success `CM1_TERMINATION` requires a present authoritative command result and
complete daemon cleanup, not exit zero or local thread join. Failed/indeterminate
termination may expose a known command and incomplete/unknown cleanup;
initial rejection may expose `CM1_KNOWN_FINAL`. Loss never authorizes replay.
Default-Cancel selector Delete pins GUID identity, retains failed/uncertain rows
and removes only after definitive termination plus local cleanup/join.
See [wire Kill](../daemon-client/request-kill.md),
[results/errors](core-v1.md#requests-and-result-mapping) and
[lifecycle](../../monitor-lifecycle.md#selector-delete-terminate-and-remove).
[#67](https://github.com/jackgeek/climon2/issues/67) owns termination;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
