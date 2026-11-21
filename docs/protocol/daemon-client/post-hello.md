# Daemon-client post-hello boundary

**Current:** after the frozen [hello handshake](../../../GLOSSARY.md#sessions),
a [session daemon](../../../GLOSSARY.md#sessions) speaks only
[daemon wire revision 1](core-v1.md), owned by
[#67](https://github.com/jackgeek/climon2/issues/67). The version-private
post-hello protocol that came before it was retired in this release
([#104](https://github.com/jackgeek/climon2/issues/104)); there were no prior
releases to stay compatible with. This release's
[client executable](../../../GLOSSARY.md#sessions) attaches and
[kills](../../../GLOSSARY.md#attachment-and-control) through its own
[client library](../../../GLOSSARY.md#sessions) over revision 1
([#100](https://github.com/jackgeek/climon2/issues/100),
[#101](https://github.com/jackgeek/climon2/issues/101)).
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

## Before hello

A connection's first [frame](../TERMS.md#frame) must be a valid
[hello request](hello-request.md) of protocol 1, complete within two seconds
of accept. Anything else is refused: a frame whose length is outside 1 to
1,048,576 [bytes](../TERMS.md#byte), a frame of another type, which is
refused as soon as its type byte arrives, a hello whose
[JSON](../TERMS.md#json) is malformed or whose protocol is not 1, and a
hello still incomplete at the deadline. The daemon closes a refused
connection without sending a byte; nothing else changes.

## After hello

Every frame after a valid hello is a revision-1 frame, read under the
revision-1 frame limit that the [hello reply](hello-reply.md#revision-1-advertisement)
advertises: its length is checked from its prefix before its body arrives.
A type that revision 1 does not define, a second hello included, is
connection-fatal `unknown_message_type`, and a server type sent by a
[client](../../../GLOSSARY.md#sessions) is `wrong_direction`, as the
[core reference](core-v1.md#framing-and-validation) states for every frame.
A peer starts revision 1 with Open or standalone Kill; any other request sent
first is rejected, and the stream goes on: [Focus](request-focus.md) with
`unsupported_capability`, since no Open selected `focus-v1`, and every other
request with `invalid_state`.

The revision-1 advertisement in the hello reply never changes the frozen
tokens (`attach`, `kill`, `state-v1` and `client-startup-v1`) and `state`
that CLI admission and `c2 list` use, and they never imply revision-1
support. A [passive monitor connection](../../../GLOSSARY.md#attachment-and-control)
is not an [attachment](../../../GLOSSARY.md#attachment-and-control), so
`state` never counts it until its Acquire or a promotion makes it one.

## Presentation release

An ordinary interactive attachment's
[release](../../../GLOSSARY.md#restoration-and-presentation) output resets
terminal modes to the normal-shell baseline, then restores the normal-screen
cursor recorded by that attachment's committed
[presentation](../../../GLOSSARY.md#restoration-and-presentation).
This keeps subsequent shell input below restored output, including after Vim
has left the alternate screen. It does not restore the
[host](../../../GLOSSARY.md#sessions)'s pre-attachment screen or query the host.
See the [terminal architecture](../../adr/0007-daemon-owned-virtual-terminal.md#presentation-release).

## Scope

The frozen [hello request](hello-request.md) and [reply](hello-reply.md) remain
unchanged in required fields, protocol 1 and existing limits.
New optional support advertisement and first-role validation precede effects;
no separate negotiation exchange or release-inferred support is introduced.
The [complete core reference](core-v1.md) specifies framing, types,
[capabilities](../../../GLOSSARY.md#sessions), errors, metadata, passive succession,
[viewing](../../../GLOSSARY.md#status-and-attention)/acknowledgement, command results,
kill confirmation, source normalization and bounds obligations.
Every type has a [message page](../README.md#daemon-client-wire-revision-1-partly-implemented).

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
concurrent truthful shutdown, run-local selector removal and
[headless-session](../../../GLOSSARY.md#sessions) zero-client lifetime, all
implemented: the headless lifetime ([#78](https://github.com/jackgeek/climon2/issues/78)) and, by the monitor, recovery
([#116](https://github.com/jackgeek/climon2/issues/116)), removal ([#117](https://github.com/jackgeek/climon2/issues/117)) and shutdown ([#118](https://github.com/jackgeek/climon2/issues/118)); none of these
amendments changes a wire schema. Native declarations are agreed in #71; the
[acceptance matrix](../../monitor-acceptance.md) is agreed in #62,
not executed conformance evidence.

Portable core traffic does not depend on Unix sockets. Future monitor
forwarding and browser consumption are planned, not implemented; routing,
multiplexing, discovery and remote security remain unresolved. No initial
checksum, plugin [API](../TERMS.md#api), native-library-in-browser model or new crash-survival
guarantee is implied.

**Sources:** [frozen hello](../../../crates/climon2-core/src/protocol.rs),
[hello and revision-1 framing in the daemon](../../../crates/climon2-daemon/src/connection.rs),
[revision-1 connections](../../../crates/climon2-daemon/src/session/wire.rs),
[architecture boundary](../../adr/0001-version-matched-client-and-frozen-hello.md),
[approved wire decision](https://github.com/jackgeek/climon2/issues/67),
[retirement](https://github.com/jackgeek/climon2/issues/104).
