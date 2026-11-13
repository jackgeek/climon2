# Display — `0x49`

**Status: agreed and implemented** for interactive attachments
([#87](https://github.com/jackgeek/climon2/issues/87)) and, with canonical
presentation, for monitor-mode connections
([#89](https://github.com/jackgeek/climon2/issues/89)), owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) delivers ordered terminal
[bytes](../TERMS.md#byte) to the controlling
[client](../../../GLOSSARY.md#sessions), including background control.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Binary body and example

The 16-byte header is two nonzero unsigned 64-bit
[big-endian](../TERMS.md#big-endian) integers:
[period](../../../GLOSSARY.md#attachment-and-control),
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Nonempty binary bytes follow unchanged, including [NUL](../TERMS.md#nul)/
invalid [UTF-8](../TERMS.md#unicode-and-utf-8).
No [request ID](../TERMS.md#request-id) or [JSON](../TERMS.md#json) body is present.

```text
00 00 00 14 49
00 00 00 00 00 00 00 02
00 00 00 00 00 00 00 03
00 ff 41
```

Length 20 is one type + 16 header + 3 payload bytes. Period 2, restoration 3
carries exactly `00 ff 41`. Chunk to the advertised server [frame](../TERMS.md#frame) bound;
chunk boundaries do not change byte order.

## Order and guarantees

Core support is required. Matching chunks between
[Restoration begin](event-restoration-begin.md) and
[complete](event-restoration-complete.md) form
[restoration](../../../GLOSSARY.md#restoration-and-presentation); live chunks follow
with the same authority IDs. Empty restoration has no Display frames.
Metadata may interleave; terminal bytes remain ordered.
No old-authority Display follows a newer begin or
[revocation](../../../GLOSSARY.md#attachment-and-control) invalidating it.
Revoked clients receive metadata, not core live mirroring.
For monitor-mode connections all display/restoration bytes use effective session
geometry, including background grants, never outer-host/requested dimensions.
The [canonical #63 policy](../terminal-integration.md#canonical-monitor-presentation-amendment-to-daemon-wire-revision-1)
needs no extra geometry field and leaves ordinary [direct](../../../GLOSSARY.md#restoration-and-presentation)/[derived](../../../GLOSSARY.md#restoration-and-presentation) attach unchanged.
The monitor parses exact bytes into its own model and paints read-only Rust views
with Ratatui; no session bytes go directly to the host and no duplicate query
replies are emitted. Native cell serialization is not introduced.

Delivery is not applied [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
application consumption or command completion.
Empty Display is an invalid operation; malformed framing/header/authority is
invalid under [shared validation](core-v1.md#framing-and-validation): a body
shorter than the 16-byte header, or a zero period or restoration ID, is
`invalid_record`.
No optional fields/defaults/text replacement.
Required bytes cannot be silently lost on healthy streams; explicitly fail
the affected connection under #69 policy instead.
#63 owns rendering/application/fidelity, #69 numeric transfer/queue limits.
Related: [Input](request-input.md).

## Implemented behavior

The [session daemon](../../../GLOSSARY.md#sessions) sends Display to an
[attachment](../../../GLOSSARY.md#attachment-and-control) while
it controls: its restoration chunks, live output, and the
[release](../../../GLOSSARY.md#restoration-and-presentation) of its
[presentation](../../../GLOSSARY.md#restoration-and-presentation) when its control or
attachment ends, before
[Control revoked](event-control-revoked.md) or the [Leave](request-leave.md)
success. Each frame carries the period and restoration ID of the current
restoration and at most 65,535 payload bytes, the chunk size of the private
protocol's output. The daemon never sends an empty Display. Bytes are exact:
the daemon's live-output policy decides what reaches the attachment, as for
an ordinary attach, and terminal queries are answered by the daemon alone.
An interactive attachment's bytes are presented for its host, Direct or
Derived. A monitor-mode connection's, once its Acquire or a
[promotion](../../../GLOSSARY.md#attachment-and-control) made it an attachment, have
[canonical presentation](../../../GLOSSARY.md#restoration-and-presentation)
whatever host size it reports: its restorations are canonical, as
[Restoration begin](event-restoration-begin.md#implemented-behavior)
describes, and its live output is the filtered output of Direct presentation
at the session geometry. No Display reaches it at a new effective geometry
before the Restoration begin at that geometry.
