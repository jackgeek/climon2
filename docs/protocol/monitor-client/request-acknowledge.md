# Acknowledge attention

**Status: agreed semantics/wire mapping, not implemented; native declarations pending #71.**
The [monitor](../../../GLOSSARY.md#sessions) requests explicit acknowledgement of
current pending [attention](../../../GLOSSARY.md#status-and-attention) through its
[client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md) and inherited
[communication contract](../README.md#shared-in-process-communication-contract).

[Wire Acknowledge](../daemon-client/request-acknowledge.md) is separate from
acquisition and [viewing](../../../GLOSSARY.md#status-and-attention),
with **no observed episode ID**. Validate current
[controller](../../../GLOSSARY.md#attachment-and-control), matching period/
[restoration](../../../GLOSSARY.md#restoration-and-presentation),
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts) and latest reported
[viewing](../../../GLOSSARY.md#status-and-attention)
true. No pending episode yields explicit no-change success.

Explicit ordinary attach, focus/selection and menu close/cancel may submit once
ready and unobstructed. Cancel deferred submission if the local target context
changes. Opening/browsing menus never acknowledges; continuous visibility does
not auto-acknowledge new attention.
Local queue acceptance is not [daemon](../../../GLOSSARY.md#sessions)
acknowledgement; stale execution is
explicitly rejected, unknown execution remains
[indeterminate](../TERMS.md#indeterminate-outcome).

The wire page supplies exact fields, result/errors and a valid wire example,
not native layouts/calls. [#71](https://github.com/jackgeek/climon2/issues/71)
owns native declarations and executable examples; bounds remain
[#69](https://github.com/jackgeek/climon2/issues/69).
Related: [viewing](request-viewing.md), [status](event-session-status.md).
