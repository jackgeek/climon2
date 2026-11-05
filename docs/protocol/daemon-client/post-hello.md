# Daemon-client post-hello boundary

**Status:** the existing version-matched private protocol is implemented.
Its future cross-release frozen contract is unresolved.
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

The [current implementation](../../../crates/climon2-core/src/protocol.rs)
contains attach/control, raw input/output, resize, [readiness](../../../GLOSSARY.md), [detach](../../../GLOSSARY.md), [kill](../../../GLOSSARY.md), and
termination messages. Current input/output [frames](../TERMS.md#frame) are limited to 65,536 [bytes](../TERMS.md#byte)
including the type byte; their bodies are raw bytes.

This first draft deliberately does not reproduce those private message [schemas](../TERMS.md#schema)
as a future [plugin](../../../GLOSSARY.md) contract. Their type codes, period representation, geometry,
and acknowledgement shapes cannot simply be copied into the monitor-client interface
or assumed frozen across future releases.

The implemented [hello request](hello-request.md) and [reply](hello-reply.md)
remain the documented frozen discovery exchange.

## Pending specification

[Freeze daemon-client communication contract](https://github.com/jackgeek/climon2/issues/67)
owns post-hello framing and message schemas, [capabilities](../../../GLOSSARY.md), validation,
readiness/control semantics, compatible [extensions](../../../GLOSSARY.md), and rejection behavior.

It must provide support discovery before [control acquisition](../../../GLOSSARY.md),
passive monitor connections without automatic takeover, and bidirectional plugin
extension relay regardless of control. Negotiated plugin data must reach the
monitor for background and revoked clients without permitting core screen
mirroring, application input, or resize. The exact passive connection/attachment
model and wire schemas are not yet assigned.

The [startup/discovery resolution](https://github.com/jackgeek/climon2/issues/59)
also requires daemon-core
[title state and changes](../monitor-client/event-title.md) and
[effective session status/time](../monitor-client/event-session-status.md)
for passive, background, and revoked clients, without taking control or
implementing plugins. The [mechanics and diagrams](../../session-status.md)
supersede the earlier plugin release requirement. Older daemons lacking title
support remain usable with command/arguments fallback.

Specify [viewing reports](../monitor-client/request-viewing.md) independently
of retained control, and distinguish command completion from uncertain
disconnection/passive reconnect. Current hello's optional `state` describes
lifecycle/attachments/control, not these new activity/attention machines.
Do not silently repurpose that existing shape. Frozen hello required fields
remain unchanged; exact core status/title/viewing schemas and compatibility
remain unresolved.

The in-process [monitor-client contract](https://github.com/jackgeek/climon2/issues/66)
does not select this daemon transport's framing or encoding.

Numerical cross-interface bounds belong to
[operational limits](https://github.com/jackgeek/climon2/issues/69).
[Session](../../../GLOSSARY.md) survival/recovery belongs to
[lifecycle](https://github.com/jackgeek/climon2/issues/61).

Once the frozen schemas are decided, add one linked page per daemon message or
operation, with valid examples. Until then, no future post-hello wire example,
extension advertisement shape, or plugin interface is assigned here.

**Sources:** [current handshake/private-protocol ADR](../../adr/0001-version-matched-client-and-frozen-hello.md)
and [monitor ownership resolution](https://github.com/jackgeek/climon2/issues/58#issuecomment-6597610447).
