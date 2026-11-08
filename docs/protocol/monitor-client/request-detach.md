# Detach

**Status:** agreed communication/lifecycle requirements, not implemented; native declarations unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) requests deliberate
[detach](../../../GLOSSARY.md) of an accepted [attachment](../../../GLOSSARY.md).
Success requires [daemon](../../../GLOSSARY.md#sessions) acceptance, not merely local connection closure.
[Client](../../../GLOSSARY.md#sessions) thread completion and resource cleanup are separate observations.
Neither a timeout nor incomplete cleanup proves deliberate detach.

Switching away retains a [client](../../../GLOSSARY.md) and its connection; it is not detach.
Application input stops when monitor exit begins. Client failure does not by
itself guarantee that the [session](../../../GLOSSARY.md) survives.

The [shared contract](../README.md#shared-in-process-communication-contract)
and [semantic examples](../README.md#semantic-examples) apply. Local queue
acceptance is not daemon acceptance of detach. Independent shutdown remains
serviceable when ordinary queues are full, but cannot fabricate detach success.
Closure reason is observable outside ordinary queue capacity; queued records
remain ahead of the terminal indication. Release delivered objects and stop
concurrent handle calls/polling before dedicated-thread cleanup completes.
Outstanding objects explicitly prevent successful destruction.

The [agreed lifecycle](../../monitor-lifecycle.md#coordinated-shutdown) requests
shutdown concurrently for every owned passive, attached, partially started and
failing instance. Each attempts required atomic Leave, closes its connection
and cleans up workers on its owner thread. Drain/release deliveries, end handle
use and join all client threads before clean exit. A lost Leave response stays
unconfirmed; closure, final events or join never convert it to accepted detach.
Repeated exit/signals do not restart the deadline. Stalls use bounded
whole-process fail-stop after best-effort [host](../../../GLOSSARY.md#sessions)
restoration/root [attention](../../../GLOSSARY.md#status-and-attention)
[clear](../monitor-host/report-clear.md),
not thread killing, code unloading or false cleanup success.
These rules add no native operation fields; #71 still owns declarations.
[Library lifetime](https://github.com/jackgeek/climon2/issues/65) owns workers,
borrowed storage, and loaded-library safety.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) owns
the shared rules.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact arguments, results, errors, and executable examples.
[Operational limits](../../monitor-operational-limits.md#deadlines-and-progress) specifies deadlines.

[Wire Leave](../daemon-client/request-leave.md) removes registration/membership
before succession and closes after its response. Its `attachment_detached`
distinguishes passive departure from accepted detach. Any
[controller](../../../GLOSSARY.md#attachment-and-control) departure,
deliberate or lost, can promote a passive successor after existing attachment
hand-back and before final-participant policy; committed termination overrides
succession. Future
[headless sessions](../../../GLOSSARY.md#sessions) survive zero clients even
after later attachment loss, until completion or
[Kill](../../../GLOSSARY.md#attachment-and-control); ordinary leave policy is
unchanged and no native-crash isolation is added. Native declarations remain #71.
