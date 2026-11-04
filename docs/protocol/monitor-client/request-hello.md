# Library compatibility check

**Status:** required check; embedding interface unresolved, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Before creating a [client](../../../GLOSSARY.md) connection or taking control,
the [monitor](../../../GLOSSARY.md) must validate the loaded
[client library](../../../GLOSSARY.md)'s exact release identity and required
embedding support. Loading a library does not establish an
[attachment](../../../GLOSSARY.md) or [readiness](../../../GLOSSARY.md).
Missing artifacts, release mismatch, and unsupported interfaces fail explicitly.

[Define version-matched client library loading and lifetime](https://github.com/jackgeek/climon2/issues/65)
must define multi-instance and cross-release compatibility checks, symbols,
memory ownership, and failure handling.
[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns subsequent communication. This filename does not prescribe a hello
message, protocol number, or C function. A complete call example awaits the
[interface schemas](https://github.com/jackgeek/climon2/issues/71).

This is separate from the implemented
[daemon hello](../daemon-client/hello-request.md).
