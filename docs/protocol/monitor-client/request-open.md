# Open connection

**Status: agreed revision-1 specification. The
[client library](../../../GLOSSARY.md#sessions) implements monitor-mode Open
([#93](https://github.com/jackgeek/climon2/issues/93)), as its
[implemented behavior](core-v1.md#implemented-behavior) details; interactive
Open reports `CM1_UNSUPPORTED_OPERATION` until
[#94](https://github.com/jackgeek/climon2/issues/94). The caller is not
implemented.**
The [monitor](../../../GLOSSARY.md#sessions) asks its
[client](../../../GLOSSARY.md#sessions) to establish a role with the
[session daemon](../../../GLOSSARY.md#sessions). Use the [overview](../README.md),
[glossary](../../../GLOSSARY.md) and [terms](../TERMS.md).

## Schema and example

`CM1_OPEN` uses `cm1_request.args.open`: mode, [geometry](../TERMS.md#geometry),
required/optional [capability](../../../GLOSSARY.md#sessions) lists.
Monitor mode requires zero geometry; interactive requires
positive columns/rows. Reserved values are zero. Exact
[startup inputs](core-v1.md#startup-limits-and-resources) are copied separately
by create. See [header](client-v1.h) and [Open mapping](core-v1.md#requests-and-result-mapping).

```c
cm1_request request = {0};
request.tag = CM1_OPEN;
request.request_id = 1;
request.args.open.mode = CM1_MONITOR;
return api->submit(communication, &request);
```

One Open attempt per instance; never replace an established role. Validate exact
daemon identity/release, wire revision, peer frame budgets and effective
capability intersection before effects. Unsupported required support rejects;
unknown optional support is unselected. Missing optional title/status/focus is
explicit, not fabricated supported empty/clear state.

Success returns `CM1_OPENED`, logical connection [GUID](../TERMS.md#guid)
and selected tokens, then
exactly one [Initial metadata](event-initial-metadata.md) barrier before gap-free
changes. Monitor Open registers an eligible passive successor without attachment,
geometry or takeover; interactive Open attaches/grants after metadata.
Open is not Ready or current cached presentation.
Passive recovery creates a fresh [generation](../TERMS.md#connection-generation)/instance after prior cleanup/join,
without takeover or replay. See [wire Open](../daemon-client/request-open.md),
[admission/ownership](core-v1.md#function-calls-and-ownership), and
[lifecycle](../../monitor-lifecycle.md).
[#67](https://github.com/jackgeek/climon2/issues/67) owns roles;
[#71](https://github.com/jackgeek/climon2/issues/71) owns native fields.
