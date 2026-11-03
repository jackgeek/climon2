# Technical terms

This reference explains protocol vocabulary. The [domain glossary](../../GLOSSARY.md)
owns climon2 concepts such as sessions, controllers, restoration, and readiness.
The [protocol overview](README.md) owns wire rules and limits; definitions here
do not add protocol requirements.

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

## Canonical decimal string

An unsigned integer written as decimal digits in one unambiguous form:
`"0"` or a nonzero digit followed by digits, without a sign, spaces, or leading
zeros. Helper request IDs additionally require a positive value.

## Envelope

The outer structure that identifies a message and carries shared fields.
It is distinct from the message-specific payload and the client startup envelope.

## EOF

End of file: the end of a byte stream. Pipe EOF alone does not explain whether
a helper detached successfully, crashed, or completed cleanup.

## Frame

A length-delimited unit on a communication channel.
A logical message can use several frames, and a read can contain part of a frame.

## GUID

A globally unique identifier. A climon2 session ID is a GUID, not a process ID.

## High-water mark

The greatest request ID received in one originating direction.
Strictly increasing IDs let a receiver detect reuse without storing all past IDs.

## Idempotency key

An identifier that an explicitly defined retry mechanism uses to recognize
the same logical operation. Baseline helper request IDs do not supply that
mechanism or promise exactly-once execution.

## Indeterminate outcome

An outcome that cannot be established after communication fails.
It is neither confirmed success nor proof that nothing happened.

## Interception seam

A defined place where a plugin can observe or transform terminal traffic or
structured messages. It is not an unchecked bypass of protocol validation.

## JSON

JavaScript Object Notation, a text format for objects, arrays, strings, numbers,
booleans, and null. JSON wire examples in this reference show message bodies,
not their length prefixes or following binary frames.

## Namespaced name

A name scoped to avoid collisions with names owned by other extensions.
`org.example.attention` illustrates the agreed extension shape; the exact naming
grammar and registration policy are still unspecified.

## OSC

Operating System Command, a terminal escape-sequence family.
An OSC terminal sequence is not itself a structured plugin protocol message.
The first draft does not specify OSC 7501 or OSC 9 plugin decoding.

## Payload

The message-specific content inside an envelope or its declared binary frame.
Its encoding and schema belong to the operation or extension that owns it.

## Pseudo-terminal (PTY)

An operating-system terminal pair connecting a session daemon to its command.
It is distinct from the user's host terminal and a daemon socket.

## QUIC

A secure transport with multiple streams. It provides ordered bytes within a
stream, but not ordering between streams. It is not the agreed helper transport.
See [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html#section-2).

## Request ID

An identifier matching a request with its response in a conversation.
It is distinct from a control period, restoration ID, and idempotency key.

## Standard streams

Stdin, stdout, and stderr are a process's standard input, output, and diagnostic
streams. Helper mode reserves stdin/stdout for protocol frames and stderr for
diagnostics.

## Unicode and UTF-8

Unicode represents text, including non-Latin writing and emoji.
UTF-8 encodes Unicode text as bytes; one character may require several bytes.
Arbitrary terminal bytes need not be valid UTF-8.
