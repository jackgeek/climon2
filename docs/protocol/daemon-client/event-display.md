# Display — `0x49`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) delivers ordered terminal
[bytes](../TERMS.md#byte) to the controlling
[client](../../../GLOSSARY.md#sessions), including background control.
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Binary body and example

The 16-byte header is two nonzero unsigned 64-bit
[big-endian](../TERMS.md#big-endian) integers: period,
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

Delivery is not applied [readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts),
application consumption or command completion.
Empty Display is an invalid operation; malformed framing/header/authority is
invalid under [shared validation](core-v1.md#framing-and-validation).
No optional fields/defaults/text replacement.
Required bytes cannot be silently lost on healthy streams; explicitly fail
the affected connection under #69 policy instead.
#63 owns rendering/application/fidelity, #69 numeric transfer/queue limits.
Related: [Input](request-input.md).
