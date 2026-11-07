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

## API

An application programming interface: the operations and data contract exposed
to a caller. A public native API also requires an agreed binary interface.

## Backpressure

Slowing a sender when the receiver cannot keep up, so pending data stays bounded.
It is not permission to silently drop required messages.

## Baseline

The required common protocol behavior that compatible participants agree to
support. Optional [extensions](../../GLOSSARY.md#protocol-and-plugin-concepts)
add behavior without redefining that common contract.

## BEL

The terminal bell control [byte](#byte), hexadecimal `07`. A future
[attention](../../GLOSSARY.md#status-and-attention) [plugin](../../GLOSSARY.md#protocol-and-plugin-concepts)
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

## C0

The control-byte range `00` through `1f`. Several key chords can have the same
encoding in a legacy terminal; their spelling does not make them distinguishable.

## Canonical decimal string

An [unsigned integer](#unsigned-integer) written as decimal digits in one unambiguous form:
`"0"` or a nonzero digit followed by digits, without a sign, spaces, or leading
zeros. Each interface specifies its permitted range separately.

## Communication handle

An [opaque](#opaque-handle) per-instance library handle through which the
[monitor](../../GLOSSARY.md#sessions) submits work
and drains deliveries. It is distinct from the independent shutdown control
handle; thread safety does not allow concurrent instance destruction.

## Connection generation

The identity distinguishing successive [daemon](../../GLOSSARY.md#sessions)
connections within one [client](../../GLOSSARY.md#sessions)
instance. It is separate from [request IDs](#request-id),
[control periods](../../GLOSSARY.md#attachment-and-control),
[restoration IDs](../../GLOSSARY.md#protocol-and-plugin-concepts),
and attention-episode identities.

## Discriminator

A field or code identifying which message variant is being sent.

## Display cell

One column position in a terminal grid. Wide characters span cells; combining
characters can share a cell. Byte or character offsets are not cell offsets.

## Counter

An increasing identifier or sequence value. Its range, representation and
scope belong to the owning interface; it is not a timestamp or a retry key.

## Geometry

A terminal's size in columns and rows, distinct from pixel dimensions.
Representable values do not determine operational sizing or clamping policy.

## Availability

Whether an interface can supply a value, distinct from a supported value being
unset or empty. The owning interface specifies its explicit representation.

## Envelope

The outer structure that identifies a message and carries shared fields.
It is distinct from the message-specific [payload](#payload) and the client
[startup envelope](../../GLOSSARY.md#sessions).

## Event

A message that reports information without asking for a [response](#response).

## EOF

End of file: the end of a byte stream. EOF alone does not explain whether
a client [detached](../../GLOSSARY.md#attachment-and-control) successfully or completed cleanup.

## FIFO

First in, first out: records leave a queue in the order they entered it.

## Frame

A length-delimited unit on a communication channel.
A logical message can use several frames, and a read can contain part of a frame.

## Function table

An agreed layout of function pointers through which a caller uses one revision
of a native-library interface.

## FSM

A finite state machine: a set of states and the event-driven transitions
between them.

## GUID

A globally unique identifier. A climon2 [session ID](../../GLOSSARY.md#protocol-and-plugin-concepts)
is a GUID, not a process ID.

## High-water mark

The greatest locally accepted request ID in one instance's lifetime for the
monitor-client contract. Rejected submissions do not advance it. Strictly
increasing IDs detect reuse without storing all past IDs.

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

## I/O

Input/output: reading or writing data to another resource, such as a socket.

## JSON

JavaScript Object Notation, a text format for objects, arrays, strings, numbers,
booleans, and null. JSON wire examples in this reference show message bodies,
not their length prefixes.

## MiB

A mebibyte: 1,048,576 bytes. This unit is distinct from a decimal megabyte.

## Monotonic clock

A clock for measuring elapsed time that does not go backwards when wall-clock
time is corrected. Inactivity deadlines use this clock, not [UTC](#utc) timestamps.

## Namespaced name

A name scoped to avoid collisions with names owned by other extensions.
`org.example.attention` illustrates an extension name; the exact naming
grammar and registration policy are still unspecified.

## Native path

A filesystem path represented without losing platform-native data, including
non-[UTF-8](#unicode-and-utf-8) bytes on Unix or native code units on Windows.

## NUL

The zero-valued byte. It terminates traditional C strings but is ordinary data
in a length-delimited binary buffer.

## Opaque handle

An identifier for library-owned state whose internal representation a caller
must not inspect. Its interface specifies validity, permitted uses, and release.

## OSC

Operating System Command, a terminal escape-sequence family.
An OSC terminal sequence is not itself a structured plugin protocol message.
OSC 7501 is the Program Status Protocol; OSC 133 marks shell prompts and
commands. The planned core interpretation and notification/progress relay are
specified by the [session-status revision](../session-status.md), not a plugin
decoder contract.

## Payload

The content carried by a message or operation, distinct from its routing and
correlation metadata.

## Pollable notification descriptor

A library-owned operating-system notification resource that the monitor can
include in its event wait. Readiness signals possible drain progress or stream
closure, not the number of records. The interface drain operation, not direct
descriptor [I/O](#io), consumes deliveries and manages notification state.

## Pseudo-terminal (PTY)

An operating-system terminal pair connecting a session daemon to its command.
It is distinct from the user's [host](../../GLOSSARY.md#sessions) terminal and a daemon socket.

## QUIC

A secure transport with multiple streams. It provides ordered bytes within a
stream, but not ordering between streams. It is outside the current monitor plan.
See [RFC 9000](https://www.rfc-editor.org/rfc/rfc9000.html#section-2).

## Request

A message asking another participant to perform an operation.

## RDP

Remote Desktop Protocol. “RDP-like” describes interactive remote presentation
and input, not use of that protocol or a compatibility claim.

## Request ID

An identifier matching a [request](#request) with its response in a conversation.
It is distinct from a control period, restoration ID, and [idempotency key](#idempotency-key).

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

A brief on-screen notification. The planned monitor's
[attention mechanics](../session-status.md#titles-and-notifications) specify
when notifications occur; exact visual presentation remains unresolved.

## TCP/IP

Transmission Control Protocol / Internet Protocol: networking protocols that
can carry an ordered byte stream between hosts. Selecting this carrier alone
does not specify routing, authentication, or application multiplexing.

## TUI

A terminal user interface, drawn as terminal text and cells rather than native
graphical widgets. A [TUI fixture](../../GLOSSARY.md#testing) is a test program,
not the monitor itself.

## UID

A user identifier assigned by the operating system. Same-UID peer checks are
part of the current local socket deployment, not a remote authentication scheme.

## WebSocket

A browser-compatible bidirectional transport over a network connection.
It can carry forwarded terminal traffic without loading a native client
library in the browser.

## UI

User interface: a program's user-facing presentation and controls.

## Unicode and UTF-8

Unicode represents text, including non-Latin writing and emoji.
UTF-8 encodes Unicode text as bytes; one character may require several bytes.
Arbitrary terminal bytes need not be valid UTF-8.

## Unsigned integer

A whole number that cannot be negative. Its bit width determines the largest
representable value.

## UTC

Coordinated Universal Time, used for externally reported status-change
timestamps. A UTC timestamp is not an implicit unique identifier or a
monotonic elapsed-time measurement.
