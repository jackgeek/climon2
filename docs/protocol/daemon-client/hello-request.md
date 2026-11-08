# Daemon hello request

**Status:** implemented and frozen. This is not the [helper](../../../GLOSSARY.md) hello [envelope](../TERMS.md#envelope).
See the [overview](../README.md), [domain glossary](../../../GLOSSARY.md),
and [technical terms](../TERMS.md).

A [client](../../../GLOSSARY.md) sends hello first on a new [daemon](../../../GLOSSARY.md) connection, including when performing
a [probe](../../../GLOSSARY.md). Hello does not attach, resize, or [take control](../../../GLOSSARY.md).

## Frame and body

The [frame](../TERMS.md#frame) is a four-[byte](../TERMS.md#byte) [big-endian](../TERMS.md#big-endian) length, type byte `0x01`, and [JSON](../TERMS.md#json) body.
Length includes type and body and is between 1 and 1,048,576 inclusive.
The body must be one [UTF-8](../TERMS.md#unicode-and-utf-8) JSON object.

| Required field | Type | Meaning |
|---|---|---|
| `protocol` | [Unsigned 32-bit integer](../TERMS.md#unsigned-integer) | The current [request](../TERMS.md#request) value is 1. |

```json
{"protocol":1}
```

That exact body has 14 bytes. Its frame starts with `00 00 00 0f 01`:
length 15, type `0x01`, then the body.

Unknown optional fields are ignored. Missing/wrong-type required fields,
malformed JSON, or an invalid frame are rejected. The current daemon rejects
a request protocol other than 1 and an out-of-state/repeated hello.
Required-field duplicates are invalid under the current typed JSON parser.

## Deadline and outcome

The daemon requires a valid hello within two seconds of accepting the connection.
A client performing a probe has two seconds starting before connect to receive the full
[reply](hello-reply.md). Partial reads do not restart those deadlines.

Reply success identifies a [live session](../../../GLOSSARY.md); it is not permission to infer
[attachment](../../../GLOSSARY.md), [controller](../../../GLOSSARY.md) identity, or future [plugin](../../../GLOSSARY.md) support from a release number.
After hello, a connection may probe without attaching. Current attach/[kill](../../../GLOSSARY.md)
traffic and its future frozen replacement are described by the
[post-hello boundary](post-hello.md).

**Sources:** [handshake ADR](../../adr/0001-version-matched-client-and-frozen-hello.md),
[implementation](../../../crates/climon2-core/src/protocol.rs), and
[message tests](../../../crates/climon2-core/tests/messages.rs).
