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

[#71](https://github.com/jackgeek/climon2/issues/71) owns native declarations.
The [agreed lifecycle](../../monitor-lifecycle.md) owns recovery/cleanup policy;
[Operational limits](../../monitor-operational-limits.md#deadlines-and-progress) specifies numeric bounds/deadlines.
Related: [session end](event-session-ended.md), [detach](request-detach.md).

Planned [selector Delete](../../monitor-selector.md#delete-terminate-and-remove)
is a default-Cancel modal tied to an immutable
[session ID](../../../GLOSSARY.md#protocol-and-plugin-concepts), not a new Kill
operation. Suppress duplicate pending work. Dismiss only after authoritative
termination and local delivery/handle release, instance cleanup and thread join.
Rejection/failure keeps the row; lost responses permit passive outcome recovery
but no automatic Kill replay. Dismissal discards cache for this run only and
blocks same-run rediscovery. Fallback selection does not acquire control.
Existing `climon2 kill <session>` must preserve authoritative termination for
future [headless sessions](../../../GLOSSARY.md#sessions), tested through the
existing command rather than a duplicate CLI. All these amendments are
unimplemented; native declarations #71, numerical limits #69 and acceptance #62
remain open.
