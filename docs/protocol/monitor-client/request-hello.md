# Library compatibility check

**Status:** embedding design agreed; complete declarations pending, not implemented.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

Before creating a [client](../../../GLOSSARY.md) instance or
[daemon](../../../GLOSSARY.md#sessions) connection,
the [monitor](../../../GLOSSARY.md) must validate the loaded
[client library](../../../GLOSSARY.md)'s exact release identity and required
embedding support. Loading a library does not establish an
[attachment](../../../GLOSSARY.md) or [readiness](../../../GLOSSARY.md).
Missing artifacts, release mismatch, and unsupported interfaces fail explicitly.

The [embedding reference](../library-embedding.md), owned by
[Define version-matched client library loading and lifetime](https://github.com/jackgeek/climon2/issues/65),
selects a versioned [C ABI](../TERMS.md#c-abi) and the stable
`climon2_client_get_interface` entry point. It requests an interface revision
and returns the supported revision's [function table](../TERMS.md#function-table)
and release identity, or an explicit unsupported result. Release identity and
interface compatibility are separate checks. Exact declarations and result
representations remain pending; revision 1 is planned, not published.
[Define monitor-client communication contract](https://github.com/jackgeek/climon2/issues/66)
owns the agreed [subsequent communication](../README.md#shared-in-process-communication-contract).
After these checks, the instance reports [capabilities](../../../GLOSSARY.md#sessions) available through both
library and daemon; revision support alone does not establish operation support.
Required support is checked before control acquisition or operation submission.
The [semantic examples](../README.md#semantic-examples) describe subsequent
admission and delivery without inventing calls. This filename does not prescribe a hello
message or daemon protocol number. A complete call example awaits the
[interface schemas](https://github.com/jackgeek/climon2/issues/71).

This is separate from the implemented
[daemon hello](../daemon-client/hello-request.md).
