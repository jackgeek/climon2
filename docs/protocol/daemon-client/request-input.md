# Input — `0x31`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [client](../../../GLOSSARY.md#sessions) sends actual user application
[bytes](../TERMS.md#byte) to the
[session daemon](../../../GLOSSARY.md#sessions), only as current ready
[controller](../../../GLOSSARY.md#attachment-and-control).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Binary body and example

The 24-byte header is three nonzero [big-endian](../TERMS.md#big-endian)
unsigned 64-bit integers: [request ID](../TERMS.md#request-id),
[control period](../../../GLOSSARY.md#attachment-and-control),
[restoration ID](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Nonempty application bytes follow unchanged, including [NUL](../TERMS.md#nul)
and invalid [UTF-8](../TERMS.md#unicode-and-utf-8).
This is not [JSON](../TERMS.md#json). Length includes type, header and payload; chunk within the
advertised request limit, with a separate request ID per chunk.

```text
00 00 00 1c 31
00 00 00 00 00 00 00 02
00 00 00 00 00 00 00 01
00 00 00 00 00 00 00 01
00 ff 41
```

Length 28 is one type + 24 header + 3 payload bytes. Request 2, period 1,
restoration 1 forwards the exact payload `00 ff 41`, not text.

## Acceptance and failures

Validate current [control period](../../../GLOSSARY.md#attachment-and-control),
[restoration](../../../GLOSSARY.md#restoration-and-presentation) and
[readiness](../../../GLOSSARY.md#protocol-and-plugin-concepts).
Accept the full payload for forwarding or reject; never partially accept.
Success [Response](response.md) result is
`{"kind":"input_accepted","accepted_bytes":<positive u32>}`, equal to the full
payload size. It does not prove application consumption.
Accepted real user input clears pending/acknowledged
[attention](../../../GLOSSARY.md#status-and-attention) under the
[status mechanics](../../session-status.md#attention); terminal replies and
unmarked injected work are not core Input.

Stale authority, no [attachment](../../../GLOSSARY.md#attachment-and-control)/
control/readiness, empty input, terminating
session and resource/I/O failure report explicit
[operational errors](core-v1.md#errors-and-bounded-delivery).
Invalid framing/header/IDs is fatal. A body shorter than the 24-byte header
is `invalid_record`, whatever bytes it holds. In a complete header, a zero
request ID is `invalid_request_id`, and a zero period or restoration ID with
a nonzero request ID is `invalid_record`.
Well-formed rejected requests consume
wire IDs; loss never permits automatic replay. There are no optional fields,
text conversion or defaults. Core [capability](../../../GLOSSARY.md#sessions)
is required; bounds/policy #69
and [monitor](../../../GLOSSARY.md#sessions) routing/application #63 remain separate.
Related: [Ready](request-ready.md), [Display](event-display.md).

The [#63 input contract](../terminal-integration.md#transparent-keyboard-and-exact-paste)
requires supported cursor/keypad encoding, exact paste payload with markers only
when application paste mode is on, and translated application-rectangle mouse
events. Monitor input additionally waits for coherent selected paint, excluding
modal UI/stale/too-small targets, not nonmodal toast coverage. Host replies stay
monitor-local and application focus uses non-user [Focus](request-focus.md);
neither enters real-user Input or clears attention.
