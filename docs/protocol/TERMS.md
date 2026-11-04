# Technical terms

This reference explains protocol vocabulary. The [domain glossary](../../GLOSSARY.md)
owns climon2 concepts such as [sessions](../../GLOSSARY.md),
[controllers](../../GLOSSARY.md), [restoration](../../GLOSSARY.md), and
[readiness](../../GLOSSARY.md).
The [protocol overview](README.md) owns wire rules and limits; definitions here
do not add protocol requirements.

## ADR

An architecture decision record: a document explaining a design decision and
the alternatives considered.

## Backpressure

Slowing a sender when the receiver cannot keep up, so pending data stays bounded.
It is not permission to silently drop required messages.

## Baseline

The required common protocol behavior that compatible participants agree to
support. Optional extensions add behavior without redefining that common contract.

## BEL

The terminal bell control byte, hexadecimal `07`. A future attention plugin
could observe it; that plugin strategy is not specified by this reference.

## Big endian

An integer encoding that puts its most significant byte first.
For example, the four bytes `00 00 00 0f` encode the decimal number 15.

## Byte

A unit of eight bits, with a value from 0 through 255.
A byte count is not a character count.

## C ABI

The C application binary interface: the calling convention and binary data
layout used to call functions across a native-library seam.

## Canonical decimal string

An unsigned integer written as decimal digits in one unambiguous form:
`"0"` or a nonzero digit followed by digits, without a sign, spaces, or leading
zeros. Each interface specifies its permitted range separately.

## Discriminator

A field or code identifying which message variant is being sent.

## Envelope

The outer structure that identifies a message and carries shared fields.
It is distinct from the message-specific payload and the client startup envelope.

## Event

A message that reports information without asking for a response.

## EOF

End of file: the end of a byte stream. EOF alone does not explain whether
a client detached successfully or completed cleanup.

## Frame

A length-delimited unit on a communication channel.
A logical message can use several frames, and a read can contain part of a frame.

## Function table

An agreed layout of function pointers through which a caller uses one revision
of a native-library interface.

## GUID

A globally unique identifier. A climon2 session ID is a GUID, not a process ID.

## High-water mark

The greatest request ID received in one originating direction.
Strictly increasing IDs let a receiver detect reuse without storing all past IDs.

## Idempotency key

An identifier that an explicitly defined retry mechanism uses to recognize
the same logical operation. A request ID alone does not supply that mechanism
or promise exactly-once execution.

## Indeterminate outcome

An outcome that cannot be established after communication fails.
It is neither confirmed success nor proof that nothing happened.

## Interception seam

A defined place where a plugin can observe or transform terminal traffic or
structured messages. It is not an unchecked bypass of protocol validation.

## JSON

JavaScript Object Notation, a text format for objects, arrays, strings, numbers,
booleans, and null. JSON wire examples in this reference show message bodies,
not their length prefixes.

## MiB

A mebibyte: 1,048,576 bytes. This unit is distinct from a decimal megabyte.

## Namespaced name

A name scoped to avoid collisions with names owned by other extensions.
`org.example.attention` illustrates an extension name; the exact naming
grammar and registration policy are still unspecified.

## Native path

A filesystem path represented without losing platform-native data, including
non-UTF-8 bytes on Unix or native code units on Windows.

## Opaque handle

An identifier for library-owned state whose internal representation a caller
must not inspect. Its interface specifies validity, permitted uses, and release.

## OSC

Operating System Command, a terminal escape-sequence family.
An OSC terminal sequence is not itself a structured plugin protocol message.
OSC 7501 is the Program Status Protocol in the attention-plugin example.
The first draft does not specify OSC 7501 or OSC 9 plugin decoding.

## Payload

The content carried by a message or operation, distinct from its routing and
correlation metadata.

## Pseudo-terminal (PTY)

An operating-system terminal pair connecting a session daemon to its command.
It is distinct from the user's host terminal and a daemon socket.

## QUIC

A secure transport with multiple streams. It provides ordered bytes within a
stream, but not ordering between streams. It is outside the current monitor plan.
See [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html#section-2).

## Request

A message asking another participant to perform an operation.

## Request ID

An identifier matching a request with its response in a conversation.
It is distinct from a control period, restoration ID, and idempotency key.

## Response

A message reporting the outcome of a request.

## Sandboxing

Restricting what executing code can access or change.
The future plugin system's isolation model is not specified by this reference.

## Schema

The permitted structure, fields, types, and values of a message.

## Standard streams

Stdin, stdout, and stderr are a process's standard input, output, and diagnostic
streams.

## Toast

A brief on-screen notification. The attention-plugin example anticipates such
notifications but does not specify their presentation.

## Unicode and UTF-8

Unicode represents text, including non-Latin writing and emoji.
UTF-8 encodes Unicode text as bytes; one character may require several bytes.
Arbitrary terminal bytes need not be valid UTF-8.

## Unsigned integer

A whole number that cannot be negative. Its bit width determines the largest
representable value.
