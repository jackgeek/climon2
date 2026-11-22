# Title changed — `0x42`

**Status: agreed and implemented**, owned by [#67](https://github.com/jackgeek/climon2/issues/67),
with the decoding limits below.
The [session daemon](../../../../GLOSSARY.md#sessions) sends current title changes
to a subscribed [client](../../../../GLOSSARY.md#sessions).
See the [overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](README.md).

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
[grant](../../../../GLOSSARY.md#attachment-and-control).
Null/empty title selects command/arguments menu fallback; unavailable remains
a distinct initial availability state. Changes do not acknowledge
[attention](../../../../GLOSSARY.md#status-and-attention).
The [#63 chrome contract](../terminal-integration.md#chrome-cursor-and-host-copy)
uses title/fallback only as a label, never a host-title change. Overflowing
label text is truncated statically, ending with `…` at a
[display-cell](../TERMS.md#display-cell)-safe width (nothing marquees); status/control/error stay
stationary and full details remain in the selector.

No response/[request ID](../TERMS.md#request-id) or defaults/unknown members.
Malformed/unsupported events follow the fatal
[validation rules](README.md#framing-and-validation).
Required healthy-stream updates cannot silently disappear under congestion;
bounds/exhaustion policy are in
[operational limits](../../behaviour/monitor/operational-limits.md#payloads-and-parser-state).
No archive or naming
[API](../TERMS.md#api) is added.
Related: [public title mapping](../monitor-client/event-title.md).

## Implemented behavior

The [session daemon](../../../../GLOSSARY.md#sessions) sends one Title changed
for each change of the application's own title, in order, even when one read
of command output holds several, to every connection that selected
`title-v1`, whatever its control state. Changes are by value: setting the
same title again sends nothing. `ESC c` (full reset) returns the title to
unset. Titles come from the session's terminal decoder, Alacritty 0.26.0
behind `climon2-vt`, and are never logged.

The decoder bounds and normalizes titles. A title longer than the 4,096
[UTF-8](../TERMS.md#unicode-and-utf-8) [bytes](../TERMS.md#byte) of the
[operational limits](../../behaviour/monitor/operational-limits.md#payloads-and-parser-state)
is ignored, never truncated; the OSC 0/2 sequence's `0;` or `2;` does not
count against it (owner decision on
[#90](https://github.com/jackgeek/climon2/issues/90)). The decoder also
trims surrounding whitespace, drops each semicolon-separated part of a title that is not valid UTF-8 instead of
replacing the invalid bytes (a title of one such part becomes empty), and
drops everything from a title's 15th semicolon on. This decoding is accepted
for revision 1, as decided on
[#86](https://github.com/jackgeek/climon2/issues/86).
