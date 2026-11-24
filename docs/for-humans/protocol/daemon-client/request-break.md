# Break — `0x3a`

**Status: agreed and implemented
([#326](https://github.com/jackgeek/climon2/issues/326)) as an additive amendment
to wire revision 1, like [Focus](request-focus.md).** The
[client library](../../../../GLOSSARY.md#sessions) sends it, and the
[client executable](../../../../GLOSSARY.md#sessions)'s ordinary attach on
Windows uses it.
The [client](../../../../GLOSSARY.md#sessions) asks the
[session daemon](../../../../GLOSSARY.md#sessions), as the current ready
[controller](../../../../GLOSSARY.md#attachment-and-control), to give the
command's console Ctrl+Break. Ctrl+Break has no [byte](../TERMS.md#byte), so it
cannot travel as [Input](request-input.md).
See the [overview](../README.md), [glossary](../../../../GLOSSARY.md),
[terms](../TERMS.md), and inherited [wire rules](README.md).

## Body, support, and example

One [JSON](../TERMS.md#json) [frame](../TERMS.md#frame) with required
`request_id`, `period` and `restoration_id`, all
[Counters](README.md#shared-types). There are no optional fields, defaults or
unknown members.

```json
{"request_id":"9","period":"2","restoration_id":"3"}
```

`0x3a` is the next unused [request](../TERMS.md#request) code after `0x39`
Focus; it changes no existing code. Break requires the selected optional core
[capability](../../../../GLOSSARY.md#sessions) `break-v1`, advertised in the
existing [hello](../../../../GLOSSARY.md#sessions) capability array and
selected by [Open](request-open.md). Only a session daemon on Windows
advertises it: Unix terminals have no Ctrl+Break to pass on. There is no new
hello field, release-number inference or extension negotiation, and a session
whose daemon does not advertise `break-v1` stays usable without it.

## Acceptance and response

Break needs `break-v1`, checked first, then the ready controller's current
[authority](README.md#implemented-authority-checks). Success
[Response](response.md) result is `{"kind":"break_sent"}`:

```json
{"request_id":"9","outcome":"ok","result":{"kind":"break_sent"},"error":null}
```

Success proves that the daemon started delivering Ctrl+Break, not that a
program received it, handled it or ended. Break is never Input: it changes no
[status](../../../../GLOSSARY.md#status-and-attention) and does not count as
user activity.

A Break whose Open did not select `break-v1` is rejected:

```json
{"request_id":"9","outcome":"rejected","result":null,"error":{"code":"unsupported_capability","message":"break-v1 was not selected"}}
```

## Rejection and failure

Break uses existing operational codes: `unsupported_capability`,
`not_attached`, `not_controller`, `stale_period`, `stale_restoration`,
`not_ready`, `invalid_state` and `session_terminating` as for the other
authority-checked requests, and `io_error` when the daemon cannot start
delivering it. Each rejection has no effects, and the stream goes on.
Wrong types or members, malformed framing or IDs and the wrong direction are
[fatal](README.md#framing-and-validation). A rejected Break is never replayed.
Related: [Input](request-input.md), [Focus](request-focus.md),
[Acknowledge](request-acknowledge.md), and the
[native Break](../monitor-client/request-break.md).

## Implemented behavior

The session daemon on Windows advertises `break-v1` and selects it when Open
asks for it. It answers a Break that passes its checks by starting a
short-lived helper that attaches to the console of the command's first process
and generates `CTRL_BREAK_EVENT` there, as the
[Windows lifecycle contract](../../windows/lifecycle.md#ctrlc-and-ctrlbreak)
describes, then answers `break_sent` without waiting for the helper. A helper
that cannot start is `io_error`. Every program on the command's console
receives the event; one with the default handler ends with `0xC000013A`.

A session daemon on Linux or macOS does not advertise `break-v1`, so it never
selects it, and answers every Break with `unsupported_capability`.
