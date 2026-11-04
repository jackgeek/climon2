# Detach

**Status:** planned semantics; interface and lifecycle details unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [monitor](../../../GLOSSARY.md) requests deliberate
[detach](../../../GLOSSARY.md) of an accepted [attachment](../../../GLOSSARY.md).
Success requires daemon acceptance, not merely local connection closure.
Client thread completion and resource cleanup are separate observations.
Neither a timeout nor incomplete cleanup proves deliberate detach.

Switching away retains a [client](../../../GLOSSARY.md) and its connection; it is not detach.
Application input stops when monitor exit begins. Client failure does not by
itself guarantee that the [session](../../../GLOSSARY.md) survives.

[Lifecycle](https://github.com/jackgeek/climon2/issues/61) must specify shutdown
of passive connections as well as attachments, session survival, and recovery.
[Library lifetime](https://github.com/jackgeek/climon2/issues/65) owns workers,
borrowed storage, and loaded-library safety.
The [communication decision](https://github.com/jackgeek/climon2/issues/66) and
[interface schemas](https://github.com/jackgeek/climon2/issues/71) must assign
arguments, results, errors, and examples.
[Operational limits](https://github.com/jackgeek/climon2/issues/69) owns deadlines.
