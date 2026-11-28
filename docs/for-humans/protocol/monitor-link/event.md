# Event — `0x81` to `0x8e`

**Status: agreed in [#480](https://github.com/jackgeek/climon2/issues/480). The `climon2-link` crate implements the encoding of every event, its order and its credit; no released program speaks it yet.**
The [relay](../../../../GLOSSARY.md#monitor-links) sends one client interface
[event](../TERMS.md#event) on a
[session stream](../../../../GLOSSARY.md#monitor-links), exactly as the
stream's client instance delivered it. See the [overview](../README.md),
[glossary](../../../../GLOSSARY.md), [terms](../TERMS.md) and the
[shared rules](README.md).

This page owns only the encoding. What each event means and what order events
arrive in belong to the [client interface](../monitor-client/README.md) page
linked in each row.

## Encoding rule

Every event record is built from its C structure in the client interface
[header](../monitor-client/client-v1.h) by the rule the
[request](request.md) page gives, with two differences:

1. The tag is `0x80` plus the record's `CM1_` number.
2. The body starts with the record's
   [connection generation](../TERMS.md#connection-generation) as a `u64`.

A union is encoded as its active member alone.

## Shared groups

| Group | Layout |
|---|---|
| Geometry | Columns `u32`, rows `u32`. |
| Optional text | Present `u32`, value `text`. The value is empty when absent. |
| Title | State `u32`, value `bytes`. |
| Status | State `u32`, episode present `u32`, episode ID `u64`, changed at 24 [bytes](../TERMS.md#byte). |
| Available status | Available `u32`, Status. |
| Progress | State `u32`, percent present `u32`, percent `u32`. |
| Available progress | Available `u32`, Progress. |
| Command result | Kind `u32`, value `u32`, signal name as Optional text. |
| Optional command | Present `u32`, Command result. |

## Layouts

| Tag | Event | Body after the generation |
|---|---|---|
| `0x81` | [Response](../monitor-client/response.md) | Request ID `u64`, outcome `u32`, result tag `u32`, `error`, then the result below. |
| `0x82` | [Initial metadata](../monitor-client/event-initial-metadata.md) | Title, Available status, Available progress, Optional command. |
| `0x83` | [Title](../monitor-client/event-title.md) | Title. |
| `0x84` | [Session status](../monitor-client/event-session-status.md) | Status. |
| `0x85` | [Notification](../monitor-client/event-notification.md) | Source `u32`, episode present `u32`, episode ID `u64`, then source ID, title and body, each as Optional text. |
| `0x86` | [Progress](../monitor-client/event-progress.md) | Progress. |
| `0x87` | [Control granted](../monitor-client/event-control.md) | Period `u64`, restoration ID `u64`, Geometry, reason `u32`. |
| `0x88` | [Control revoked](../monitor-client/event-control.md) | Period `u64`, restoration ID `u64`, reason `u32`. |
| `0x89` | [Restoration start](../monitor-client/event-restoration-start.md) | Period `u64`, restoration ID `u64`, Geometry. |
| `0x8a` | [Display](../monitor-client/event-display.md) | Period `u64`, restoration ID `u64`, display `bytes`. |
| `0x8b` | [Restoration complete](../monitor-client/event-restoration-complete.md) | Period `u64`, restoration ID `u64`. |
| `0x8c` | [Session ended](../monitor-client/event-session-ended.md) | Command result, Available status. |
| `0x8d` | [Geometry](../monitor-client/event-geometry.md) | Period `u64`, Geometry. |
| `0x8e` | [Diagnostic](../monitor-client/event-diagnostic.md) | `error`. |

### Response results

A Response ends with the result member that its result tag selects, as the
client interface's
[result mapping](../monitor-client/README.md#requests-and-result-mapping)
states. Each member is encoded as:

| Result member | Layout |
|---|---|
| None | Nothing. |
| Opened | Connection ID 16 bytes, selected `tokens`. |
| Accepted bytes | `u32`. |
| Authority result | Changed `u8`, period `u64`, restoration ID `u64`. |
| Flag result | Changed `u8`, value `u8`, accepted bytes `u32`. |
| Changed | `u8`. |
| Attachment detached | `u8`. |
| Termination | Optional command, cleanup `u32`. |
| Known final | Command result. |

## Example

Display, generation 3, control period 2, restoration 1, the two bytes `hi`:

```text
8a 00 00 00 00 00 00 00 03 00 00 00 00 00 00 00
02 00 00 00 00 00 00 00 01 00 00 00 02 68 69
```

## Rules

- Events are allowed after [Stream opened](stream-opened.md) and before
  [Stream ended](stream-ended.md).
- The relay sends events in the order the library delivered them. It never
  invents, repeats, reorders or drops one while the stream lives.
- Each event uses [credit](credit.md) equal to its record's length. The relay
  waits for credit; it does not split an event across what credit it has.
- An event may be far larger than a [unit](README.md#units). It then travels
  as several units, and stream-0 records may overtake it.
- The session's status and notifications arrive only as events on its stream.
  Nothing raised while the link was down is replayed. After a reconnect, the
  new client's Initial metadata carries the current status.
- The hub monitor treats every byte of every event as hostile, as it treats
  the output of a local session.
- The client's final run report is not an event. It travels in
  [Stream ended](stream-ended.md).

Owned by [#195](https://github.com/jackgeek/climon2/issues/195) for what is
carried and [#480](https://github.com/jackgeek/climon2/issues/480) for the
encoding.
