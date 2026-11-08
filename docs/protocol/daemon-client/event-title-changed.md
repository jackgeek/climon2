# Title changed — `0x42`

**Status: agreed, not implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67).
The [session daemon](../../../GLOSSARY.md#sessions) sends current title changes
to a subscribed [client](../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](core-v1.md).

## Body

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with sole required
`title`, string or explicit null (unset). Empty is supported, not unavailable.

```json
{"title":"Build — café"}
```

## Delivery and interpretation

Requires selected `title-v1`; no unselected-family event may be sent.
Follow [Initial metadata](event-initial-metadata.md) gap-free and in order,
including while passive, background, revoked, or not ready. Title is inert
[Unicode](../TERMS.md#unicode-and-utf-8) metadata, never terminal presentation
encoding or percent-encoded [OSC](../TERMS.md#osc). Invalid source text replacement occurs only
at the text boundary. Render literally; no display scraping or control
[grant](../../../GLOSSARY.md#attachment-and-control).
Null/empty title selects command/arguments menu fallback; unavailable remains
a distinct initial availability state. Changes do not acknowledge
[attention](../../../GLOSSARY.md#status-and-attention).
The [#63 chrome contract](../terminal-integration.md#chrome-cursor-and-host-copy)
uses title/fallback only as a label, never a host-title change. Only overflowing
label text marquees with [display-cell](../TERMS.md#display-cell)-safe clipping; status/control/error stay
stationary and full details remain in the selector.

No response/[request ID](../TERMS.md#request-id) or defaults/unknown members.
Malformed/unsupported events follow the fatal
[validation rules](core-v1.md#framing-and-validation).
Required healthy-stream updates cannot silently disappear under congestion;
bounds/exhaustion policy are in
[operational limits](../../monitor-operational-limits.md#payloads-and-parser-state).
No archive or naming
[API](../TERMS.md#api) is added.
Related: [public title mapping](../monitor-client/event-title.md).
