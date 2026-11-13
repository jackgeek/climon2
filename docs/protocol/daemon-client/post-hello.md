# Daemon-client post-hello boundary

**Current:** a [session daemon](../../../GLOSSARY.md#sessions) speaks two
post-hello protocols. The version-matched private protocol is implemented and
serves ordinary attach and [kill](../../../GLOSSARY.md#attachment-and-control).
[Daemon wire revision 1](core-v1.md), owned by
[#67](https://github.com/jackgeek/climon2/issues/67), freezes a separate portable
core contract; it is implemented for
[passive monitor connections](../../../GLOSSARY.md#attachment-and-control)
and is added beside the private protocol, which it is to replace before
release.
These are not interchangeable formats.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

## Choosing a protocol

The first [frame](../TERMS.md#frame) after hello chooses the protocol for the
rest of the connection, by its type [byte](../TERMS.md#byte):

| First frame type | Protocol |
|---|---|
| `0x30`–`0x39` (revision-1 [requests](../TERMS.md#request)) or `0x40`–`0x4d` (revision-1 server types) | [Revision 1](core-v1.md). A server type sent by a [client](../../../GLOSSARY.md#sessions) is then `wrong_direction`. |
| Anything else, such as Attach `0x10` or Kill `0x16` | The private protocol, exactly as before; an unknown type is still its `Error`. |

That frame's length is checked under the frozen hello limit (1 to 1,048,576
[bytes](../TERMS.md#byte)) before its type arrives, so a bad length there is
a private-protocol failure. After the choice, private types on a revision-1
connection are `unknown_message_type`, and revision-1 types on a private
connection are private failures. The daemon reads no further than the frame
it is decoding, so the choice never depends on how bytes were split across
reads. A peer starts revision 1 with Open or standalone Kill; any other
request sent first also selects revision 1 and is rejected as
`invalid_state`, and the stream goes on.

The revision-1 advertisement in the [hello reply](hello-reply.md#revision-1-advertisement)
never changes the frozen tokens and `state` that admission and `climon2 list`
use. A [passive monitor connection](../../../GLOSSARY.md#attachment-and-control)
is not an [attachment](../../../GLOSSARY.md#attachment-and-control), so `state`
never counts it.

## The private protocol

The [current implementation](../../../crates/climon2-core/src/protocol.rs)
contains attach/control, raw input/output, resize,
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
[detach](../../../GLOSSARY.md#attachment-and-control),
[kill](../../../GLOSSARY.md#attachment-and-control) and termination messages.
Current input/output [frames](../TERMS.md#frame) are limited to 65,536
[bytes](../TERMS.md#byte), including type; their bodies are raw bytes.
Do not treat their private codes, authority IDs or response shapes as revision 1.

Current [release](../../../GLOSSARY.md#restoration-and-presentation) output
resets terminal modes to the normal-shell baseline, then restores the
normal-screen cursor recorded by that
[attachment](../../../GLOSSARY.md#attachment-and-control)'s committed
[presentation](../../../GLOSSARY.md#restoration-and-presentation).
This keeps subsequent shell input below restored output, including after Vim
has left the alternate screen. It does not restore the
[host](../../../GLOSSARY.md#sessions)'s pre-attachment screen or query the host.
See the [terminal architecture](../../adr/0007-daemon-owned-virtual-terminal.md#presentation-release).

The frozen [hello request](hello-request.md) and [reply](hello-reply.md) remain
unchanged in required fields, protocol 1 and existing limits.
New optional support advertisement and first-role validation precede effects;
no separate negotiation exchange or release-inferred support is introduced.
The [complete core reference](core-v1.md) specifies framing, types,
[capabilities](../../../GLOSSARY.md#sessions), errors, metadata, passive succession,
[viewing](../../../GLOSSARY.md#status-and-attention)/acknowledgement, command results,
kill confirmation, source normalization and bounds obligations.
Every new type has a [message page](../README.md#daemon-client-wire-revision-1-partly-implemented).

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
schemas. Native declarations are agreed in #71; the
[acceptance matrix](../../monitor-acceptance.md) is agreed in #62,
not executed conformance evidence.

Portable core traffic does not depend on Unix sockets. Future monitor
forwarding and browser consumption are planned, not implemented; routing,
multiplexing, discovery and remote security remain unresolved. No initial
checksum, plugin [API](../TERMS.md#api), native-library-in-browser model or new crash-survival
guarantee is implied.

**Sources:** [current implementation](../../../crates/climon2-core/src/protocol.rs),
[protocol choice and revision-1 framing](../../../crates/climon2-daemon/src/connection.rs),
[passive monitor connections](../../../crates/climon2-daemon/src/session/wire.rs),
[architecture boundary](../../adr/0001-version-matched-client-and-frozen-hello.md),
[approved wire decision](https://github.com/jackgeek/climon2/issues/67).
