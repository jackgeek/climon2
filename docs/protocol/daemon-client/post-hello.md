# Daemon-client post-hello boundary

**Current:** the version-matched private protocol is implemented.
**Agreed, not implemented:** [daemon wire revision 1](core-v1.md), owned by
[#67](https://github.com/jackgeek/climon2/issues/67), freezes a separate portable
core contract. These are not interchangeable formats.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [current implementation](../../../crates/climon2-core/src/protocol.rs)
contains attach/control, raw input/output, resize,
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
[detach](../../../GLOSSARY.md#attachment-and-control),
[kill](../../../GLOSSARY.md#attachment-and-control) and termination messages.
Current input/output [frames](../TERMS.md#frame) are limited to 65,536
[bytes](../TERMS.md#byte), including type; their bodies are raw bytes.
Do not treat their private codes, authority IDs or response shapes as revision 1.

The frozen [hello request](hello-request.md) and [reply](hello-reply.md) remain
unchanged in required fields, protocol 1 and existing limits.
New optional support advertisement and first-role validation precede effects;
no separate negotiation exchange or release-inferred support is introduced.
The [complete core reference](core-v1.md) specifies framing, types,
[capabilities](../../../GLOSSARY.md#sessions), errors, metadata, passive succession,
[viewing](../../../GLOSSARY.md#status-and-attention)/acknowledgement, command results,
kill confirmation, source normalization and bounds obligations.
Every new type has a [message page](../README.md#daemon-client-agreed-wire-revision-1-not-implemented).

This wire format is not the in-process
[monitor](../../../GLOSSARY.md#sessions)-client
[contract](../README.md#shared-in-process-communication-contract).
Map wire records and IDs through the
[complete native revision-1 declarations](../monitor-client/core-v1.md).
The [agreed #63 terminal contract](../terminal-integration.md) owns rendering/
application/resize semantics, canonical monitor geometry and optional non-user
[Focus `0x39`](request-focus.md), negotiated as `focus-v1`.
[#69](https://github.com/jackgeek/climon2/issues/69) owns numerical
limits/deadlines/policy. The
[agreed lifecycle contract](../../monitor-lifecycle.md) owns recovery,
concurrent truthful shutdown, run-local selector removal and future headless
zero-client lifetime. These amendments are unimplemented and change no wire
schemas. Native declarations are agreed in #71; final acceptance #62 remains open.

Portable core traffic does not depend on Unix sockets. Future monitor
forwarding and browser consumption are planned, not implemented; routing,
multiplexing, discovery and remote security remain unresolved. No initial
checksum, plugin [API](../TERMS.md#api), native-library-in-browser model or new crash-survival
guarantee is implied.

**Sources:** [current implementation](../../../crates/climon2-core/src/protocol.rs),
[architecture boundary](../../adr/0001-version-matched-client-and-frozen-hello.md),
[approved wire decision](https://github.com/jackgeek/climon2/issues/67).
