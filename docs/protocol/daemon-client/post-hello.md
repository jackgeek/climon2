# Daemon-client post-hello boundary

**Status:** the existing version-matched private protocol is implemented.
Its future cross-release frozen contract is unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [current implementation](../../../crates/climon2-core/src/protocol.rs)
contains attach/control, raw input/output, resize, [readiness](../../../GLOSSARY.md), [detach](../../../GLOSSARY.md), [kill](../../../GLOSSARY.md), and
termination messages. Current input/output [frames](../TERMS.md#frame) are limited to 65,536 [bytes](../TERMS.md#byte)
including the type byte; their bodies are raw bytes, not [helper](../../../GLOSSARY.md)-style adjacent
binary [payloads](../TERMS.md#payload).

This first draft deliberately does not reproduce those private message [schemas](../TERMS.md#schema)
as a future [plugin](../../../GLOSSARY.md) contract. Their type codes, period representation, geometry,
and acknowledgement shapes cannot simply be copied into the helper interface
or assumed frozen across future releases.

The implemented [hello request](hello-request.md) and [reply](hello-reply.md)
remain the documented frozen discovery exchange.

## Pending specification

[Freeze daemon-client communication contract](https://github.com/jackgeek/climon2/issues/67)
owns post-hello framing and message schemas, [capabilities](../../../GLOSSARY.md), validation,
readiness/control semantics, compatible [extensions](../../../GLOSSARY.md), and rejection behavior.

It must provide pre-[attachment](../../../GLOSSARY.md) support discovery and mapping for the agreed
bidirectional [JSON](../TERMS.md#json)/binary plugin extension relay. The helper framing decision
does not select this daemon transport's full framing for it.

Numerical cross-interface bounds belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
[Session](../../../GLOSSARY.md) survival/recovery belongs to
[lifecycle](https://github.com/jackgeek/climon2/issues/61).

Once the frozen schemas are decided, add one linked page per daemon message or
operation, with valid examples. Until then, no future post-hello wire example,
extension advertisement shape, or plugin interface is assigned here.

**Sources:** [current handshake/private-protocol ADR](../../adr/0001-version-matched-client-and-frozen-hello.md)
and [helper relay resolution](https://github.com/jackgeek/climon2/issues/66#issuecomment-6616062936).
