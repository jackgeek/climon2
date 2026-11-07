# Kill

**Status: agreed semantics/wire mapping, not implemented; native declarations pending #71.**
The [monitor](../../../GLOSSARY.md#sessions) requests
[kill](../../../GLOSSARY.md#attachment-and-control) through a
[client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md) and inherited
[communication contract](../README.md#shared-in-process-communication-contract).

[Wire Kill](../daemon-client/request-kill.md) is a standalone first-role request,
not an [attachment](../../../GLOSSARY.md#attachment-and-control) or repurposed opened stream. The library must map native work
to that path without inventing success. Validate revision/required support
before effects. Confirmation requires known command result and complete
[daemon](../../../GLOSSARY.md#sessions)
cleanup, not command exit zero, monitor cleanup or thread join.

Repeated kills join committed termination without restarting grace. Termination
overrides succession. Failed/[indeterminate](../TERMS.md#indeterminate-outcome)
cleanup may expose a known result,
never successful confirmation; loss does not prove rejection or completion.
The wire page supplies exact fields, result/errors and a valid wire example,
not native layouts/calls.

[#71](https://github.com/jackgeek/climon2/issues/71) owns native declarations,
[#61](https://github.com/jackgeek/climon2/issues/61) lifecycle/cleanup policy and
[#69](https://github.com/jackgeek/climon2/issues/69) numeric bounds/deadlines.
Related: [session end](event-session-ended.md), [detach](request-detach.md).
