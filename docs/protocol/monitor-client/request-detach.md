# Detach

**Status:** agreed communication requirements, not implemented; interface and lifecycle details unresolved.
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

[Lifecycle](https://github.com/jackgeek/climon2/issues/61) must specify shutdown
of passive connections as well as attachments, session survival, and recovery.
[Library lifetime](https://github.com/jackgeek/climon2/issues/65) owns workers,
borrowed storage, and loaded-library safety.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) owns
the shared rules.
[Interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
exact arguments, results, errors, and executable examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns deadlines.
